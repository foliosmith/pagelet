//! Transport-neutral control-plane operations used by native adapters.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{
    core::{CancellationToken, LayoutUnit, PageletError, ProtocolError, ResourceId, TextAnchor},
    engine::{Engine, LayoutProgress, LayoutSession, PageRequest},
    epub::{BookSummary, Navigation, ResourceBytes},
    layout::{self, HitTestResult, LayoutOptions},
    text::MeasuredBatch,
};

use super::{
    ffi_boundary, BookHandle, ChapterHandle, DisposeOutcome, EngineHandle, HandleRegistry,
    LayoutSessionHandle, RequestHandle,
};

/// Result of starting one host-measured layout request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ControlLayoutRequest {
    /// Cancellable request handle, present while host measurements are needed.
    pub request: Option<RequestHandle>,
    /// Current layout lifecycle state.
    pub progress: LayoutProgress,
}

/// Typed control plane shared by C and generated host adapters.
///
/// Every public operation catches panics before returning to its caller.
#[derive(Debug, Default)]
pub struct ControlPlane {
    registry: HandleRegistry,
}

impl ControlPlane {
    /// Create an empty control plane.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an engine with mobile-safe defaults.
    pub fn engine_create(&self) -> Result<EngineHandle, PageletError> {
        ffi_boundary(|| Ok(self.registry.insert_engine(Engine::new())?))
    }

    /// Open an EPUB path under a live engine.
    pub fn book_open_path(
        &self,
        engine: EngineHandle,
        path: &Path,
    ) -> Result<BookHandle, PageletError> {
        ffi_boundary(|| {
            let engine_value = self.registry.engine(engine)?;
            let book = engine_value.open_path(path)?;
            Ok(self.registry.insert_book(engine, book)?)
        })
    }

    /// Open an EPUB from a borrowed file descriptor without adopting ownership.
    ///
    /// The file position is restored before returning.
    pub fn book_open_fd(
        &self,
        engine: EngineHandle,
        file: &File,
    ) -> Result<BookHandle, PageletError> {
        ffi_boundary(|| {
            let engine_value = self.registry.engine(engine)?;
            let mut borrowed = file.try_clone()?;
            let original_position = borrowed.stream_position()?;
            borrowed.seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new();
            let read_result = borrowed.read_to_end(&mut bytes);
            let restore_result = borrowed.seek(SeekFrom::Start(original_position));
            read_result?;
            restore_result?;
            let book = engine_value.open_bytes(bytes)?;
            Ok(self.registry.insert_book(engine, book)?)
        })
    }

    /// Return an owned book summary DTO.
    pub fn book_summary(&self, book: BookHandle) -> Result<BookSummary, PageletError> {
        ffi_boundary(|| Ok(self.registry.book(book)?.summary().clone()))
    }

    /// Return an owned navigation DTO.
    pub fn book_navigation(&self, book: BookHandle) -> Result<Navigation, PageletError> {
        ffi_boundary(|| Ok(self.registry.book(book)?.navigation().clone()))
    }

    /// Open and register one spine chapter.
    pub fn chapter_open(
        &self,
        book: BookHandle,
        spine_index: usize,
    ) -> Result<ChapterHandle, PageletError> {
        ffi_boundary(|| {
            let chapter = self.registry.book(book)?.open_spine_item(spine_index)?;
            Ok(self.registry.insert_chapter(book, chapter)?)
        })
    }

    /// Create a layout session owned by the chapter's book.
    pub fn layout_session_create(
        &self,
        chapter: ChapterHandle,
        options: LayoutOptions,
    ) -> Result<LayoutSessionHandle, PageletError> {
        ffi_boundary(|| {
            let book = self.registry.chapter_book(chapter)?;
            let chapter_value = self.registry.chapter(chapter)?;
            let session = LayoutSession::prepare(chapter_value, options);
            Ok(self.registry.insert_layout_session(book, session)?)
        })
    }

    /// Start one host-measured layout request.
    pub fn layout_request(
        &self,
        layout: LayoutSessionHandle,
        request: PageRequest,
    ) -> Result<ControlLayoutRequest, PageletError> {
        ffi_boundary(|| {
            if self.registry.has_request_for_layout(layout)? {
                return Err(protocol_error(
                    "layout session already has an in-flight request",
                ));
            }
            let session = self.registry.layout_session(layout)?;
            let progress = lock_layout(&session).layout(request)?;
            let request = if matches!(progress, LayoutProgress::NeedMeasurements(_)) {
                Some(
                    self.registry
                        .insert_request(layout, CancellationToken::new())?,
                )
            } else {
                None
            };
            Ok(ControlLayoutRequest { request, progress })
        })
    }

    /// Submit a complete host measurement batch for one request.
    pub fn layout_submit_measurements(
        &self,
        request: RequestHandle,
        measured: MeasuredBatch,
    ) -> Result<LayoutProgress, PageletError> {
        let result = ffi_boundary(|| {
            let cancellation = self.registry.request_cancellation(request)?;
            if cancellation.is_cancelled() {
                return Err(PageletError::Cancelled);
            }
            let layout = self.registry.request_layout(request)?;
            let session = self.registry.layout_session(layout)?;
            let result = lock_layout(&session).submit_measurements(measured);
            result
        });
        self.registry.dispose_request(request);
        result
    }

