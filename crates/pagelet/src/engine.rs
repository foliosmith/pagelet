//! Stateful engine facade and generation-aware request scheduler.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use crate::{
    core::{CancellationToken, PageletError, ResourceLimits},
    document::{BookIr, ChapterIr, ResourceKind},
    epub::{
        self, BookSummary, CompatibilityMode, Navigation, OpenOptions, ResourceBytes, ResourceId,
    },
    layout::{HostMeasuredLayout, HostMeasuredPagination, LayoutOptions, PaginatedDocument},
    text::{MeasureBatch, MeasuredBatch},
};

/// Maximum memory assigned to session-owned caches.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct CacheBudget {
    pub bytes: usize,
}

impl Default for CacheBudget {
    fn default() -> Self {
        Self { bytes: 32 << 20 }
    }
}

/// Immutable engine configuration.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EngineConfig {
    pub compatibility: CompatibilityMode,
    pub limits: ResourceLimits,
    pub cache_budget: CacheBudget,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            compatibility: CompatibilityMode::Compatible,
            limits: ResourceLimits::default(),
            cache_budget: CacheBudget::default(),
        }
    }
}

/// Builder for an isolated [`Engine`].
#[derive(Debug, Default, Clone, Copy)]
pub struct EngineBuilder {
    config: EngineConfig,
}

impl EngineBuilder {
    #[must_use]
    pub const fn compatibility(mut self, compatibility: CompatibilityMode) -> Self {
        self.config.compatibility = compatibility;
        self
    }

    #[must_use]
    pub const fn limits(mut self, limits: ResourceLimits) -> Self {
        self.config.limits = limits;
        self
    }

    #[must_use]
    pub const fn cache_budget(mut self, cache_budget: CacheBudget) -> Self {
        self.config.cache_budget = cache_budget;
        self
    }

    #[must_use]
    pub const fn build(self) -> Engine {
        Engine {
            config: self.config,
        }
    }
}

/// Root facade. Every instance owns its configuration; no mutable singleton is used.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Engine {
    config: EngineConfig,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            config: EngineConfig {
                compatibility: CompatibilityMode::Compatible,
                limits: ResourceLimits::mobile_defaults(),
                cache_budget: CacheBudget { bytes: 32 << 20 },
            },
        }
    }

    #[must_use]
    pub const fn builder() -> EngineBuilder {
        EngineBuilder {
            config: EngineConfig {
                compatibility: CompatibilityMode::Compatible,
                limits: ResourceLimits::mobile_defaults(),
                cache_budget: CacheBudget { bytes: 32 << 20 },
            },
        }
    }

    #[must_use]
    pub const fn config(&self) -> EngineConfig {
        self.config
    }

    pub fn open_bytes(&self, bytes: impl Into<Vec<u8>>) -> Result<BookSession, PageletError> {
        let options = OpenOptions {
            compatibility_mode: self.config.compatibility,
            limits: self.config.limits,
        };
        let opened = epub::open_book_session_context(bytes, options)?;
        let package = Arc::new(epub::book_ir_from_opened(&opened, options)?);
        Ok(BookSession {
            inner: Arc::new(BookSessionInner {
                opened,
                package,
                options,
                chapters: Mutex::new(BTreeMap::new()),
                resources: Mutex::new(BTreeMap::new()),
                chapter_cache_hits: AtomicU64::new(0),
                chapter_cache_misses: AtomicU64::new(0),
            }),
        })
    }

    pub fn open_path(&self, path: impl AsRef<Path>) -> Result<BookSession, PageletError> {
        self.open_bytes(fs::read(path)?)
    }
}

#[derive(Debug)]
struct BookSessionInner {
    opened: epub::OpenedBook,
    package: Arc<BookIr>,
    options: OpenOptions,
    chapters: Mutex<BTreeMap<usize, Arc<ChapterIr>>>,
    resources: Mutex<BTreeMap<ResourceId, Arc<ResourceBytes>>>,
    chapter_cache_hits: AtomicU64,
    chapter_cache_misses: AtomicU64,
}

/// Observable counters for caches currently owned by one book session.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct EngineStats {
    /// ChapterIR lookups served from the book-session cache.
    pub chapter_cache_hits: u64,
    /// ChapterIR lookups that required parsing before insertion.
    pub chapter_cache_misses: u64,
}

