//! Rust-side primitives for pagelet native adapters.
//!
//! Hosts receive typed opaque tokens rather than pointers or Rust collection
//! indexes. The control-plane C API is layered on top of this registry.
#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

use crate::{
    core::{CancellationToken, InternalErrorId, PageletError, ProtocolError},
    document::ChapterIr,
    engine::{BookSession, Engine, LayoutSession},
};

mod control;

pub use control::{ControlLayoutRequest, ControlPlane};

const HANDLE_KIND_BITS: u32 = 3;
const HANDLE_KIND_MASK: u64 = (1 << HANDLE_KIND_BITS) - 1;
const MAX_HANDLE_GENERATION: u64 = u64::MAX >> HANDLE_KIND_BITS;

static NEXT_HANDLE_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_INTERNAL_ERROR_ID: AtomicU64 = AtomicU64::new(1);

/// Runtime kind encoded in an opaque handle token.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum HandleKind {
    Engine = 1,
    Book = 2,
    Chapter = 3,
    LayoutSession = 4,
    Request = 5,
}

impl HandleKind {
    const fn tag(self) -> u64 {
        self as u64
    }
}

impl fmt::Display for HandleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine => f.write_str("engine"),
            Self::Book => f.write_str("book"),
            Self::Chapter => f.write_str("chapter"),
            Self::LayoutSession => f.write_str("layout session"),
            Self::Request => f.write_str("request"),
        }
    }
}

macro_rules! define_handle {
    ($name:ident, $kind:ident, $docs:literal) => {
        #[doc = $docs]
        ///
        /// The numeric representation is an opaque transport token. It is not
        /// a pointer or an index and must not be interpreted by hosts.
        #[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
        #[repr(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Validate and wrap a raw transport token.
            #[must_use]
            pub const fn from_raw(raw: u64) -> Option<Self> {
                if raw >> HANDLE_KIND_BITS != 0 && raw & HANDLE_KIND_MASK == HandleKind::$kind.tag()
                {
                    Some(Self(raw))
                } else {
                    None
                }
            }

            /// Return the opaque transport token used by native adapters.
            #[must_use]
            pub const fn as_raw(self) -> u64 {
                self.0
            }
        }

        impl From<$name> for u64 {
            fn from(handle: $name) -> Self {
                handle.as_raw()
            }
        }
    };
}

define_handle!(EngineHandle, Engine, "Opaque handle for an engine.");
define_handle!(BookHandle, Book, "Opaque handle for an opened book.");
define_handle!(
    ChapterHandle,
    Chapter,
    "Opaque handle for a prepared chapter."
);
define_handle!(
    LayoutSessionHandle,
    LayoutSession,
    "Opaque handle for a host-measured layout session."
);
define_handle!(
    RequestHandle,
    Request,
    "Opaque handle for cancellable in-flight work."
);

trait TypedHandle: Copy {
    const KIND: HandleKind;

    fn raw(self) -> u64;
    fn from_allocated_raw(raw: u64) -> Self;
}

macro_rules! impl_typed_handle {
    ($name:ident, $kind:ident) => {
        impl TypedHandle for $name {
            const KIND: HandleKind = HandleKind::$kind;

            fn raw(self) -> u64 {
                self.0
            }

            fn from_allocated_raw(raw: u64) -> Self {
                debug_assert_eq!(raw & HANDLE_KIND_MASK, Self::KIND.tag());
                Self(raw)
            }
        }
    };
}

impl_typed_handle!(EngineHandle, Engine);
impl_typed_handle!(BookHandle, Book);
impl_typed_handle!(ChapterHandle, Chapter);
impl_typed_handle!(LayoutSessionHandle, LayoutSession);
impl_typed_handle!(RequestHandle, Request);

/// Failure returned while validating or allocating opaque handles.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HandleError {
    /// The token is malformed, belongs to the wrong kind, or was disposed.
    InvalidOrDisposed { expected: HandleKind },
    /// The process-wide generation space has been exhausted.
    GenerationExhausted,
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrDisposed { expected } => {
                write!(f, "invalid or disposed {expected} handle")
            }
            Self::GenerationExhausted => f.write_str("opaque handle generation space exhausted"),
        }
    }
}