    /// Read one indexed publication resource.
    pub fn resource_read(
        &self,
        book: BookHandle,
        resource_id: ResourceId,
    ) -> Result<ResourceBytes, PageletError> {
        ffi_boundary(|| self.registry.book(book)?.read_resource(resource_id))
    }

    /// Hit-test a retained page scene.
    pub fn hit_test(
        &self,
        layout: LayoutSessionHandle,
        page_index: u32,
        x: LayoutUnit,
        y: LayoutUnit,
    ) -> Result<Option<HitTestResult>, PageletError> {
        ffi_boundary(|| {
            let session = self.registry.layout_session(layout)?;
            let session = lock_layout(&session);
            let document = session
                .document()
                .ok_or_else(|| protocol_error("layout session has no accepted page document"))?;
            let page = document
                .pages
                .iter()
                .find(|page| page.page_index == page_index)
                .ok_or_else(|| {
                    protocol_error("page index is not retained by the layout session")
                })?;
            Ok(layout::hit_test(page, x, y))
        })
    }

    /// Resolve a stable text anchor to a retained page index.
    pub fn anchor_to_page(
        &self,
        layout: LayoutSessionHandle,
        anchor: TextAnchor,
    ) -> Result<Option<u32>, PageletError> {
        ffi_boundary(|| {
            let session = self.registry.layout_session(layout)?;
            let session = lock_layout(&session);
            let document = session
                .document()
                .ok_or_else(|| protocol_error("layout session has no accepted page document"))?;
            Ok(layout::anchor_to_page(&document.pages, anchor))
        })
    }

    /// Cancel and retire an in-flight request.
    pub fn request_cancel(&self, request: RequestHandle) -> Result<(), PageletError> {
        let result = ffi_boundary(|| {
            self.registry.request_cancellation(request)?.cancel();
            Ok(())
        });
        self.registry.dispose_request(request);
        result
    }

    /// Dispose any opaque handle and its owned descendants.
    pub fn handle_dispose(&self, raw: u64) -> Result<DisposeOutcome, PageletError> {
        ffi_boundary(|| Ok(self.registry.dispose(raw)))
    }
}

fn lock_layout(session: &Arc<Mutex<LayoutSession>>) -> MutexGuard<'_, LayoutSession> {
    match session.lock() {
        Ok(session) => session,
        Err(poisoned) => {
            session.clear_poison();
            poisoned.into_inner()
        }
    }
}