/// An opened publication. Dropping the last clone releases its index and chapter cache.
#[derive(Debug, Clone)]
pub struct BookSession {
    inner: Arc<BookSessionInner>,
}

impl BookSession {
    /// Return the package-level IR parsed once when the book session opened.
    #[must_use]
    pub fn package(&self) -> &BookIr {
        &self.inner.package
    }

    #[must_use]
    pub fn summary(&self) -> &BookSummary {
        &self.inner.opened.summary
    }

    #[must_use]
    pub fn navigation(&self) -> &Navigation {
        &self.inner.opened.summary.navigation
    }

    pub fn open_spine_item(&self, spine_index: usize) -> Result<Arc<ChapterIr>, PageletError> {
        if let Some(chapter) = self
            .inner
            .chapters
            .lock()
            .expect("chapter cache poisoned")
            .get(&spine_index)
        {
            self.inner
                .chapter_cache_hits
                .fetch_add(1, Ordering::Relaxed);
            return Ok(Arc::clone(chapter));
        }
        self.inner
            .chapter_cache_misses
            .fetch_add(1, Ordering::Relaxed);
        let chapter = Arc::new(epub::open_spine_item_from_context(
            &self.inner.opened,
            &self.inner.package,
            spine_index,
            self.inner.options,
        )?);
        let mut chapters = self.inner.chapters.lock().expect("chapter cache poisoned");
        Ok(Arc::clone(chapters.entry(spine_index).or_insert(chapter)))
    }

    /// Return a point-in-time snapshot of session-owned cache counters.
    #[must_use]
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            chapter_cache_hits: self.inner.chapter_cache_hits.load(Ordering::Relaxed),
            chapter_cache_misses: self.inner.chapter_cache_misses.load(Ordering::Relaxed),
        }
    }

    pub fn create_layout_session(
        &self,
        spine_index: usize,
        options: LayoutOptions,
    ) -> Result<LayoutSession, PageletError> {
        Ok(LayoutSession::prepare(
            self.open_spine_item(spine_index)?,
            options,
        ))
    }

    /// Read one indexed publication resource without reopening the EPUB.
    pub fn read_resource(&self, resource_id: ResourceId) -> Result<ResourceBytes, PageletError> {
        if let Some(resource) = self
            .inner
            .resources
            .lock()
            .expect("resource cache poisoned")
            .get(&resource_id)
        {
            return Ok((**resource).clone());
        }
        let resource = epub::read_resource_from_context(&self.inner.opened, resource_id)?;
        let cacheable = self
            .inner
            .package
            .resources
            .resources
            .iter()
            .find(|entry| entry.id == resource_id)
            .is_some_and(|entry| matches!(entry.kind, ResourceKind::Css | ResourceKind::Font));
        if cacheable {
            self.inner
                .resources
                .lock()
                .expect("resource cache poisoned")
                .insert(resource_id, Arc::new(resource.clone()));
        }
        Ok(resource)
    }
}

/// Pages requested from a prepared chapter.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PageRequest {
    pub start_page: usize,
    pub max_pages: usize,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            start_page: 0,
            max_pages: usize::MAX,
        }
    }
}

/// Observable states of host-measured layout.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LayoutProgress {
    NeedMeasurements(MeasureBatch),
    Pages(PaginatedDocument),
    Complete,
}

/// One prepared chapter layout with an explicit measurement lifecycle.
#[derive(Debug)]
pub struct LayoutSession {
    prepared: Option<HostMeasuredLayout>,
    pagination: Option<HostMeasuredPagination>,
    request: PageRequest,
}

impl LayoutSession {
    /// Prepare a host-measured layout session for one cached chapter.
    #[must_use]
    pub fn prepare(chapter: Arc<ChapterIr>, options: LayoutOptions) -> Self {
        Self {
            prepared: Some(HostMeasuredLayout::prepare((*chapter).clone(), options)),
            pagination: None,
            request: PageRequest::default(),
        }
    }

    /// Return the last accepted page document, if measurements completed.
    #[must_use]
    pub fn document(&self) -> Option<&PaginatedDocument> {
        self.pagination
            .as_ref()
            .map(HostMeasuredPagination::document)
    }

