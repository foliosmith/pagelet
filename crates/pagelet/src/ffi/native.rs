//! C ABI shims for the typed control plane and owned byte buffers.
#![allow(unsafe_code)]

use std::{fs::File, path::Path, slice, str, sync::OnceLock};

#[cfg(unix)]
use std::os::fd::BorrowedFd;

use crate::{
    cli,
    core::{
        DiagnosticCode, DocumentId, LayoutUnit, NodeId, PageletError, ProtocolError, ResourceId,
        TextAffinity, TextAnchor,
    },
    engine::{LayoutProgress, PageRequest},
    layout::{LayoutConstraints, LayoutOptions},
    wire::{MeasureBatch as WireMeasureBatch, MeasuredBatch as WireMeasuredBatch, PageBatch},
};

use super::{
    ffi_boundary, next_internal_error_id, BookHandle, BufferError, BufferPool, ChapterHandle,
    ControlPlane, EngineHandle, LayoutSessionHandle, NativeBuffer, RequestHandle,
};

static CONTROL_PLANE: OnceLock<ControlPlane> = OnceLock::new();
static BUFFER_POOL: OnceLock<BufferPool> = OnceLock::new();

/// Stable C status code returned by native adapter calls.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum FfiStatus {
    Ok = 0,
    InvalidArgument = 1,
    Io = 2,
    InvalidContainer = 3,
    InvalidPackage = 4,
    UnsupportedFeature = 5,
    ResourceLimitExceeded = 6,
    Parse = 7,
    Layout = 8,
    Cancelled = 9,
    Protocol = 10,
    Internal = 11,
    BufferTooSmall = 12,
}

/// Borrowed immutable bytes. The caller retains ownership.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FfiByteSlice {
    pub data: *const u8,
    pub len: usize,
}

/// Borrowed mutable bytes. The caller retains ownership.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FfiMutableByteSlice {
    pub data: *mut u8,
    pub len: usize,
}

/// C-compatible page geometry and pagination options.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiLayoutOptions {
    pub viewport_width: i64,
    pub viewport_height: i64,
    pub margin_start: i64,
    pub margin_end: i64,
    pub margin_top: i64,
    pub margin_bottom: i64,
    pub max_pages: u32,
}

impl Default for FfiLayoutOptions {
    fn default() -> Self {
        let options = LayoutOptions::default();
        Self {
            viewport_width: options.constraints.viewport_width.raw(),
            viewport_height: options.constraints.viewport_height.raw(),
            margin_start: options.constraints.margin_start.raw(),
            margin_end: options.constraints.margin_end.raw(),
            margin_top: options.constraints.margin_top.raw(),
            margin_bottom: options.constraints.margin_bottom.raw(),
            max_pages: options.max_pages,
        }
    }
}

/// C-compatible requested page window.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiPageRequest {
    pub start_page: u64,
    pub max_pages: u64,
}

impl Default for FfiPageRequest {
    fn default() -> Self {
        Self {
            start_page: 0,
            max_pages: u64::MAX,
        }
    }
}

/// Result carrying only status.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiStatusResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
}

/// Result carrying one opaque handle.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiHandleResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub handle: u64,
}

/// Result carrying one Rust-owned byte buffer.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiBufferResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub buffer: NativeBuffer,
}

/// Observable state returned by layout request/submit calls.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum FfiLayoutState {
    Complete = 0,
    NeedMeasurements = 1,
    Pages = 2,
}

/// Result carrying a layout state, request handle, and versioned wire buffer.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiLayoutResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub state: FfiLayoutState,
    pub _state_padding: u32,
    pub request: u64,
    pub buffer: NativeBuffer,
}

/// Result carrying resource bytes and copied metadata strings.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiResourceResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub resource_id: u32,
    pub _resource_padding: u32,
    pub bytes: NativeBuffer,
    pub path: NativeBuffer,
    pub media_type: NativeBuffer,
}

/// Result carrying an optional hit-test result.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiHitTestResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub found: u8,
    pub affinity: u8,
    pub _padding: [u8; 6],
    pub node_id: u32,
    pub utf8_byte_offset: u32,
    pub fragment_id: u32,
    pub _tail_padding: u32,
}

/// Result carrying an optional page index.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiAnchorResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub found: u8,
    pub _padding: [u8; 3],
    pub page_index: u32,
}