fn protocol_error(message: impl Into<std::sync::Arc<str>>) -> PageletError {
    PageletError::Protocol(ProtocolError::new(message))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        engine::LayoutProgress,
        testkit::{FixtureKind, GeneratedEpubFixture},
        text::{DefaultTextBackend, TextBackend},
    };

    use super::*;

    static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempEpub {
        path: std::path::PathBuf,
    }

    impl TempEpub {
        fn new(bytes: &[u8]) -> Self {
            let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pagelet-control-{}-{sequence}.epub",
                std::process::id()
            ));
            fs::write(&path, bytes).expect("write temporary EPUB");
            Self { path }
        }
    }

    impl Drop for TempEpub {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn control_plane_runs_complete_host_measured_lifecycle() {
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let temp = TempEpub::new(fixture.bytes());
        let control = ControlPlane::new();

        let engine = control.engine_create().expect("engine");
        let book = control
            .book_open_path(engine, &temp.path)
            .expect("open path");
        let summary = control.book_summary(book).expect("summary");
        assert!(!summary.package.spine.is_empty());
        assert_eq!(
            control.book_navigation(book).expect("navigation"),
            summary.navigation
        );

        let resource = summary.resources.first().expect("resource");
        let resource_bytes = control
            .resource_read(book, resource.id)
            .expect("resource bytes");
        assert_eq!(resource_bytes.id, resource.id);
        assert!(!resource_bytes.bytes.is_empty());

        let chapter = control.chapter_open(book, 0).expect("chapter");
        let layout = control
            .layout_session_create(chapter, LayoutOptions::default())
            .expect("layout");
        let started = control
            .layout_request(layout, PageRequest::default())
            .expect("layout request");
        let request = started.request.expect("request handle");
        let LayoutProgress::NeedMeasurements(batch) = started.progress else {
            panic!("layout must request host measurements");
        };
        let measured = DefaultTextBackend::default()
            .measure_batch(&batch, &CancellationToken::new())
            .expect("measure batch");
        let LayoutProgress::Pages(document) = control
            .layout_submit_measurements(request, measured)
            .expect("submit measurements")
        else {
            panic!("measurements must produce pages");
        };
        let page = document.pages.first().expect("page");
        let paint = page.text_paints.first().expect("text paint");
        let paragraph = page
            .paragraphs
            .iter()
            .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
            .expect("paint paragraph");
        let line = paragraph
            .lines
            .get(usize::try_from(paint.first_line).expect("line index"))
            .expect("paint line");
        let cluster = paragraph
            .clusters
            .iter()
            .find(|cluster| {
                cluster.line_index == paint.first_line
                    && cluster.text_end > paint.visible_text_range.start
                    && cluster.text_start < paint.visible_text_range.end
            })
            .expect("visible cluster");
        let x = paint.paint_origin.x
            + LayoutUnit::from_raw(cluster.x_start.raw().saturating_add(cluster.x_end.raw()) / 2);
        let y = paint.paint_origin.y
            + line.layout_rect.y
            + LayoutUnit::from_raw(line.layout_rect.height.raw() / 2);
        let hit = control
            .hit_test(layout, page.page_index, x, y)
            .expect("hit test")
            .expect("text hit");
        assert_eq!(hit.node_id, paint.node_id);

        let anchor = page.start_anchor.expect("start anchor");
        assert_eq!(
            control
                .anchor_to_page(layout, anchor)
                .expect("anchor lookup"),
            Some(page.page_index)
        );
        assert_eq!(
            control
                .handle_dispose(engine.as_raw())
                .expect("dispose engine"),
            DisposeOutcome::Disposed { descendants: 3 }
        );
    }

    #[test]
    fn borrowed_fd_is_not_adopted_and_position_is_restored() {
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let temp = TempEpub::new(fixture.bytes());
        let control = ControlPlane::new();
        let engine = control.engine_create().expect("engine");
        let mut file = File::open(&temp.path).expect("open temporary EPUB");
        file.seek(SeekFrom::Start(7)).expect("seek input fd");

        let book = control.book_open_fd(engine, &file).expect("open fd");
        assert_eq!(file.stream_position().expect("fd position"), 7);
        assert!(!control
            .book_summary(book)
            .expect("summary")
            .package
            .spine
            .is_empty());
        assert_eq!(
            control
                .handle_dispose(engine.as_raw())
                .expect("dispose engine"),
            DisposeOutcome::Disposed { descendants: 1 }
        );
    }

    #[test]
    fn request_cancel_retires_work_and_keeps_layout_reusable() {
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let temp = TempEpub::new(fixture.bytes());
        let control = ControlPlane::new();
        let engine = control.engine_create().expect("engine");
        let book = control
            .book_open_path(engine, &temp.path)
            .expect("open book");
        let chapter = control.chapter_open(book, 0).expect("chapter");
        let layout = control
            .layout_session_create(chapter, LayoutOptions::default())
            .expect("layout");

        let first = control
            .layout_request(layout, PageRequest::default())
            .expect("first request");
        let request = first.request.expect("request");
        assert!(matches!(
            control.layout_request(layout, PageRequest::default()),
            Err(PageletError::Protocol(_))
        ));
        control.request_cancel(request).expect("cancel request");
        let second = control
            .layout_request(layout, PageRequest::default())
            .expect("layout remains reusable");
        control
            .request_cancel(second.request.expect("second request"))
            .expect("cancel second request");
        assert_eq!(
            control
                .handle_dispose(engine.as_raw())
                .expect("dispose engine"),
            DisposeOutcome::Disposed { descendants: 3 }
        );
    }

    #[test]
    fn rejected_measurements_do_not_poison_the_layout_session() {
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let temp = TempEpub::new(fixture.bytes());
        let control = ControlPlane::new();
        let engine = control.engine_create().expect("engine");
        let book = control
            .book_open_path(engine, &temp.path)
            .expect("open book");
        let chapter = control.chapter_open(book, 0).expect("chapter");
        let layout = control
            .layout_session_create(chapter, LayoutOptions::default())
            .expect("layout");
        let first = control
            .layout_request(layout, PageRequest::default())
            .expect("first request");
        let request = first.request.expect("request");
        let LayoutProgress::NeedMeasurements(batch) = first.progress else {
            panic!("layout must request measurements");
        };
        let backend = DefaultTextBackend::default();
        let invalid = MeasuredBatch::new(backend.backend_id(), backend.font_fingerprint(), vec![]);
        assert!(matches!(
            control.layout_submit_measurements(request, invalid),
            Err(PageletError::Protocol(_))
        ));

        let second = control
            .layout_request(layout, PageRequest::default())
            .expect("retry request");
        let retry_request = second.request.expect("retry handle");
        let LayoutProgress::NeedMeasurements(retry_batch) = second.progress else {
            panic!("retry must request measurements");
        };
        assert_eq!(retry_batch, batch);
        let measured = backend
            .measure_batch(&retry_batch, &CancellationToken::new())
            .expect("valid measurements");
        assert!(matches!(
            control
                .layout_submit_measurements(retry_request, measured)
                .expect("retry succeeds"),
            LayoutProgress::Pages(_)
        ));
        assert_eq!(
            control
                .handle_dispose(engine.as_raw())
                .expect("dispose engine"),
            DisposeOutcome::Disposed { descendants: 3 }
        );
    }
}