    pub fn layout(&mut self, request: PageRequest) -> Result<LayoutProgress, PageletError> {
        self.request = request;
        if let Some(pagination) = &mut self.pagination {
            if pagination.document().complete
                && request.start_page >= pagination.document().pages.len()
            {
                return Ok(LayoutProgress::Complete);
            }
            return pagination
                .page_range(request.start_page, request.max_pages)
                .map(LayoutProgress::Pages);
        }
        let batch = self
            .prepared
            .as_ref()
            .expect("layout already completed")
            .measure_batch()
            .clone();
        Ok(LayoutProgress::NeedMeasurements(batch))
    }

    pub fn submit_measurements(
        &mut self,
        measured: MeasuredBatch,
    ) -> Result<LayoutProgress, PageletError> {
        if self.pagination.is_some() {
            return Ok(LayoutProgress::Complete);
        }
        let prepared = self
            .prepared
            .as_ref()
            .expect("measurements submitted more than once")
            .clone();
        let mut pagination = prepared.resume_progressively(measured)?;
        let document = pagination.page_range(self.request.start_page, self.request.max_pages)?;
        self.prepared = None;
        self.pagination = Some(pagination);
        Ok(LayoutProgress::Pages(document))
    }
}

/// Worker concurrency limits.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WorkerConfig {
    pub foreground_threads: usize,
    pub background_threads: usize,
    pub max_inflight_chapters: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            foreground_threads: 1,
            background_threads: 1,
            max_inflight_chapters: 2,
        }
    }
}

/// Monotonic request generation.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Generation(pub u64);

/// Scheduling lane used to keep visible-page work ahead of prefetch.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum WorkPriority {
    Foreground,
    Background,
}

/// Cancellation and generation identity attached to scheduled work.
#[derive(Debug, Clone)]
pub struct ScheduledRequest {
    id: u64,
    generation: Generation,
    priority: WorkPriority,
    cancellation: CancellationToken,
}

impl ScheduledRequest {
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }
    #[must_use]
    pub const fn priority(&self) -> WorkPriority {
        self.priority
    }
    #[must_use]
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

/// Generation-aware scheduler. Work may finish after invalidation, but stale results are rejected.
#[derive(Debug)]
pub struct Scheduler {
    config: WorkerConfig,
    current: Generation,
    next_request_id: u64,
    requests: Vec<ScheduledRequest>,
}

impl Scheduler {
    #[must_use]
    pub fn new(config: WorkerConfig) -> Self {
        assert!(
            config.foreground_threads > 0,
            "foreground_threads must be positive"
        );
        assert!(
            config.max_inflight_chapters > 0,
            "max_inflight_chapters must be positive"
        );
        assert!(
            config.foreground_threads + config.background_threads > 0,
            "at least one worker thread must be configured"
        );
        Self {
            config,
            current: Generation(0),
            next_request_id: 0,
            requests: Vec::new(),
        }
    }