impl std::error::Error for HandleError {}

impl From<HandleError> for PageletError {
    fn from(error: HandleError) -> Self {
        Self::Protocol(ProtocolError::new(error.to_string()))
    }
}

/// Result of an idempotent handle disposal.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DisposeOutcome {
    /// The handle and the stated number of descendants were disposed.
    Disposed { descendants: usize },
    /// The token was invalid or had already been disposed.
    AlreadyDisposed,
}

/// Point-in-time count of live registry entries.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct HandleLeakReport {
    pub engines: usize,
    pub books: usize,
    pub chapters: usize,
    pub layout_sessions: usize,
    pub requests: usize,
}

impl HandleLeakReport {
    /// Return the total number of live opaque handles.
    #[must_use]
    pub const fn total(self) -> usize {
        self.engines + self.books + self.chapters + self.layout_sessions + self.requests
    }

    /// Return true when no handles remain live.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.total() == 0
    }
}

enum Entry {
    Engine(Engine),
    Book {
        engine: EngineHandle,
        session: BookSession,
    },
    Chapter {
        book: BookHandle,
        chapter: Arc<ChapterIr>,
    },
    LayoutSession {
        book: BookHandle,
        session: Arc<Mutex<LayoutSession>>,
    },
    Request {
        layout: LayoutSessionHandle,
        cancellation: CancellationToken,
    },
}

impl Entry {
    const fn kind(&self) -> HandleKind {
        match self {
            Self::Engine(_) => HandleKind::Engine,
            Self::Book { .. } => HandleKind::Book,
            Self::Chapter { .. } => HandleKind::Chapter,
            Self::LayoutSession { .. } => HandleKind::LayoutSession,
            Self::Request { .. } => HandleKind::Request,
        }
    }

    const fn parent_raw(&self) -> Option<u64> {
        match self {
            Self::Engine(_) => None,
            Self::Book { engine, .. } => Some(engine.0),
            Self::Chapter { book, .. } | Self::LayoutSession { book, .. } => Some(book.0),
            Self::Request { layout, .. } => Some(layout.0),
        }
    }

    fn cancel_if_request(&self) {
        if let Self::Request { cancellation, .. } = self {
            cancellation.cancel();
        }
    }
}

/// Process-local owner of all native adapter objects.
///
/// Handles use a monotonically allocated generation plus a type tag. Tokens
/// are never reused during the process lifetime, so a stale handle cannot
/// alias a newly-created object. Dropping a non-empty registry trips a debug
/// assertion after releasing all objects and cancelling live requests.
pub struct HandleRegistry {
    entries: Mutex<BTreeMap<u64, Entry>>,
}

impl Default for HandleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for HandleRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleRegistry")
            .field("live_handles", &self.leak_report())
            .finish()
    }
}