/// Result of copying a Rust-owned buffer into host-owned memory.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct FfiCopyResult {
    pub status: FfiStatus,
    pub _status_padding: u32,
    pub internal_error_id: u64,
    pub written: usize,
    pub required: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_engine_create() -> FfiHandleResult {
    handle_result(ffi_boundary(|| {
        Ok(control_plane().engine_create()?.as_raw())
    }))
}

/// Open an EPUB from a UTF-8 path.
///
/// # Safety
///
/// `path.data` must be readable for `path.len` bytes for the duration of this
/// call, or it must be null when `path.len` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pagelet_book_open_path(
    engine: u64,
    path: FfiByteSlice,
) -> FfiHandleResult {
    handle_result(ffi_boundary(|| {
        let engine = parse_engine(engine)?;
        // SAFETY: guaranteed by the C ABI contract documented on this function.
        let path = unsafe { borrowed_bytes(path)? };
        let path =
            str::from_utf8(path).map_err(|_| protocol_error("book path is not valid UTF-8"))?;
        Ok(control_plane()
            .book_open_path(engine, Path::new(path))?
            .as_raw())
    }))
}

/// Open an EPUB from a borrowed POSIX file descriptor.
///
/// # Safety
///
/// `fd` must identify an open file for the duration of this call. Ownership is
/// not adopted and the descriptor is never closed by pagelet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pagelet_book_open_fd(engine: u64, fd: i32) -> FfiHandleResult {
    handle_result(ffi_boundary(|| {
        let engine = parse_engine(engine)?;
        if fd < 0 {
            return Err(protocol_error("file descriptor must be non-negative"));
        }
        #[cfg(unix)]
        {
            // SAFETY: guaranteed by this function's C ABI contract.
            let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
            let owned = borrowed.try_clone_to_owned()?;
            let file = File::from(owned);
            Ok(control_plane().book_open_fd(engine, &file)?.as_raw())
        }
        #[cfg(not(unix))]
        {
            let _ = engine;
            Err(PageletError::UnsupportedFeature(
                crate::core::UnsupportedFeature::new(
                    "book_open_fd is supported only on POSIX targets",
                ),
            ))
        }
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_book_summary(book: u64) -> FfiBufferResult {
    buffer_result(ffi_boundary(|| {
        let summary = control_plane().book_summary(parse_book(book)?)?;
        allocate_buffer(cli::book_summary_json(&summary).into_bytes())
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_book_navigation(book: u64) -> FfiBufferResult {
    buffer_result(ffi_boundary(|| {
        let navigation = control_plane().book_navigation(parse_book(book)?)?;
        allocate_buffer(cli::navigation_json(&navigation).into_bytes())
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_chapter_open(book: u64, spine_index: u64) -> FfiHandleResult {
    handle_result(ffi_boundary(|| {
        let spine_index = usize::try_from(spine_index)
            .map_err(|_| protocol_error("spine index does not fit this target"))?;
        Ok(control_plane()
            .chapter_open(parse_book(book)?, spine_index)?
            .as_raw())
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_layout_session_create(
    chapter: u64,
    options: FfiLayoutOptions,
) -> FfiHandleResult {
    handle_result(ffi_boundary(|| {
        Ok(control_plane()
            .layout_session_create(parse_chapter(chapter)?, layout_options(options)?)?
            .as_raw())
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_layout_request(layout: u64, request: FfiPageRequest) -> FfiLayoutResult {
    layout_result(ffi_boundary(|| {
        let request = page_request(request)?;
        let result = control_plane().layout_request(parse_layout(layout)?, request)?;
        let request_handle = result.request.map_or(0, RequestHandle::as_raw);
        match result.progress {
            LayoutProgress::NeedMeasurements(batch) => {
                let encoded = WireMeasureBatch::from(batch).encode().map_err(wire_error)?;
                match allocate_buffer(encoded) {
                    Ok(buffer) => Ok((FfiLayoutState::NeedMeasurements, request_handle, buffer)),
                    Err(error) => {
                        if let Some(request) = result.request {
                            let _ = control_plane().request_cancel(request);
                        }
                        Err(error)
                    }
                }
            }
            LayoutProgress::Pages(document) => Ok((
                FfiLayoutState::Pages,
                request_handle,
                allocate_page_batch(document.pages)?,
            )),
            LayoutProgress::Complete => Ok((
                FfiLayoutState::Complete,
                request_handle,
                NativeBuffer::EMPTY,
            )),
        }
    }))
}

/// Submit a versioned `MeasuredBatch` wire payload.
///
/// # Safety
///
/// `measured.data` must be readable for `measured.len` bytes for the duration
/// of this call, or it must be null when `measured.len` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pagelet_layout_submit_measurements(
    request: u64,
    measured: FfiByteSlice,
) -> FfiLayoutResult {
    layout_result(ffi_boundary(|| {
        // SAFETY: guaranteed by the C ABI contract documented on this function.
        let measured = unsafe { borrowed_bytes(measured)? };
        let measured = WireMeasuredBatch::decode(measured)
            .map_err(wire_error)?
            .into_text_batch();
        match control_plane().layout_submit_measurements(parse_request(request)?, measured)? {
            LayoutProgress::NeedMeasurements(batch) => Ok((
                FfiLayoutState::NeedMeasurements,
                request,
                allocate_buffer(WireMeasureBatch::from(batch).encode().map_err(wire_error)?)?,
            )),
            LayoutProgress::Pages(document) => Ok((
                FfiLayoutState::Pages,
                0,
                allocate_page_batch(document.pages)?,
            )),
            LayoutProgress::Complete => Ok((FfiLayoutState::Complete, 0, NativeBuffer::EMPTY)),
        }
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_resource_read(book: u64, resource_id: u32) -> FfiResourceResult {
    resource_result(ffi_boundary(|| {
        let resource =
            control_plane().resource_read(parse_book(book)?, ResourceId::new(resource_id))?;
        let bytes = allocate_buffer(resource.bytes)?;
        let path = match allocate_buffer(resource.path.as_bytes().to_vec()) {
            Ok(path) => path,
            Err(error) => {
                buffer_pool().release(bytes);
                return Err(error);
            }
        };
        let media_type = match allocate_buffer(resource.media_type.as_str().as_bytes().to_vec()) {
            Ok(media_type) => media_type,
            Err(error) => {
                buffer_pool().release(bytes);
                buffer_pool().release(path);
                return Err(error);
            }
        };
        Ok((resource.id.get(), bytes, path, media_type))
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_hit_test(
    layout: u64,
    page_index: u32,
    x: i64,
    y: i64,
) -> FfiHitTestResult {
    hit_test_result(ffi_boundary(|| {
        control_plane().hit_test(
            parse_layout(layout)?,
            page_index,
            LayoutUnit::from_raw(x),
            LayoutUnit::from_raw(y),
        )
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_anchor_to_page(
    layout: u64,
    document_id: u32,
    node_id: u32,
    utf8_byte_offset: u32,
    affinity: u32,
) -> FfiAnchorResult {
    anchor_result(ffi_boundary(|| {
        let affinity = match affinity {
            0 => TextAffinity::Upstream,
            1 => TextAffinity::Downstream,
            _ => return Err(protocol_error("text affinity must be 0 or 1")),
        };
        control_plane().anchor_to_page(
            parse_layout(layout)?,
            TextAnchor::new(
                DocumentId::new(document_id),
                NodeId::new(node_id),
                utf8_byte_offset,
                affinity,
            ),
        )
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_request_cancel(request: u64) -> FfiStatusResult {
    status_result(ffi_boundary(|| {
        control_plane().request_cancel(parse_request(request)?)
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_handle_dispose(handle: u64) -> FfiStatusResult {
    status_result(ffi_boundary(|| {
        control_plane().handle_dispose(handle)?;
        Ok(())
    }))
}

/// Copy a Rust-owned buffer into caller-owned memory.
///
/// # Safety
///
/// `destination.data` must be writable for `destination.len` bytes for the
/// duration of this call, or it must be null when `destination.len` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pagelet_buffer_copy(
    buffer: NativeBuffer,
    destination: FfiMutableByteSlice,
) -> FfiCopyResult {
    let result = ffi_boundary(|| {
        // SAFETY: guaranteed by the C ABI contract documented on this function.
        let destination = unsafe { borrowed_bytes_mut(destination)? };
        buffer_pool()
            .copy_to(buffer, destination)
            .map_err(buffer_error)
    });
    match result {
        Ok(written) => FfiCopyResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            written,
            required: written,
        },
        Err(PageletError::Protocol(error))
            if error.message.starts_with("destination too small:") =>
        {
            FfiCopyResult {
                status: FfiStatus::BufferTooSmall,
                _status_padding: 0,
                internal_error_id: 0,
                written: 0,
                required: buffer.len,
            }
        }
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiCopyResult {
                status,
                _status_padding: 0,
                internal_error_id,
                written: 0,
                required: buffer.len,
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_buffer_free(buffer: NativeBuffer) -> FfiStatusResult {
    status_result(ffi_boundary(|| {
        buffer_pool().release(buffer);
        Ok(())
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn pagelet_debug_live_buffer_count() -> usize {
    std::panic::catch_unwind(|| buffer_pool().live_count()).unwrap_or(usize::MAX)
}

fn control_plane() -> &'static ControlPlane {
    CONTROL_PLANE.get_or_init(ControlPlane::new)
}

fn buffer_pool() -> &'static BufferPool {
    BUFFER_POOL.get_or_init(BufferPool::new)
}

fn parse_engine(raw: u64) -> Result<EngineHandle, PageletError> {
    EngineHandle::from_raw(raw).ok_or_else(|| protocol_error("invalid engine handle token"))
}

fn parse_book(raw: u64) -> Result<BookHandle, PageletError> {
    BookHandle::from_raw(raw).ok_or_else(|| protocol_error("invalid book handle token"))
}

fn parse_chapter(raw: u64) -> Result<ChapterHandle, PageletError> {
    ChapterHandle::from_raw(raw).ok_or_else(|| protocol_error("invalid chapter handle token"))
}

fn parse_layout(raw: u64) -> Result<LayoutSessionHandle, PageletError> {
    LayoutSessionHandle::from_raw(raw)
        .ok_or_else(|| protocol_error("invalid layout session handle token"))
}

fn parse_request(raw: u64) -> Result<RequestHandle, PageletError> {
    RequestHandle::from_raw(raw).ok_or_else(|| protocol_error("invalid request handle token"))
}

fn layout_options(options: FfiLayoutOptions) -> Result<LayoutOptions, PageletError> {
    if options.viewport_width <= 0 || options.viewport_height <= 0 {
        return Err(protocol_error(
            "layout viewport dimensions must be positive",
        ));
    }
    if options.max_pages == 0 {
        return Err(protocol_error("layout max_pages must be positive"));
    }
    let mut layout = LayoutOptions::new(LayoutConstraints {
        viewport_width: LayoutUnit::from_raw(options.viewport_width),
        viewport_height: LayoutUnit::from_raw(options.viewport_height),
        margin_start: LayoutUnit::from_raw(options.margin_start),
        margin_end: LayoutUnit::from_raw(options.margin_end),
        margin_top: LayoutUnit::from_raw(options.margin_top),
        margin_bottom: LayoutUnit::from_raw(options.margin_bottom),
    });
    if layout.constraints.content_width() == LayoutUnit::ZERO
        || layout.constraints.content_height() == LayoutUnit::ZERO
    {
        return Err(protocol_error("layout margins consume the viewport"));
    }
    layout.max_pages = options.max_pages;
    Ok(layout)
}

fn page_request(request: FfiPageRequest) -> Result<PageRequest, PageletError> {
    Ok(PageRequest {
        start_page: usize::try_from(request.start_page)
            .map_err(|_| protocol_error("start page does not fit this target"))?,
        max_pages: if request.max_pages == u64::MAX {
            usize::MAX
        } else {
            usize::try_from(request.max_pages)
                .map_err(|_| protocol_error("max pages does not fit this target"))?
        },
    })
}

fn allocate_page_batch(pages: Vec<crate::layout::PageScene>) -> Result<NativeBuffer, PageletError> {
    allocate_buffer(PageBatch::new(pages).encode().map_err(wire_error)?)
}

fn allocate_buffer(bytes: Vec<u8>) -> Result<NativeBuffer, PageletError> {
    buffer_pool().allocate(bytes).map_err(buffer_error)
}

fn wire_error(error: crate::wire::WireError) -> PageletError {
    protocol_error(error.to_string())
}

fn buffer_error(error: BufferError) -> PageletError {
    match error {
        BufferError::GenerationExhausted => PageletError::Internal(next_internal_error_id()),
        BufferError::InvalidOrReleased => protocol_error(error.to_string()),
        BufferError::DestinationTooSmall {
            required,
            available,
        } => protocol_error(format!(
            "destination too small: required={required}, available={available}"
        )),
    }
}

fn protocol_error(message: impl Into<std::sync::Arc<str>>) -> PageletError {
    PageletError::Protocol(ProtocolError::new(message))
}

unsafe fn borrowed_bytes<'a>(bytes: FfiByteSlice) -> Result<&'a [u8], PageletError> {
    if bytes.len == 0 {
        return Ok(&[]);
    }
    if bytes.data.is_null() {
        return Err(protocol_error("non-empty byte slice has a null pointer"));
    }
    // SAFETY: the caller guarantees a readable allocation of `len` bytes.
    Ok(unsafe { slice::from_raw_parts(bytes.data, bytes.len) })
}

unsafe fn borrowed_bytes_mut<'a>(bytes: FfiMutableByteSlice) -> Result<&'a mut [u8], PageletError> {
    if bytes.len == 0 {
        return Ok(&mut []);
    }
    if bytes.data.is_null() {
        return Err(protocol_error(
            "non-empty mutable byte slice has a null pointer",
        ));
    }
    // SAFETY: the caller guarantees a writable allocation of `len` bytes.
    Ok(unsafe { slice::from_raw_parts_mut(bytes.data, bytes.len) })
}

fn error_status(error: &PageletError) -> (FfiStatus, u64) {
    let status = match error.code() {
        DiagnosticCode::Io => FfiStatus::Io,
        DiagnosticCode::InvalidContainer => FfiStatus::InvalidContainer,
        DiagnosticCode::InvalidPackage => FfiStatus::InvalidPackage,
        DiagnosticCode::UnsupportedFeature => FfiStatus::UnsupportedFeature,
        DiagnosticCode::ResourceLimitExceeded => FfiStatus::ResourceLimitExceeded,
        DiagnosticCode::Parse => FfiStatus::Parse,
        DiagnosticCode::Layout => FfiStatus::Layout,
        DiagnosticCode::Cancelled => FfiStatus::Cancelled,
        DiagnosticCode::Protocol => FfiStatus::Protocol,
        DiagnosticCode::Internal => FfiStatus::Internal,
    };
    let internal_error_id = match error {
        PageletError::Internal(id) => id.0,
        _ => 0,
    };
    (status, internal_error_id)
}

fn status_result(result: Result<(), PageletError>) -> FfiStatusResult {
    match result {
        Ok(()) => FfiStatusResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiStatusResult {
                status,
                _status_padding: 0,
                internal_error_id,
            }
        }
    }
}

fn handle_result(result: Result<u64, PageletError>) -> FfiHandleResult {
    match result {
        Ok(handle) => FfiHandleResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            handle,
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiHandleResult {
                status,
                _status_padding: 0,
                internal_error_id,
                handle: 0,
            }
        }
    }
}

fn buffer_result(result: Result<NativeBuffer, PageletError>) -> FfiBufferResult {
    match result {
        Ok(buffer) => FfiBufferResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            buffer,
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiBufferResult {
                status,
                _status_padding: 0,
                internal_error_id,
                buffer: NativeBuffer::EMPTY,
            }
        }
    }
}

fn layout_result(
    result: Result<(FfiLayoutState, u64, NativeBuffer), PageletError>,
) -> FfiLayoutResult {
    match result {
        Ok((state, request, buffer)) => FfiLayoutResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            state,
            _state_padding: 0,
            request,
            buffer,
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiLayoutResult {
                status,
                _status_padding: 0,
                internal_error_id,
                state: FfiLayoutState::Complete,
                _state_padding: 0,
                request: 0,
                buffer: NativeBuffer::EMPTY,
            }
        }
    }
}

fn resource_result(
    result: Result<(u32, NativeBuffer, NativeBuffer, NativeBuffer), PageletError>,
) -> FfiResourceResult {
    match result {
        Ok((resource_id, bytes, path, media_type)) => FfiResourceResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            resource_id,
            _resource_padding: 0,
            bytes,
            path,
            media_type,
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiResourceResult {
                status,
                _status_padding: 0,
                internal_error_id,
                resource_id: 0,
                _resource_padding: 0,
                bytes: NativeBuffer::EMPTY,
                path: NativeBuffer::EMPTY,
                media_type: NativeBuffer::EMPTY,
            }
        }
    }
}

fn hit_test_result(
    result: Result<Option<crate::layout::HitTestResult>, PageletError>,
) -> FfiHitTestResult {
    match result {
        Ok(Some(hit)) => FfiHitTestResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            found: 1,
            affinity: match hit.affinity {
                TextAffinity::Upstream => 0,
                TextAffinity::Downstream => 1,
            },
            _padding: [0; 6],
            node_id: hit.node_id.get(),
            utf8_byte_offset: hit.utf8_byte_offset,
            fragment_id: hit.fragment_id,
            _tail_padding: 0,
        },
        Ok(None) => FfiHitTestResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            found: 0,
            affinity: 0,
            _padding: [0; 6],
            node_id: 0,
            utf8_byte_offset: 0,
            fragment_id: 0,
            _tail_padding: 0,
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiHitTestResult {
                status,
                _status_padding: 0,
                internal_error_id,
                found: 0,
                affinity: 0,
                _padding: [0; 6],
                node_id: 0,
                utf8_byte_offset: 0,
                fragment_id: 0,
                _tail_padding: 0,
            }
        }
    }
}

fn anchor_result(result: Result<Option<u32>, PageletError>) -> FfiAnchorResult {
    match result {
        Ok(page) => FfiAnchorResult {
            status: FfiStatus::Ok,
            _status_padding: 0,
            internal_error_id: 0,
            found: u8::from(page.is_some()),
            _padding: [0; 3],
            page_index: page.unwrap_or_default(),
        },
        Err(error) => {
            let (status, internal_error_id) = error_status(&error);
            FfiAnchorResult {
                status,
                _status_padding: 0,
                internal_error_id,
                found: 0,
                _padding: [0; 3],
                page_index: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
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
                "pagelet-native-{}-{sequence}.epub",
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

    fn bytes(value: &[u8]) -> FfiByteSlice {
        FfiByteSlice {
            data: value.as_ptr(),
            len: value.len(),
        }
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn c_result_layouts_have_explicit_stable_padding() {
        use std::mem::{align_of, size_of};

        assert_eq!(size_of::<FfiStatusResult>(), 16);
        assert_eq!(size_of::<FfiHandleResult>(), 24);
        assert_eq!(size_of::<FfiBufferResult>(), 40);
        assert_eq!(size_of::<FfiLayoutResult>(), 56);
        assert_eq!(size_of::<FfiResourceResult>(), 96);
        assert_eq!(size_of::<FfiHitTestResult>(), 40);
        assert_eq!(size_of::<FfiAnchorResult>(), 24);
        assert_eq!(size_of::<FfiCopyResult>(), 32);
        assert_eq!(size_of::<NativeBuffer>(), 24);
        assert_eq!(size_of::<FfiLayoutOptions>(), 56);
        assert_eq!(align_of::<FfiLayoutResult>(), 8);
        assert_eq!(align_of::<FfiResourceResult>(), 8);
    }

    #[test]
    fn c_abi_round_trips_versioned_measurements_and_page_scenes() {
        let fixture = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);
        let temp = TempEpub::new(fixture.bytes());
        let path = temp.path.to_string_lossy();

        let engine = pagelet_engine_create();
        assert_eq!(engine.status, FfiStatus::Ok);
        // SAFETY: `path` remains alive and readable for this call.
        let book = unsafe { pagelet_book_open_path(engine.handle, bytes(path.as_bytes())) };
        assert_eq!(book.status, FfiStatus::Ok);

        let summary = pagelet_book_summary(book.handle);
        assert_eq!(summary.status, FfiStatus::Ok);
        let summary_bytes = buffer_pool().to_vec(summary.buffer).expect("summary bytes");
        assert!(str::from_utf8(&summary_bytes)
            .expect("summary utf8")
            .contains("\"spine\""));
        assert_eq!(pagelet_buffer_free(summary.buffer).status, FfiStatus::Ok);
        assert_eq!(pagelet_buffer_free(summary.buffer).status, FfiStatus::Ok);

        let navigation = pagelet_book_navigation(book.handle);
        assert_eq!(navigation.status, FfiStatus::Ok);
        let navigation_bytes = buffer_pool()
            .to_vec(navigation.buffer)
            .expect("navigation bytes");
        assert!(str::from_utf8(&navigation_bytes)
            .expect("navigation utf8")
            .contains("\"toc\""));
        pagelet_buffer_free(navigation.buffer);

        let resource = pagelet_resource_read(book.handle, 0);
        assert_eq!(resource.status, FfiStatus::Ok);
        assert!(!buffer_pool()
            .to_vec(resource.bytes)
            .expect("resource bytes")
            .is_empty());
        pagelet_buffer_free(resource.bytes);
        pagelet_buffer_free(resource.path);
        pagelet_buffer_free(resource.media_type);

        let chapter = pagelet_chapter_open(book.handle, 0);
        assert_eq!(chapter.status, FfiStatus::Ok);
        let layout = pagelet_layout_session_create(chapter.handle, FfiLayoutOptions::default());
        assert_eq!(layout.status, FfiStatus::Ok);
        let requested = pagelet_layout_request(layout.handle, FfiPageRequest::default());
        assert_eq!(requested.status, FfiStatus::Ok);
        assert_eq!(requested.state, FfiLayoutState::NeedMeasurements);
        let wire_request = buffer_pool()
            .to_vec(requested.buffer)
            .expect("measure buffer");
        let measure_batch = WireMeasureBatch::decode(&wire_request)
            .expect("decode measure batch")
            .into_text_batch();
        pagelet_buffer_free(requested.buffer);

        let backend = DefaultTextBackend::default();
        let measured = backend
            .measure_batch(&measure_batch, &crate::core::CancellationToken::new())
            .expect("measure");
        let measured = WireMeasuredBatch::from(measured)
            .encode()
            .expect("encode measured batch");
        // SAFETY: `measured` remains alive and readable for this call.
        let submitted =
            unsafe { pagelet_layout_submit_measurements(requested.request, bytes(&measured)) };
        assert_eq!(submitted.status, FfiStatus::Ok);
        assert_eq!(submitted.state, FfiLayoutState::Pages);
        let page_bytes = buffer_pool().to_vec(submitted.buffer).expect("page buffer");
        let pages = PageBatch::decode(&page_bytes).expect("decode page batch");
        let page = pages.pages.first().expect("page");
        let anchor = page.start_anchor.expect("page anchor");
        let resolved = pagelet_anchor_to_page(
            layout.handle,
            anchor.document_id.get(),
            anchor.node_id.get(),
            anchor.utf8_byte_offset,
            match anchor.affinity {
                TextAffinity::Upstream => 0,
                TextAffinity::Downstream => 1,
            },
        );
        assert_eq!(resolved.status, FfiStatus::Ok);
        assert_eq!(resolved.found, 1);
        assert_eq!(resolved.page_index, page.page_index);

        let mut copied = vec![0_u8; submitted.buffer.len];
        // SAFETY: `copied` remains writable for its declared length.
        let copied_result = unsafe {
            pagelet_buffer_copy(
                submitted.buffer,
                FfiMutableByteSlice {
                    data: copied.as_mut_ptr(),
                    len: copied.len(),
                },
            )
        };
        assert_eq!(copied_result.status, FfiStatus::Ok);
        assert_eq!(copied, page_bytes);
        pagelet_buffer_free(submitted.buffer);
        assert_eq!(pagelet_handle_dispose(engine.handle).status, FfiStatus::Ok);
    }

    #[test]
    fn c_abi_rejects_null_non_empty_slices_and_small_copy_targets() {
        let engine = pagelet_engine_create();
        // SAFETY: this deliberately-invalid descriptor is accepted by the
        // function contract and must be rejected before dereference.
        let opened = unsafe {
            pagelet_book_open_path(
                engine.handle,
                FfiByteSlice {
                    data: std::ptr::null(),
                    len: 1,
                },
            )
        };
        assert_eq!(opened.status, FfiStatus::Protocol);

        let buffer = buffer_pool().allocate(vec![1, 2, 3]).expect("buffer");
        let mut destination = [0_u8; 2];
        // SAFETY: `destination` is writable for its declared length.
        let copied = unsafe {
            pagelet_buffer_copy(
                buffer,
                FfiMutableByteSlice {
                    data: destination.as_mut_ptr(),
                    len: destination.len(),
                },
            )
        };
        assert_eq!(copied.status, FfiStatus::BufferTooSmall);
        assert_eq!(copied.required, 3);
        pagelet_buffer_free(buffer);
        pagelet_handle_dispose(engine.handle);
    }
}