    #[must_use]
    pub const fn config(&self) -> WorkerConfig {
        self.config
    }
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.current
    }

    pub fn schedule(&mut self) -> Option<ScheduledRequest> {
        self.schedule_with_priority(WorkPriority::Foreground)
    }

    pub fn schedule_with_priority(&mut self, priority: WorkPriority) -> Option<ScheduledRequest> {
        self.requests
            .retain(|request| !request.cancellation.is_cancelled());
        if self.requests.len() >= self.config.max_inflight_chapters {
            return None;
        }
        let lane_capacity = match priority {
            WorkPriority::Foreground => self.config.foreground_threads,
            WorkPriority::Background => self.config.background_threads,
        };
        if self
            .requests
            .iter()
            .filter(|request| request.priority == priority)
            .count()
            >= lane_capacity
        {
            return None;
        }
        let request = ScheduledRequest {
            id: self.next_request_id,
            generation: self.current,
            priority,
            cancellation: CancellationToken::new(),
        };
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.requests.push(request.clone());
        Some(request)
    }

    /// Retire a finished request and report whether its result is still current.
    pub fn complete(&mut self, request: &ScheduledRequest) -> bool {
        let accepted = self.accepts(request);
        self.requests.retain(|active| active.id != request.id);
        accepted
    }

    /// Start a new layout generation after a resize or font change.
    pub fn invalidate(&mut self) -> Generation {
        self.current.0 = self.current.0.wrapping_add(1);
        for request in &self.requests {
            request.cancel();
        }
        self.requests.clear();
        self.current
    }

    /// Accept only results belonging to the current generation.
    #[must_use]
    pub fn accepts(&self, request: &ScheduledRequest) -> bool {
        request.generation == self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testkit::{EpubFixtureBuilder, FixtureKind, GeneratedEpubFixture},
        text::{DefaultTextBackend, TextBackend},
    };

    #[test]
    fn book_session_reuses_chapter_identity_and_exposes_metadata() {
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let session = Engine::new()
            .open_bytes(fixture.bytes().to_vec())
            .expect("open fixture");
        assert!(!session.summary().package.spine.is_empty());
        assert_eq!(session.navigation(), &session.summary().navigation);
        let first = session.open_spine_item(0).expect("first open");
        let second = session.open_spine_item(0).expect("cached open");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            session.stats(),
            EngineStats {
                chapter_cache_hits: 1,
                chapter_cache_misses: 1,
            }
        );
    }

    #[test]
    fn scheduler_rejects_stale_generation_results() {
        let mut scheduler = Scheduler::new(WorkerConfig::default());
        let old = scheduler.schedule().expect("request");
        assert!(scheduler.accepts(&old));
        assert_eq!(scheduler.invalidate(), Generation(1));
        assert!(old.cancellation().is_cancelled());
        assert!(!scheduler.accepts(&old));
        let current = scheduler.schedule().expect("current request");
        assert!(scheduler.accepts(&current));
        assert!(scheduler.complete(&current));
        assert!(scheduler.schedule().is_some());
    }

    #[test]
    fn scheduler_enforces_priority_lane_capacity() {
        let mut scheduler = Scheduler::new(WorkerConfig {
            foreground_threads: 1,
            background_threads: 1,
            max_inflight_chapters: 2,
        });
        let foreground = scheduler.schedule().expect("foreground");
        assert!(scheduler.schedule().is_none());
        let background = scheduler
            .schedule_with_priority(WorkPriority::Background)
            .expect("background");
        assert_eq!(background.priority(), WorkPriority::Background);
        assert!(scheduler.complete(&foreground));
        assert!(scheduler.complete(&background));
    }

    #[test]
    fn builder_keeps_engines_isolated() {
        let strict = Engine::builder()
            .compatibility(CompatibilityMode::Strict)
            .build();
        let compatible = Engine::new();
        assert_eq!(strict.config().compatibility, CompatibilityMode::Strict);
        assert_eq!(
            compatible.config().compatibility,
            CompatibilityMode::Compatible
        );
    }

    #[test]
    fn layout_session_advances_through_measure_pages_and_complete() {
        let body = (0..12)
            .map(|index| format!("<p>Progressive paragraph {index}.</p>"))
            .collect::<String>();
        let fixture = EpubFixtureBuilder::epub3(FixtureKind::MinimalEpub3, "Progressive")
            .add_xhtml("EPUB/chapter-1.xhtml", "Chapter 1", body)
            .build();
        let book = Engine::new()
            .open_bytes(fixture.bytes().to_vec())
            .expect("open fixture");
        let mut layout = book
            .create_layout_session(
                0,
                LayoutOptions::new(crate::layout::LayoutConstraints::new(
                    crate::core::LayoutUnit::from_px(320),
                    crate::core::LayoutUnit::from_px(96),
                )),
            )
            .expect("layout session");
        let LayoutProgress::NeedMeasurements(batch) = layout
            .layout(PageRequest {
                start_page: 0,
                max_pages: 1,
            })
            .expect("prepare layout")
        else {
            panic!("layout must request measurements first");
        };
        let measured = DefaultTextBackend::default()
            .measure_batch(&batch, &CancellationToken::new())
            .expect("measure batch");
        let LayoutProgress::Pages(document) = layout
            .submit_measurements(measured)
            .expect("submit measurements")
        else {
            panic!("measurements must produce pages");
        };
        assert_eq!(document.pages.len(), 1);
        assert!(!document.complete);
        let LayoutProgress::Pages(next) = layout
            .layout(PageRequest {
                start_page: 1,
                max_pages: 1,
            })
            .expect("request next page")
        else {
            panic!("next range must produce pages");
        };
        assert_eq!(next.pages.first().map(|page| page.page_index), Some(1));

        let retained = layout.document().expect("retained progressive document");
        assert_eq!(retained.pages.len(), 2);
    }
}