impl HandleRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Register an engine root.
    pub fn insert_engine(&self, engine: Engine) -> Result<EngineHandle, HandleError> {
        self.insert_entry(Entry::Engine(engine))
    }

    /// Register a book owned by a live engine.
    pub fn insert_book(
        &self,
        engine: EngineHandle,
        session: BookSession,
    ) -> Result<BookHandle, HandleError> {
        let mut entries = self.lock_entries();
        require_kind(&entries, engine.raw(), HandleKind::Engine)?;
        insert_locked(&mut entries, Entry::Book { engine, session })
    }

    /// Register a chapter owned by a live book.
    pub fn insert_chapter(
        &self,
        book: BookHandle,
        chapter: Arc<ChapterIr>,
    ) -> Result<ChapterHandle, HandleError> {
        let mut entries = self.lock_entries();
        require_kind(&entries, book.raw(), HandleKind::Book)?;
        insert_locked(&mut entries, Entry::Chapter { book, chapter })
    }

    /// Register a layout session owned by a live book.
    pub fn insert_layout_session(
        &self,
        book: BookHandle,
        session: LayoutSession,
    ) -> Result<LayoutSessionHandle, HandleError> {
        let mut entries = self.lock_entries();
        require_kind(&entries, book.raw(), HandleKind::Book)?;
        insert_locked(
            &mut entries,
            Entry::LayoutSession {
                book,
                session: Arc::new(Mutex::new(session)),
            },
        )
    }

    /// Register cancellable work owned by a live layout session.
    pub fn insert_request(
        &self,
        layout: LayoutSessionHandle,
        cancellation: CancellationToken,
    ) -> Result<RequestHandle, HandleError> {
        let mut entries = self.lock_entries();
        require_kind(&entries, layout.raw(), HandleKind::LayoutSession)?;
        insert_locked(
            &mut entries,
            Entry::Request {
                layout,
                cancellation,
            },
        )
    }

    /// Resolve an engine handle.
    pub fn engine(&self, handle: EngineHandle) -> Result<Engine, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::Engine(engine)) => Ok(*engine),
            _ => Err(invalid(HandleKind::Engine)),
        }
    }

    /// Resolve a book handle.
    pub fn book(&self, handle: BookHandle) -> Result<BookSession, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::Book { session, .. }) => Ok(session.clone()),
            _ => Err(invalid(HandleKind::Book)),
        }
    }

    /// Resolve a chapter handle.
    pub fn chapter(&self, handle: ChapterHandle) -> Result<Arc<ChapterIr>, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::Chapter { chapter, .. }) => Ok(Arc::clone(chapter)),
            _ => Err(invalid(HandleKind::Chapter)),
        }
    }

    pub(crate) fn chapter_book(&self, handle: ChapterHandle) -> Result<BookHandle, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::Chapter { book, .. }) => Ok(*book),
            _ => Err(invalid(HandleKind::Chapter)),
        }
    }

    /// Resolve a layout session into separately synchronized shared state.
    pub fn layout_session(
        &self,
        handle: LayoutSessionHandle,
    ) -> Result<Arc<Mutex<LayoutSession>>, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::LayoutSession { session, .. }) => Ok(Arc::clone(session)),
            _ => Err(invalid(HandleKind::LayoutSession)),
        }
    }

    /// Resolve the cancellation token for in-flight work.
    pub fn request_cancellation(
        &self,
        handle: RequestHandle,
    ) -> Result<CancellationToken, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::Request { cancellation, .. }) => Ok(cancellation.clone()),
            _ => Err(invalid(HandleKind::Request)),
        }
    }

    pub(crate) fn request_layout(
        &self,
        handle: RequestHandle,
    ) -> Result<LayoutSessionHandle, HandleError> {
        match self.lock_entries().get(&handle.raw()) {
            Some(Entry::Request { layout, .. }) => Ok(*layout),
            _ => Err(invalid(HandleKind::Request)),
        }
    }

    pub(crate) fn has_request_for_layout(
        &self,
        handle: LayoutSessionHandle,
    ) -> Result<bool, HandleError> {
        let entries = self.lock_entries();
        require_kind(&entries, handle.raw(), HandleKind::LayoutSession)?;
        Ok(entries
            .values()
            .any(|entry| matches!(entry, Entry::Request { layout, .. } if *layout == handle)))
    }

    /// Dispose whichever live object owns the opaque token.
    ///
    /// This is the primitive used by a generic `handle_dispose` C entry point.
    /// Invalid and repeated calls are no-ops.
    pub fn dispose(&self, raw: u64) -> DisposeOutcome {
        self.dispose_matching(raw, None)
    }

    /// Dispose an engine and every object rooted beneath it.
    pub fn dispose_engine(&self, handle: EngineHandle) -> DisposeOutcome {
        self.dispose_matching(handle.raw(), Some(HandleKind::Engine))
    }

    /// Dispose a book and its chapters, layout sessions, and requests.
    pub fn dispose_book(&self, handle: BookHandle) -> DisposeOutcome {
        self.dispose_matching(handle.raw(), Some(HandleKind::Book))
    }

    /// Dispose a chapter.
    pub fn dispose_chapter(&self, handle: ChapterHandle) -> DisposeOutcome {
        self.dispose_matching(handle.raw(), Some(HandleKind::Chapter))
    }

    /// Dispose a layout session and cancel its in-flight requests.
    pub fn dispose_layout_session(&self, handle: LayoutSessionHandle) -> DisposeOutcome {
        self.dispose_matching(handle.raw(), Some(HandleKind::LayoutSession))
    }

    /// Dispose and cancel an in-flight request.
    pub fn dispose_request(&self, handle: RequestHandle) -> DisposeOutcome {
        self.dispose_matching(handle.raw(), Some(HandleKind::Request))
    }

    /// Inspect live entries. Debug builds also evaluate this report on drop.
    #[must_use]
    pub fn leak_report(&self) -> HandleLeakReport {
        report_entries(&self.lock_entries())
    }

    fn insert_entry<H: TypedHandle>(&self, entry: Entry) -> Result<H, HandleError> {
        debug_assert_eq!(entry.kind(), H::KIND);
        insert_locked(&mut self.lock_entries(), entry)
    }

    fn dispose_matching(&self, raw: u64, expected: Option<HandleKind>) -> DisposeOutcome {
        let mut entries = self.lock_entries();
        let Some(entry) = entries.get(&raw) else {
            return DisposeOutcome::AlreadyDisposed;
        };
        if expected.is_some_and(|kind| kind != entry.kind()) {
            return DisposeOutcome::AlreadyDisposed;
        }

        let mut descendants = Vec::new();
        let mut frontier = vec![raw];
        while let Some(parent) = frontier.pop() {
            for child in entries
                .iter()
                .filter_map(|(child_raw, child)| {
                    (child.parent_raw() == Some(parent)).then_some(*child_raw)
                })
                .collect::<Vec<_>>()
            {
                descendants.push(child);
                frontier.push(child);
            }
        }

        for child in descendants.iter().rev() {
            if let Some(entry) = entries.remove(child) {
                entry.cancel_if_request();
            }
        }
        if let Some(entry) = entries.remove(&raw) {
            entry.cancel_if_request();
        }

        DisposeOutcome::Disposed {
            descendants: descendants.len(),
        }
    }

    fn lock_entries(&self) -> MutexGuard<'_, BTreeMap<u64, Entry>> {
        match self.entries.lock() {
            Ok(entries) => entries,
            Err(poisoned) => {
                self.entries.clear_poison();
                poisoned.into_inner()
            }
        }
    }
}

impl Drop for HandleRegistry {
    fn drop(&mut self) {
        let entries = match self.entries.get_mut() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        #[cfg(debug_assertions)]
        let report = report_entries(entries);
        for entry in entries.values() {
            entry.cancel_if_request();
        }
        entries.clear();

        #[cfg(debug_assertions)]
        if !report.is_empty() && !std::thread::panicking() {
            panic!("leaked opaque handles at registry drop: {report:?}");
        }
    }
}

fn insert_locked<H: TypedHandle>(
    entries: &mut BTreeMap<u64, Entry>,
    entry: Entry,
) -> Result<H, HandleError> {
    debug_assert_eq!(entry.kind(), H::KIND);
    let raw = allocate_raw(H::KIND)?;
    let previous = entries.insert(raw, entry);
    debug_assert!(previous.is_none(), "opaque handle generation reused");
    Ok(H::from_allocated_raw(raw))
}

fn allocate_raw(kind: HandleKind) -> Result<u64, HandleError> {
    let generation = NEXT_HANDLE_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            (current < MAX_HANDLE_GENERATION).then_some(current + 1)
        })
        .map_err(|_| HandleError::GenerationExhausted)?;
    Ok((generation << HANDLE_KIND_BITS) | kind.tag())
}

fn require_kind(
    entries: &BTreeMap<u64, Entry>,
    raw: u64,
    expected: HandleKind,
) -> Result<(), HandleError> {
    match entries.get(&raw) {
        Some(entry) if entry.kind() == expected => Ok(()),
        _ => Err(invalid(expected)),
    }
}

const fn invalid(expected: HandleKind) -> HandleError {
    HandleError::InvalidOrDisposed { expected }
}

fn report_entries(entries: &BTreeMap<u64, Entry>) -> HandleLeakReport {
    let mut report = HandleLeakReport::default();
    for entry in entries.values() {
        match entry.kind() {
            HandleKind::Engine => report.engines += 1,
            HandleKind::Book => report.books += 1,
            HandleKind::Chapter => report.chapters += 1,
            HandleKind::LayoutSession => report.layout_sessions += 1,
            HandleKind::Request => report.requests += 1,
        }
    }
    report
}

/// Catch a Rust panic at a native adapter boundary.
///
/// Existing typed errors pass through unchanged. Panic payloads are not exposed
/// to the host; they become an opaque [`PageletError::Internal`] identifier.
pub fn ffi_boundary<T>(
    operation: impl FnOnce() -> Result<T, PageletError>,
) -> Result<T, PageletError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(_) => Err(PageletError::Internal(next_internal_error_id())),
    }
}

fn next_internal_error_id() -> InternalErrorId {
    let id = NEXT_INTERNAL_ERROR_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .unwrap_or(u64::MAX);
    InternalErrorId(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::Engine,
        layout::LayoutOptions,
        testkit::{FixtureKind, GeneratedEpubFixture},
    };

    struct PopulatedRegistry {
        registry: HandleRegistry,
        engine: EngineHandle,
        book: BookHandle,
        chapter: ChapterHandle,
        layout: LayoutSessionHandle,
        request: RequestHandle,
        cancellation: CancellationToken,
    }

    fn populated_registry() -> PopulatedRegistry {
        let registry = HandleRegistry::new();
        let engine = Engine::new();
        let engine_handle = registry.insert_engine(engine).expect("engine handle");
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let book = engine
            .open_bytes(fixture.bytes().to_vec())
            .expect("open fixture");
        let chapter = book.open_spine_item(0).expect("open chapter");
        let layout = book
            .create_layout_session(0, LayoutOptions::default())
            .expect("layout");
        let book_handle = registry
            .insert_book(engine_handle, book)
            .expect("book handle");
        let chapter_handle = registry
            .insert_chapter(book_handle, chapter)
            .expect("chapter handle");
        let layout_handle = registry
            .insert_layout_session(book_handle, layout)
            .expect("layout handle");
        let cancellation = CancellationToken::new();
        let request_handle = registry
            .insert_request(layout_handle, cancellation.clone())
            .expect("request handle");

        PopulatedRegistry {
            registry,
            engine: engine_handle,
            book: book_handle,
            chapter: chapter_handle,
            layout: layout_handle,
            request: request_handle,
            cancellation,
        }
    }

    #[test]
    fn handle_tokens_are_typed_opaque_generations() {
        let registry = HandleRegistry::new();
        let first = registry.insert_engine(Engine::new()).expect("first");
        assert_eq!(EngineHandle::from_raw(first.as_raw()), Some(first));
        assert_eq!(BookHandle::from_raw(first.as_raw()), None);
        assert_eq!(EngineHandle::from_raw(0), None);
        assert_eq!(
            registry.dispose_engine(first),
            DisposeOutcome::Disposed { descendants: 0 }
        );

        let second = registry.insert_engine(Engine::new()).expect("second");
        assert_ne!(first.as_raw(), second.as_raw());
        assert_eq!(
            registry.engine(first),
            Err(HandleError::InvalidOrDisposed {
                expected: HandleKind::Engine
            })
        );
        assert_eq!(registry.engine(second), Ok(Engine::new()));

        let other_registry = HandleRegistry::new();
        let other = other_registry
            .insert_engine(Engine::new())
            .expect("other registry");
        assert_ne!(second.as_raw(), other.as_raw());
        assert_eq!(
            registry.dispose_engine(second),
            DisposeOutcome::Disposed { descendants: 0 }
        );
        assert_eq!(
            other_registry.dispose_engine(other),
            DisposeOutcome::Disposed { descendants: 0 }
        );
    }

    #[test]
    fn registry_resolves_all_typed_values() {
        let populated = populated_registry();
        assert_eq!(
            populated.registry.engine(populated.engine),
            Ok(Engine::new())
        );
        assert!(!populated
            .registry
            .book(populated.book)
            .expect("book")
            .summary()
            .package
            .spine
            .is_empty());
        assert_eq!(
            populated
                .registry
                .chapter(populated.chapter)
                .expect("chapter")
                .nodes
                .len(),
            populated
                .registry
                .book(populated.book)
                .expect("book")
                .open_spine_item(0)
                .expect("cached chapter")
                .nodes
                .len()
        );
        assert!(populated.registry.layout_session(populated.layout).is_ok());
        assert!(!populated
            .registry
            .request_cancellation(populated.request)
            .expect("request")
            .is_cancelled());
        assert_eq!(
            populated.registry.leak_report(),
            HandleLeakReport {
                engines: 1,
                books: 1,
                chapters: 1,
                layout_sessions: 1,
                requests: 1,
            }
        );
        assert_eq!(
            populated.registry.dispose_engine(populated.engine),
            DisposeOutcome::Disposed { descendants: 4 }
        );
        assert!(populated.cancellation.is_cancelled());
    }

    #[test]
    fn book_dispose_closes_descendants_and_is_idempotent() {
        let populated = populated_registry();
        assert_eq!(
            populated.registry.dispose_book(populated.book),
            DisposeOutcome::Disposed { descendants: 3 }
        );
        assert!(populated.cancellation.is_cancelled());
        assert!(matches!(
            populated.registry.book(populated.book),
            Err(HandleError::InvalidOrDisposed {
                expected: HandleKind::Book
            })
        ));
        assert_eq!(
            populated.registry.chapter(populated.chapter),
            Err(HandleError::InvalidOrDisposed {
                expected: HandleKind::Chapter
            })
        );
        assert!(matches!(
            populated.registry.layout_session(populated.layout),
            Err(HandleError::InvalidOrDisposed {
                expected: HandleKind::LayoutSession
            })
        ));
        assert!(matches!(
            populated.registry.request_cancellation(populated.request),
            Err(HandleError::InvalidOrDisposed {
                expected: HandleKind::Request
            })
        ));
        assert_eq!(
            populated.registry.dispose_book(populated.book),
            DisposeOutcome::AlreadyDisposed
        );
        assert_eq!(
            populated.registry.engine(populated.engine),
            Ok(Engine::new())
        );
        assert_eq!(
            populated.registry.dispose_engine(populated.engine),
            DisposeOutcome::Disposed { descendants: 0 }
        );
    }

    #[test]
    fn layout_dispose_cancels_requests_without_closing_book_or_chapter() {
        let populated = populated_registry();
        assert_eq!(
            populated.registry.dispose_layout_session(populated.layout),
            DisposeOutcome::Disposed { descendants: 1 }
        );
        assert!(populated.cancellation.is_cancelled());
        assert!(populated.registry.book(populated.book).is_ok());
        assert!(populated.registry.chapter(populated.chapter).is_ok());
        assert_eq!(
            populated.registry.dispose_engine(populated.engine),
            DisposeOutcome::Disposed { descendants: 2 }
        );
    }

    #[test]
    fn disposed_parent_cannot_adopt_new_children() {
        let registry = HandleRegistry::new();
        let engine = Engine::new();
        let engine_handle = registry.insert_engine(engine).expect("engine");
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let book = engine
            .open_bytes(fixture.bytes().to_vec())
            .expect("open fixture");
        assert_eq!(
            registry.dispose_engine(engine_handle),
            DisposeOutcome::Disposed { descendants: 0 }
        );
        assert!(matches!(
            registry.insert_book(engine_handle, book),
            Err(HandleError::InvalidOrDisposed {
                expected: HandleKind::Engine
            })
        ));
    }

    #[test]
    fn panic_boundary_preserves_errors_and_maps_panics_to_internal() {
        assert_eq!(
            ffi_boundary::<()>(|| Err(PageletError::Cancelled)),
            Err(PageletError::Cancelled)
        );

        let first = ffi_boundary::<()>(|| panic!("private panic detail"))
            .expect_err("panic must become an error");
        let second =
            ffi_boundary::<()>(|| panic!("another panic")).expect_err("panic must become an error");
        let PageletError::Internal(first_id) = first else {
            panic!("expected internal error");
        };
        let PageletError::Internal(second_id) = second else {
            panic!("expected internal error");
        };
        assert_ne!(first_id, second_id);
        assert_ne!(first_id.0, 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_drop_detects_and_releases_leaked_handles() {
        let leaked = std::panic::catch_unwind(|| {
            let registry = HandleRegistry::new();
            registry.insert_engine(Engine::new()).expect("engine");
            drop(registry);
        });
        let payload = leaked.expect_err("debug drop must detect a leak");
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(message.contains("leaked opaque handles"));
    }
}
