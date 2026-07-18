//! EPUB container, package, navigation, diagnostics, and inspect support.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use miniz_oxide::inflate::{decompress_to_vec_with_limit, TINFLStatus};

pub use crate::core::ResourceId;
use crate::core::{
    ContentHash, Diagnostic, DiagnosticCode, DocumentId, LayoutUnit, NodeId, PackageError,
    PageletError, ParseError, ResourceLimitError, ResourceLimitKind, ResourceLimits, Severity,
    SourceRange, StyleId, UnsupportedFeature,
};
use crate::document;

const IMAGE_HEADER_PREFIX_BYTES: usize = 64 * 1024;

/// EPUB compatibility mode used while opening a book.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum CompatibilityMode {
    /// Reject structural deviations immediately.
    Strict,
    /// Accept common deterministic recoveries while reporting diagnostics.
    #[default]
    Compatible,
    /// Preserve diagnostics and extract whatever metadata/navigation is possible.
    Salvage,
}

/// Options for opening an EPUB package.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct OpenOptions {
    pub compatibility_mode: CompatibilityMode,
    pub limits: ResourceLimits,
}

impl OpenOptions {
    /// Create options using compatible mode and mobile resource defaults.
    #[must_use]
    pub const fn compatible() -> Self {
        Self {
            compatibility_mode: CompatibilityMode::Compatible,
            limits: ResourceLimits::mobile_defaults(),
        }
    }

    /// Create strict options.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            compatibility_mode: CompatibilityMode::Strict,
            limits: ResourceLimits::mobile_defaults(),
        }
    }

    /// Create salvage options.
    #[must_use]
    pub const fn salvage() -> Self {
        Self {
            compatibility_mode: CompatibilityMode::Salvage,
            limits: ResourceLimits::mobile_defaults(),
        }
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::compatible()
    }
}

/// Media type stored in an EPUB resource index.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MediaType(pub Arc<str>);

impl MediaType {
    /// Create a media type.
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// Return the raw media type.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Indexed resource metadata.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResourceMetadata {
    pub id: ResourceId,
    pub path: Arc<str>,
    pub media_type: MediaType,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub compression_method: u16,
}

/// Lazily read resource bytes.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResourceBytes {
    pub id: ResourceId,
    pub path: Arc<str>,
    pub media_type: MediaType,
    pub bytes: Vec<u8>,
}

/// Store abstraction over an EPUB publication container.
pub trait PublicationStore: Send + Sync {
    fn metadata(&self, resource_id: ResourceId) -> Option<&ResourceMetadata>;
    fn metadata_by_path(&self, path: &str) -> Option<&ResourceMetadata>;
    fn read(&self, resource_id: ResourceId) -> Result<ResourceBytes, PageletError>;
    fn open_stream(
        &self,
        resource_id: ResourceId,
    ) -> Result<Box<dyn Read + Send + 'static>, PageletError>;
}

/// Resource read counters for lazy-store assertions and smoke reporting.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct StoreStats {
    pub read_count: u64,
    pub decompressed_bytes: u64,
    pub copied_bytes: u64,
}

/// ZIP-backed EPUB publication store.
#[derive(Debug)]
pub struct ZipPublicationStore {
    bytes: Arc<[u8]>,
    entries: Vec<ZipEntry>,
    metadata: Vec<ResourceMetadata>,
    exact_paths: BTreeMap<Arc<str>, ResourceId>,
    lowercase_paths: BTreeMap<String, Option<ResourceId>>,
    reads: AtomicU64,
    decompressed_bytes: AtomicU64,
    copied_bytes: AtomicU64,
}

impl ZipPublicationStore {
    /// Index an EPUB ZIP without reading all resources.
    pub fn from_bytes(
        bytes: impl Into<Vec<u8>>,
        options: OpenOptions,
    ) -> Result<Self, PageletError> {
        let bytes: Arc<[u8]> = Arc::from(bytes.into().into_boxed_slice());
        let entries = index_zip_entries(&bytes, options)?;
        let entry_count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        if entry_count > u64::from(options.limits.max_zip_entries) {
            return Err(limit_error(
                ResourceLimitKind::ZipEntries,
                u64::from(options.limits.max_zip_entries),
                entry_count,
            ));
        }

        let mut metadata = Vec::with_capacity(entries.len());
        let mut exact_paths = BTreeMap::new();
        let mut lowercase_paths = BTreeMap::<String, Option<ResourceId>>::new();
        let mut total_uncompressed = 0_u64;

        for (index, entry) in entries.iter().enumerate() {
            validate_container_path(&entry.path, options.compatibility_mode)?;
            total_uncompressed = total_uncompressed.saturating_add(entry.uncompressed_size);
            if entry.uncompressed_size > options.limits.max_single_resource_bytes {
                return Err(limit_error(
                    ResourceLimitKind::SingleResourceBytes,
                    options.limits.max_single_resource_bytes,
                    entry.uncompressed_size,
                ));
            }
            if total_uncompressed > options.limits.max_total_decompressed_bytes {
                return Err(limit_error(
                    ResourceLimitKind::TotalDecompressedBytes,
                    options.limits.max_total_decompressed_bytes,
                    total_uncompressed,
                ));
            }
            let zero_compressed_ratio = if entry.uncompressed_size == 0 {
                0
            } else {
                u64::MAX
            };
            let ratio = entry
                .uncompressed_size
                .saturating_add(entry.compressed_size.saturating_sub(1))
                .checked_div(entry.compressed_size)
                .unwrap_or(zero_compressed_ratio);
            if ratio > u64::from(options.limits.max_compression_ratio) {
                return Err(limit_error(
                    ResourceLimitKind::CompressionRatio,
                    u64::from(options.limits.max_compression_ratio),
                    ratio,
                ));
            }

            let id = ResourceId::new(u32::try_from(index).map_err(|_| {
                PageletError::InvalidContainer(crate::core::ContainerError::new(
                    "too many resources to assign ResourceId",
                ))
            })?);
            let path: Arc<str> = Arc::from(entry.path.clone());
            if exact_paths.insert(path.clone(), id).is_some() {
                return Err(invalid_container(format!(
                    "duplicate ZIP entry path: {}",
                    entry.path
                )));
            }
            let lower = entry.path.to_ascii_lowercase();
            lowercase_paths
                .entry(lower)
                .and_modify(|slot| *slot = None)
                .or_insert(Some(id));

            metadata.push(ResourceMetadata {
                id,
                path,
                media_type: MediaType::new(media_type_for_path(&entry.path)),
                compressed_size: entry.compressed_size,
                uncompressed_size: entry.uncompressed_size,
                compression_method: entry.compression_method,
            });
        }

        let store = Self {
            bytes,
            entries,
            metadata,
            exact_paths,
            lowercase_paths,
            reads: AtomicU64::new(0),
            decompressed_bytes: AtomicU64::new(0),
            copied_bytes: AtomicU64::new(0),
        };
        store.validate_mimetype()?;
        Ok(store)
    }

    /// Return all indexed resource metadata.
    #[must_use]
    pub fn resources(&self) -> &[ResourceMetadata] {
        &self.metadata
    }

    /// Resolve an exact path, or a unique case-insensitive fallback in compatible modes.
    #[must_use]
    pub fn resolve_path(&self, path: &str, mode: CompatibilityMode) -> Option<ResourceId> {
        self.exact_paths.get(path).copied().or_else(|| {
            if mode == CompatibilityMode::Strict {
                None
            } else {
                self.lowercase_paths
                    .get(&path.to_ascii_lowercase())
                    .and_then(|slot| *slot)
            }
        })
    }

    /// Read a resource by path.
    pub fn read_path(
        &self,
        path: &str,
        mode: CompatibilityMode,
    ) -> Result<ResourceBytes, PageletError> {
        let id = self
            .resolve_path(path, mode)
            .ok_or_else(|| invalid_package(format!("resource not found: {path}")))?;
        self.read(id)
    }

    /// Read up to `max_uncompressed_bytes` from a resource by path.
    pub fn read_path_prefix(
        &self,
        path: &str,
        max_uncompressed_bytes: usize,
        mode: CompatibilityMode,
    ) -> Result<ResourceBytes, PageletError> {
        let id = self
            .resolve_path(path, mode)
            .ok_or_else(|| invalid_package(format!("resource not found: {path}")))?;
        self.read_prefix(id, max_uncompressed_bytes)
    }

    /// Read up to `max_uncompressed_bytes` from a resource.
    pub fn read_prefix(
        &self,
        resource_id: ResourceId,
        max_uncompressed_bytes: usize,
    ) -> Result<ResourceBytes, PageletError> {
        self.read_with_limit(resource_id, Some(max_uncompressed_bytes))
    }

    /// Return lazy read counters.
    #[must_use]
    pub fn stats(&self) -> StoreStats {
        StoreStats {
            read_count: self.reads.load(Ordering::Relaxed),
            decompressed_bytes: self.decompressed_bytes.load(Ordering::Relaxed),
            copied_bytes: self.copied_bytes.load(Ordering::Relaxed),
        }
    }

    fn validate_mimetype(&self) -> Result<(), PageletError> {
        let Some(resource_id) = self.resolve_path("mimetype", CompatibilityMode::Strict) else {
            return Err(invalid_container("missing mimetype entry"));
        };
        let mimetype = self.read(resource_id)?;
        if mimetype.bytes != b"application/epub+zip" {
            return Err(invalid_container("invalid EPUB mimetype entry"));
        }
        Ok(())
    }
}

impl PublicationStore for ZipPublicationStore {
    fn metadata(&self, resource_id: ResourceId) -> Option<&ResourceMetadata> {
        self.metadata.get(usize::try_from(resource_id.get()).ok()?)
    }

    fn metadata_by_path(&self, path: &str) -> Option<&ResourceMetadata> {
        let id = self.resolve_path(path, CompatibilityMode::Compatible)?;
        self.metadata(id)
    }

    fn read(&self, resource_id: ResourceId) -> Result<ResourceBytes, PageletError> {
        self.read_with_limit(resource_id, None)
    }

    fn open_stream(
        &self,
        resource_id: ResourceId,
    ) -> Result<Box<dyn Read + Send + 'static>, PageletError> {
        Ok(Box::new(Cursor::new(self.read(resource_id)?.bytes)))
    }
}

impl ZipPublicationStore {
    fn read_with_limit(
        &self,
        resource_id: ResourceId,
        max_uncompressed_bytes: Option<usize>,
    ) -> Result<ResourceBytes, PageletError> {
        let index = usize::try_from(resource_id.get()).map_err(|_| {
            PageletError::InvalidContainer(crate::core::ContainerError::new("invalid resource id"))
        })?;
        let entry = self.entries.get(index).ok_or_else(|| {
            PageletError::InvalidContainer(crate::core::ContainerError::new("unknown resource id"))
        })?;
        let start = entry.data_offset;
        let end = start
            .checked_add(usize::try_from(entry.compressed_size).unwrap_or(usize::MAX))
            .ok_or_else(|| invalid_container("ZIP entry range overflows"))?;
        let compressed = self
            .bytes
            .get(start..end)
            .ok_or_else(|| invalid_container("ZIP entry range is outside archive"))?;
        let bytes = match entry.compression_method {
            0 => {
                let len = max_uncompressed_bytes
                    .map(|limit| limit.min(compressed.len()))
                    .unwrap_or(compressed.len());
                compressed[..len].to_vec()
            }
            8 => inflate_zip_entry(compressed, entry, max_uncompressed_bytes)?,
            method => {
                return Err(PageletError::UnsupportedFeature(UnsupportedFeature::new(
                    format!("ZIP compression method {method} is not supported"),
                )));
            }
        };
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.decompressed_bytes.fetch_add(
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.copied_bytes.fetch_add(
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        Ok(ResourceBytes {
            id: resource_id,
            path: self
                .metadata(resource_id)
                .map(|metadata| metadata.path.clone())
                .unwrap_or_else(|| Arc::from(entry.path.as_str())),
            media_type: self
                .metadata(resource_id)
                .map(|metadata| metadata.media_type.clone())
                .unwrap_or_else(|| MediaType::new("application/octet-stream")),
            bytes,
        })
    }
}

fn inflate_zip_entry(
    compressed: &[u8],
    entry: &ZipEntry,
    max_uncompressed_bytes: Option<usize>,
) -> Result<Vec<u8>, PageletError> {
    let limit = max_uncompressed_bytes
        .unwrap_or_else(|| usize::try_from(entry.uncompressed_size).unwrap_or(usize::MAX));
    let bytes = match decompress_to_vec_with_limit(compressed, limit) {
        Ok(bytes) => bytes,
        Err(error)
            if max_uncompressed_bytes.is_some()
                && error.status == TINFLStatus::HasMoreOutput
                && !error.output.is_empty() =>
        {
            error.output
        }
        Err(error) => {
            return Err(PageletError::InvalidContainer(
                crate::core::ContainerError::new(format!(
                    "deflate decode failed for {}: {}",
                    entry.path, error
                )),
            ));
        }
    };
    if max_uncompressed_bytes.is_none() && bytes.len() != entry.uncompressed_size as usize {
        return Err(invalid_container(format!(
            "deflated entry size mismatch for {}",
            entry.path
        )));
    }
    Ok(bytes)
}

/// Parsed rootfile entry from `META-INF/container.xml`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Rootfile {
    pub full_path: String,
    pub media_type: String,
}

/// OPF package metadata.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct BookMetadata {
    pub package_version: String,
    pub unique_identifier: String,
    pub identifier: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub cover_image: Option<String>,
}

/// OPF manifest item.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ManifestItem {
    pub id: String,
    pub href: String,
    pub resolved_path: String,
    pub media_type: String,
    pub properties: Vec<String>,
    pub fallback: Option<String>,
    pub media_overlay: Option<String>,
}

/// OPF spine item.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SpineItem {
    pub idref: String,
    pub linear: bool,
    pub properties: Vec<String>,
}

/// Parsed OPF package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PackageDocument {
    pub rootfile_path: String,
    pub metadata: BookMetadata,
    pub manifest: Vec<ManifestItem>,
    pub spine: Vec<SpineItem>,
    pub spine_toc: Option<String>,
    pub page_progression_direction: Option<String>,
    pub guide: Vec<NavigationItem>,
}

/// Navigation source selected for a book.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum NavigationSource {
    Epub3Nav,
    Ncx,
    Guide,
    #[default]
    Spine,
}

/// Navigation model.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Navigation {
    pub source: NavigationSource,
    pub toc: Vec<NavigationItem>,
    pub page_list: Vec<NavigationItem>,
    pub landmarks: Vec<NavigationItem>,
}

/// One navigation node.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct NavigationItem {
    pub label: String,
    pub href: String,
    pub children: Vec<NavigationItem>,
}

/// Capability support status.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum CapabilityStatus {
    Supported,
    SupportedWithLimitations,
    UnsupportedDiagnosed,
}

/// One capability item.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Capability {
    pub feature: String,
    pub status: CapabilityStatus,
    pub message: String,
}

/// Compatibility and feature report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CapabilityReport {
    pub mode: CompatibilityMode,
    pub capabilities: Vec<Capability>,
}

impl CapabilityReport {
    fn new(mode: CompatibilityMode) -> Self {
        let mut report = Self {
            mode,
            capabilities: Vec::new(),
        };
        report.push(
            "zip/stored",
            CapabilityStatus::Supported,
            "stored ZIP entries can be read lazily",
        );
        report.push(
            "zip/deflate",
            CapabilityStatus::SupportedWithLimitations,
            "deflate entries are inflated with resource limits before use",
        );
        report.push(
            "epub/package",
            CapabilityStatus::SupportedWithLimitations,
            "metadata, manifest, spine, nav, NCX and guide are parsed for M1",
        );
        report.push(
            "epub/xhtml-chapter-ir",
            CapabilityStatus::SupportedWithLimitations,
            "spine XHTML can be tokenized and mapped to ChapterIR for M2-supported nodes",
        );
        report
    }

    fn push(&mut self, feature: &str, status: CapabilityStatus, message: &str) {
        self.capabilities.push(Capability {
            feature: feature.into(),
            status,
            message: message.into(),
        });
    }
}

/// Book summary produced by `open_book`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookSummary {
    pub rootfiles: Vec<Rootfile>,
    pub package: PackageDocument,
    pub navigation: Navigation,
    pub diagnostics: Vec<Diagnostic>,
    pub capability_report: CapabilityReport,
    pub resources: Vec<ResourceMetadata>,
    pub store_stats: StoreStats,
}

#[derive(Debug)]
pub(crate) struct OpenedBook {
    pub(crate) summary: BookSummary,
    store: ZipPublicationStore,
}

/// Open an EPUB and parse metadata/navigation with compatible defaults.
pub fn open_book(bytes: impl Into<Vec<u8>>) -> Result<BookSummary, PageletError> {
    open_book_with_options(bytes, OpenOptions::default())
}

/// Open an EPUB and parse metadata/navigation using explicit options.
pub fn open_book_with_options(
    bytes: impl Into<Vec<u8>>,
    options: OpenOptions,
) -> Result<BookSummary, PageletError> {
    Ok(open_book_context(bytes, options)?.summary)
}

/// Open an EPUB and return package-level [`document::BookIr`].
pub fn open_book_ir(bytes: impl Into<Vec<u8>>) -> Result<document::BookIr, PageletError> {
    open_book_ir_with_options(bytes, OpenOptions::default())
}

/// Open an EPUB and return package-level [`document::BookIr`] with explicit options.
pub fn open_book_ir_with_options(
    bytes: impl Into<Vec<u8>>,
    options: OpenOptions,
) -> Result<document::BookIr, PageletError> {
    let opened = open_book_context(bytes, options)?;
    book_ir_from_opened(&opened, options)
}

/// Parse the first readable spine item into ChapterIR.
pub fn open_first_chapter_ir(
    bytes: impl Into<Vec<u8>>,
) -> Result<document::ChapterIr, PageletError> {
    open_spine_item_chapter_ir_with_options(bytes, 0, OpenOptions::default())
}

/// Parse one spine item into ChapterIR.
pub fn open_spine_item_chapter_ir(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
) -> Result<document::ChapterIr, PageletError> {
    open_spine_item_chapter_ir_with_options(bytes, spine_index, OpenOptions::default())
}

/// Parse one spine item into ChapterIR with explicit options.
pub fn open_spine_item_chapter_ir_with_options(
    bytes: impl Into<Vec<u8>>,
    spine_index: usize,
    options: OpenOptions,
) -> Result<document::ChapterIr, PageletError> {
    let opened = open_book_context(bytes, options)?;
    let book_ir = book_ir_from_opened(&opened, options)?;
    let manifest_item = spine_manifest_item(&opened.summary.package, spine_index)?;
    let bytes = opened
        .store
        .read_path(&manifest_item.resolved_path, options.compatibility_mode)?;
    let xhtml = resource_text(&bytes)?;
    let title = text_of_first(&xhtml, &["title"]).unwrap_or_else(|| {
        navigation_title_for_href(&opened.summary.navigation, &manifest_item.resolved_path)
            .unwrap_or_else(|| manifest_item.id.clone())
    });
    chapter_ir_from_xhtml(
        DocumentId::new(u32::try_from(spine_index).unwrap_or(u32::MAX)),
        &manifest_item.resolved_path,
        &title,
        &xhtml,
        ChapterResourceContext {
            resources: &book_ir.resources,
            cover_image: book_ir.metadata.cover_image.as_deref(),
            store: Some(&opened.store),
            options,
        },
    )
}

/// Read one resource by typed id with compatible defaults.
pub fn read_resource_bytes(
    bytes: impl Into<Vec<u8>>,
    resource_id: ResourceId,
) -> Result<ResourceBytes, PageletError> {
    read_resource_bytes_with_options(bytes, resource_id, OpenOptions::default())
}

/// Read one resource by typed id using explicit open options.
pub fn read_resource_bytes_with_options(
    bytes: impl Into<Vec<u8>>,
    resource_id: ResourceId,
    options: OpenOptions,
) -> Result<ResourceBytes, PageletError> {
    let store = ZipPublicationStore::from_bytes(bytes, options)?;
    store.read(resource_id)
}

fn open_book_context(
    bytes: impl Into<Vec<u8>>,
    options: OpenOptions,
) -> Result<OpenedBook, PageletError> {
    let mut diagnostics = DiagnosticCollector::new(options.limits.max_diagnostics);
    let capability_report = CapabilityReport::new(options.compatibility_mode);
    let store = ZipPublicationStore::from_bytes(bytes, options)?;
    let rootfiles = match parse_container(&store, options, &mut diagnostics) {
        Ok(rootfiles) => rootfiles,
        Err(error) if options.compatibility_mode == CompatibilityMode::Salvage => {
            diagnostics.push_error(error.code(), error.to_string())?;
            Vec::new()
        }
        Err(error) => return Err(error),
    };
    let rootfile = choose_rootfile(&rootfiles, options.compatibility_mode)?;
    let package_bytes = store.read_path(&rootfile.full_path, options.compatibility_mode)?;
    let package_text = resource_text(&package_bytes)?;
    let package = parse_opf(&package_text, &rootfile.full_path)?;
    let navigation = parse_navigation(&store, options, &package, &mut diagnostics)?;
    let resources = store.resources().to_vec();
    let store_stats = store.stats();

    Ok(OpenedBook {
        summary: BookSummary {
            rootfiles,
            package,
            navigation,
            diagnostics: diagnostics.into_vec(),
            capability_report,
            resources,
            store_stats,
        },
        store,
    })
}

pub(crate) fn open_book_session_context(
    bytes: impl Into<Vec<u8>>,
    options: OpenOptions,
) -> Result<OpenedBook, PageletError> {
    open_book_context(bytes, options)
}

pub(crate) fn open_spine_item_from_context(
    opened: &OpenedBook,
    spine_index: usize,
    options: OpenOptions,
) -> Result<document::ChapterIr, PageletError> {
    let book_ir = book_ir_from_opened(opened, options)?;
    let manifest_item = spine_manifest_item(&opened.summary.package, spine_index)?;
    let bytes = opened
        .store
        .read_path(&manifest_item.resolved_path, options.compatibility_mode)?;
    let xhtml = resource_text(&bytes)?;
    let title = text_of_first(&xhtml, &["title"]).unwrap_or_else(|| {
        navigation_title_for_href(&opened.summary.navigation, &manifest_item.resolved_path)
            .unwrap_or_else(|| manifest_item.id.clone())
    });
    chapter_ir_from_xhtml(
        DocumentId::new(u32::try_from(spine_index).unwrap_or(u32::MAX)),
        &manifest_item.resolved_path,
        &title,
        &xhtml,
        ChapterResourceContext {
            resources: &book_ir.resources,
            cover_image: book_ir.metadata.cover_image.as_deref(),
            store: Some(&opened.store),
            options,
        },
    )
}

fn book_ir_from_summary(book: &BookSummary) -> document::BookIr {
    let mut resources = document::ResourceTable::new();
    for resource in &book.resources {
        resources.push(document::ResourceInfo {
            id: resource.id,
            path: resource.path.clone(),
            media_type: resource.media_type.0.clone(),
            kind: document::ResourceKind::from_media_type(resource.media_type.as_str()),
            compressed_size: resource.compressed_size,
            uncompressed_size: resource.uncompressed_size,
            compression_method: resource.compression_method,
        });
    }

    let manifest = book
        .package
        .manifest
        .iter()
        .map(|item| document::ManifestItem {
            id: Arc::from(item.id.as_str()),
            href: Arc::from(item.href.as_str()),
            resolved_path: Arc::from(item.resolved_path.as_str()),
            media_type: Arc::from(item.media_type.as_str()),
            properties: item
                .properties
                .iter()
                .map(|property| Arc::from(property.as_str()))
                .collect(),
            fallback: item.fallback.as_deref().map(Arc::from),
            media_overlay: item.media_overlay.as_deref().map(Arc::from),
            resource_id: resources.id_for_path(&item.resolved_path),
        })
        .collect::<Vec<_>>();

    let spine = book
        .package
        .spine
        .iter()
        .map(|item| {
            let manifest_index = book
                .package
                .manifest
                .iter()
                .position(|manifest_item| manifest_item.id == item.idref)
                .and_then(|index| u32::try_from(index).ok());
            let href = manifest_index
                .and_then(|index| book.package.manifest.get(usize::try_from(index).ok()?))
                .map(|manifest_item| Arc::from(manifest_item.resolved_path.as_str()));
            document::SpineItem {
                idref: Arc::from(item.idref.as_str()),
                linear: item.linear,
                properties: item
                    .properties
                    .iter()
                    .map(|property| Arc::from(property.as_str()))
                    .collect(),
                manifest_index,
                href,
            }
        })
        .collect();

    document::BookIr {
        metadata: document::BookMetadata {
            package_version: Arc::from(book.package.metadata.package_version.as_str()),
            unique_identifier: Arc::from(book.package.metadata.unique_identifier.as_str()),
            identifier: book.package.metadata.identifier.as_deref().map(Arc::from),
            title: book.package.metadata.title.as_deref().map(Arc::from),
            language: book.package.metadata.language.as_deref().map(Arc::from),
            cover_image: book.package.metadata.cover_image.as_deref().map(Arc::from),
        },
        package: document::PackageInfo {
            rootfile_path: Arc::from(book.package.rootfile_path.as_str()),
            version: Arc::from(book.package.metadata.package_version.as_str()),
            spine_toc: book.package.spine_toc.as_deref().map(Arc::from),
            page_progression_direction: book
                .package
                .page_progression_direction
                .as_deref()
                .map(Arc::from),
        },
        manifest,
        spine,
        navigation: navigation_ir(&book.navigation),
        resources,
        capabilities: capability_report_ir(&book.capability_report),
    }
}

fn book_ir_from_opened(
    opened: &OpenedBook,
    options: OpenOptions,
) -> Result<document::BookIr, PageletError> {
    let mut ir = book_ir_from_summary(&opened.summary);
    enrich_resource_table(&mut ir.resources, &opened.store, options)?;
    Ok(ir)
}

fn enrich_resource_table(
    resources: &mut document::ResourceTable,
    store: &ZipPublicationStore,
    options: OpenOptions,
) -> Result<(), PageletError> {
    let indexed = resources.resources.clone();
    for resource in indexed {
        match resource.kind {
            document::ResourceKind::Image => {
                let header = store.read_prefix(resource.id, IMAGE_HEADER_PREFIX_BYTES)?;
                let size = parse_image_header(&header.bytes, &resource.media_type);
                resources.set_image_size(resource.id, size);
            }
            document::ResourceKind::Font => {
                let header = store.read_prefix(resource.id, IMAGE_HEADER_PREFIX_BYTES)?;
                let mut fingerprint_bytes = Vec::new();
                fingerprint_bytes.extend_from_slice(resource.path.as_bytes());
                fingerprint_bytes.extend_from_slice(resource.media_type.as_bytes());
                fingerprint_bytes.extend_from_slice(&resource.uncompressed_size.to_le_bytes());
                fingerprint_bytes.extend_from_slice(&header.bytes);
                resources
                    .set_font_fingerprint(resource.id, ContentHash::from_bytes(&fingerprint_bytes));
            }
            _ => {}
        }
    }
    let _ = options;
    Ok(())
}

/// Heuristic to detect likely noise chapters (copyright, TOC, ads, boilerplate).
///
/// Returns true when the title or content strongly indicates the chapter is
/// not part of the main book content.
#[must_use]
pub fn is_likely_noise_chapter(
    title: &str,
    visible_text: &str,
    spine_index: usize,
    spine_len: usize,
) -> bool {
    let lower_title = title.to_ascii_lowercase();
    let lower_text = visible_text.to_ascii_lowercase();

    let noise_title_keywords = [
        "copyright",
        "legal",
        "imprint",
        "colophon",
        "acknowledgments",
        "acknowledgements",
        "credits",
        "license",
        "trademark",
        "cataloging",
        "publication data",
        "verso",
        "title page",
        "half-title",
        "bastard title",
        "also by",
        "other books",
        "by the same author",
        "front matter",
        "epigraph",
        "dedication",
        "praise for",
    ];
    if noise_title_keywords
        .iter()
        .any(|kw| lower_title.contains(kw))
    {
        return true;
    }

    let noise_text_patterns = [
        "copyright ©",
        "all rights reserved",
        "isbn",
        "library of congress",
        "cataloging-in-publication",
        "printed in",
    ];
    let short_text = visible_text.len() < 300;
    if short_text
        && noise_text_patterns
            .iter()
            .any(|pat| lower_text.contains(pat))
    {
        return true;
    }

    if spine_index == 0
        && visible_text.len() < 200
        && (lower_text.contains("copyright") || lower_text.contains("title page"))
    {
        return true;
    }

    let is_end_matter = spine_index >= spine_len.saturating_sub(2);
    if is_end_matter
        && visible_text.len() < 500
        && noise_text_patterns
            .iter()
            .any(|pat| lower_text.contains(pat))
    {
        return true;
    }

    if !lower_text.contains(' ') && visible_text.len() > 100 {
        return true;
    }

    if visible_text.len() < 30 && !lower_text.chars().any(|c| c.is_alphanumeric()) {
        return true;
    }

    false
}

/// Return spine indices that pass the noise filter, preserving original order.
#[must_use]
pub fn filter_noise_chapters(chapters: &[(usize, &str, &str)], spine_len: usize) -> Vec<usize> {
    chapters
        .iter()
        .filter(|(index, title, text)| !is_likely_noise_chapter(title, text, *index, spine_len))
        .map(|(index, ..)| *index)
        .collect()
}

/// Parse intrinsic dimensions from a bounded image header.
#[must_use]
pub fn parse_image_header(bytes: &[u8], media_type: &str) -> Option<document::ImageSize> {
    let lower = media_type.to_ascii_lowercase();
    if lower == "image/png" || bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return parse_png_size(bytes);
    }
    if lower == "image/gif" || bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return parse_gif_size(bytes);
    }
    if lower == "image/jpeg" || lower == "image/jpg" || bytes.starts_with(&[0xff, 0xd8]) {
        return parse_jpeg_size(bytes);
    }
    None
}

fn parse_png_size(bytes: &[u8]) -> Option<document::ImageSize> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    if bytes.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let width = read_be_u32(bytes, 16)?;
    let height = read_be_u32(bytes, 20)?;
    non_zero_image_size(width, height)
}

fn parse_gif_size(bytes: &[u8]) -> Option<document::ImageSize> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u32::from(read_le_u16(bytes, 6)?);
    let height = u32::from(read_le_u16(bytes, 8)?);
    non_zero_image_size(width, height)
}

fn parse_jpeg_size(bytes: &[u8]) -> Option<document::ImageSize> {
    if bytes.len() < 4 || bytes.get(0..2) != Some(&[0xff, 0xd8]) {
        return None;
    }

    let mut cursor = 2;
    while cursor + 4 <= bytes.len() {
        while bytes.get(cursor) == Some(&0xff) {
            cursor += 1;
        }
        let marker = *bytes.get(cursor)?;
        cursor += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let segment_len = usize::from(read_be_u16(bytes, cursor)?);
        if segment_len < 2 {
            return None;
        }
        let segment_end = cursor.checked_add(segment_len)?;
        if segment_end > bytes.len() {
            return None;
        }
        if is_jpeg_sof_marker(marker) {
            if cursor + 7 > bytes.len() {
                return None;
            }
            let height = u32::from(read_be_u16(bytes, cursor + 3)?);
            let width = u32::from(read_be_u16(bytes, cursor + 5)?);
            return non_zero_image_size(width, height);
        }
        cursor = segment_end;
    }
    None
}

fn is_jpeg_sof_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

fn non_zero_image_size(width: u32, height: u32) -> Option<document::ImageSize> {
    if width == 0 || height == 0 {
        None
    } else {
        Some(document::ImageSize { width, height })
    }
}

fn navigation_ir(navigation: &Navigation) -> document::Navigation {
    document::Navigation {
        source: Arc::from(navigation_source_label(navigation.source)),
        toc: navigation
            .toc
            .iter()
            .map(navigation_item_ir)
            .collect::<Vec<_>>(),
        page_list: navigation
            .page_list
            .iter()
            .map(navigation_item_ir)
            .collect::<Vec<_>>(),
        landmarks: navigation
            .landmarks
            .iter()
            .map(navigation_item_ir)
            .collect::<Vec<_>>(),
    }
}

fn navigation_item_ir(item: &NavigationItem) -> document::NavigationItem {
    document::NavigationItem {
        label: Arc::from(item.label.as_str()),
        href: Arc::from(item.href.as_str()),
        children: item.children.iter().map(navigation_item_ir).collect(),
    }
}

fn capability_report_ir(report: &CapabilityReport) -> document::CapabilityReport {
    document::CapabilityReport {
        mode: Arc::from(compatibility_mode_label(report.mode)),
        capabilities: report
            .capabilities
            .iter()
            .map(|capability| document::Capability {
                feature: Arc::from(capability.feature.as_str()),
                status: Arc::from(capability_status_label(capability.status)),
                message: Arc::from(capability.message.as_str()),
            })
            .collect(),
    }
}

fn spine_manifest_item(
    package: &PackageDocument,
    spine_index: usize,
) -> Result<&ManifestItem, PageletError> {
    let spine = package
        .spine
        .get(spine_index)
        .ok_or_else(|| invalid_package(format!("spine index out of range: {spine_index}")))?;
    package
        .manifest
        .iter()
        .find(|item| item.id == spine.idref)
        .ok_or_else(|| invalid_package(format!("spine idref not found: {}", spine.idref)))
}

fn navigation_title_for_href(navigation: &Navigation, href: &str) -> Option<String> {
    navigation
        .toc
        .iter()
        .find_map(|item| navigation_title_for_href_in_item(item, href))
}

fn navigation_title_for_href_in_item(item: &NavigationItem, href: &str) -> Option<String> {
    let item_href = href.split('#').next().unwrap_or(href);
    let nav_href = item.href.split('#').next().unwrap_or(&item.href);
    if item_href.ends_with(nav_href) || nav_href.ends_with(item_href) {
        return Some(item.label.clone());
    }
    item.children
        .iter()
        .find_map(|child| navigation_title_for_href_in_item(child, href))
}

const fn compatibility_mode_label(mode: CompatibilityMode) -> &'static str {
    match mode {
        CompatibilityMode::Strict => "strict",
        CompatibilityMode::Compatible => "compatible",
        CompatibilityMode::Salvage => "salvage",
    }
}

const fn capability_status_label(status: CapabilityStatus) -> &'static str {
    match status {
        CapabilityStatus::Supported => "supported",
        CapabilityStatus::SupportedWithLimitations => "supported-with-limitations",
        CapabilityStatus::UnsupportedDiagnosed => "unsupported-diagnosed",
    }
}

const fn navigation_source_label(source: NavigationSource) -> &'static str {
    match source {
        NavigationSource::Epub3Nav => "epub3-nav",
        NavigationSource::Ncx => "ncx",
        NavigationSource::Guide => "guide",
        NavigationSource::Spine => "spine",
    }
}

/// Parse `META-INF/container.xml`.
pub fn parse_container_xml(input: &str) -> Result<Vec<Rootfile>, PageletError> {
    let tags = scan_start_tags(input);
    let mut rootfiles = Vec::new();
    for tag in tags.iter().filter(|tag| tag.local_name() == "rootfile") {
        let full_path = tag.attr("full-path").unwrap_or_default();
        if full_path.is_empty() {
            return Err(invalid_container("rootfile full-path is empty"));
        }
        rootfiles.push(Rootfile {
            full_path: full_path.to_owned(),
            media_type: tag.attr("media-type").unwrap_or_default().to_owned(),
        });
    }
    if rootfiles.is_empty() {
        return Err(invalid_container("container.xml has no rootfile"));
    }
    Ok(rootfiles)
}

/// Parse OPF metadata, manifest, and spine.
pub fn parse_opf(input: &str, rootfile_path: &str) -> Result<PackageDocument, PageletError> {
    let package_tag = scan_start_tags(input)
        .into_iter()
        .find(|tag| tag.local_name() == "package")
        .ok_or_else(|| invalid_package("OPF package element is missing"))?;
    let package_version = package_tag.attr("version").unwrap_or_default().to_owned();
    let unique_identifier = package_tag
        .attr("unique-identifier")
        .unwrap_or_default()
        .to_owned();
    let title = text_of_first(input, &["dc:title", "title"]);
    let language = text_of_first(input, &["dc:language", "language"]);
    let identifier = identifier_for(input, &unique_identifier);
    let package_dir = parent_path(rootfile_path);

    let mut manifest = Vec::new();
    for tag in scan_start_tags(input)
        .into_iter()
        .filter(|tag| tag.local_name() == "item")
    {
        let id = tag.attr("id").unwrap_or_default().to_owned();
        let href = tag.attr("href").unwrap_or_default().to_owned();
        if id.is_empty() || href.is_empty() {
            return Err(invalid_package("manifest item requires id and href"));
        }
        let resolved_path = resolve_resource_path(&package_dir, &href)?;
        manifest.push(ManifestItem {
            id,
            href,
            resolved_path,
            media_type: tag.attr("media-type").unwrap_or_default().to_owned(),
            properties: split_properties(tag.attr("properties").unwrap_or_default()),
            fallback: tag.attr("fallback").map(ToOwned::to_owned),
            media_overlay: tag.attr("media-overlay").map(ToOwned::to_owned),
        });
    }

    let spine_tag = scan_start_tags(input)
        .into_iter()
        .find(|tag| tag.local_name() == "spine");
    let spine_toc = spine_tag
        .as_ref()
        .and_then(|tag| tag.attr("toc"))
        .map(ToOwned::to_owned);
    let page_progression_direction = spine_tag
        .as_ref()
        .and_then(|tag| tag.attr("page-progression-direction"))
        .map(ToOwned::to_owned);

    let mut spine = Vec::new();
    for tag in scan_start_tags(input)
        .into_iter()
        .filter(|tag| tag.local_name() == "itemref")
    {
        let idref = tag.attr("idref").unwrap_or_default().to_owned();
        if idref.is_empty() {
            return Err(invalid_package("spine itemref requires idref"));
        }
        let linear = tag
            .attr("linear")
            .map(|value| value != "no")
            .unwrap_or(true);
        spine.push(SpineItem {
            idref,
            linear,
            properties: split_properties(tag.attr("properties").unwrap_or_default()),
        });
    }

    let epub2_cover_id = scan_start_tags(input)
        .into_iter()
        .find(|tag| tag.local_name() == "meta" && tag.attr("name") == Some("cover"))
        .and_then(|tag| tag.attr("content").map(ToOwned::to_owned));
    let cover_image = manifest
        .iter()
        .find(|item| {
            item.properties
                .iter()
                .any(|property| property == "cover-image")
                || epub2_cover_id.as_deref() == Some(item.id.as_str())
        })
        .map(|item| item.resolved_path.clone());

    Ok(PackageDocument {
        rootfile_path: rootfile_path.to_owned(),
        metadata: BookMetadata {
            package_version,
            unique_identifier,
            identifier,
            title,
            language,
            cover_image,
        },
        manifest,
        spine,
        spine_toc,
        page_progression_direction,
        guide: parse_guide(input, &package_dir)?,
    })
}

/// Resolve an OPF-relative resource reference without leaving the container.
pub fn resolve_resource_path(base_dir: &str, href: &str) -> Result<String, PageletError> {
    let href = href.split('#').next().unwrap_or(href);
    let href = href.split('?').next().unwrap_or(href);
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("http:")
        || lower.starts_with("https:")
        || lower.starts_with("file:")
        || lower.starts_with("data:")
    {
        return Err(PageletError::UnsupportedFeature(UnsupportedFeature::new(
            "remote, file and data resource URLs are not resolved as container paths",
        )));
    }

    let mut parts = Vec::<&str>::new();
    for part in base_dir.split('/').filter(|part| !part.is_empty()) {
        parts.push(part);
    }
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(invalid_container("resource path escapes container root"));
                }
            }
            other if other.contains('\\') => {
                return Err(invalid_container("resource path contains backslash"));
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(invalid_container(
            "resource path resolves to container root",
        ));
    }
    Ok(parts.join("/"))
}

/// XHTML token with byte source range.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XhtmlToken {
    pub kind: XhtmlTokenKind,
    pub source_range: SourceRange,
}

/// Token kinds emitted by the lightweight XHTML tokenizer.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum XhtmlTokenKind {
    StartElement {
        name: String,
        attrs: BTreeMap<String, String>,
        self_closing: bool,
    },
    EndElement {
        name: String,
    },
    Text(String),
    Comment,
    ProcessingInstruction,
}

/// XHTML tree produced by [`parse_xhtml_tree`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XhtmlDocument {
    pub nodes: Vec<XhtmlNode>,
    pub root: usize,
}

impl XhtmlDocument {
    fn node(&self, id: usize) -> Option<&XhtmlNode> {
        self.nodes.get(id)
    }
}

/// One XHTML tree node.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XhtmlNode {
    pub kind: XhtmlNodeKind,
    pub source_range: SourceRange,
}

impl XhtmlNode {
    fn element(&self) -> Option<&XhtmlElement> {
        match &self.kind {
            XhtmlNodeKind::Element(element) => Some(element),
            XhtmlNodeKind::Text(_) => None,
        }
    }
}

/// XHTML tree node kind.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum XhtmlNodeKind {
    Element(XhtmlElement),
    Text(String),
}

/// XHTML element payload.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XhtmlElement {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<usize>,
}

impl XhtmlElement {
    fn local_name(&self) -> &str {
        local_name(&self.name)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
}

/// Tokenize XHTML with default resource limits.
pub fn tokenize_xhtml(input: &str) -> Result<Vec<XhtmlToken>, PageletError> {
    tokenize_xhtml_with_limits(input, ResourceLimits::default())
}

/// Tokenize XHTML with source range tracking and explicit limits.
pub fn tokenize_xhtml_with_limits(
    input: &str,
    limits: ResourceLimits,
) -> Result<Vec<XhtmlToken>, PageletError> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        if u64::try_from(tokens.len()).unwrap_or(u64::MAX) > u64::from(limits.max_dom_nodes) {
            return Err(limit_error(
                ResourceLimitKind::DomNodes,
                u64::from(limits.max_dom_nodes),
                u64::try_from(tokens.len()).unwrap_or(u64::MAX),
            ));
        }

        let Some(relative_start) = input[cursor..].find('<') else {
            push_text_token(input, cursor, input.len(), &mut tokens)?;
            break;
        };
        let start = cursor + relative_start;
        if start > cursor {
            push_text_token(input, cursor, start, &mut tokens)?;
        }

        if input[start..].starts_with("<!--") {
            let end = input[start + 4..]
                .find("-->")
                .map(|relative| start + 4 + relative + 3)
                .ok_or_else(|| {
                    PageletError::Parse(ParseError::new("unterminated XHTML comment"))
                })?;
            tokens.push(XhtmlToken {
                kind: XhtmlTokenKind::Comment,
                source_range: source_range(start, end)?,
            });
            cursor = end;
            continue;
        }

        if input[start..].starts_with("<?") {
            let end = input[start + 2..]
                .find("?>")
                .map(|relative| start + 2 + relative + 2)
                .ok_or_else(|| {
                    PageletError::Parse(ParseError::new(
                        "unterminated XHTML processing instruction",
                    ))
                })?;
            tokens.push(XhtmlToken {
                kind: XhtmlTokenKind::ProcessingInstruction,
                source_range: source_range(start, end)?,
            });
            cursor = end;
            continue;
        }

        if input[start..].starts_with("<!") {
            let end = find_tag_end(input, start)
                .ok_or_else(|| PageletError::Parse(ParseError::new("unterminated declaration")))?;
            tokens.push(XhtmlToken {
                kind: XhtmlTokenKind::Comment,
                source_range: source_range(start, end + 1)?,
            });
            cursor = end + 1;
            continue;
        }

        let end = find_tag_end(input, start)
            .ok_or_else(|| PageletError::Parse(ParseError::new("unterminated XHTML tag")))?;
        let source_range = source_range(start, end + 1)?;
        let inside = input[start + 1..end].trim();
        if let Some(rest) = inside.strip_prefix('/') {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            if name.is_empty() {
                return Err(PageletError::Parse(ParseError::new("empty XHTML end tag")));
            }
            tokens.push(XhtmlToken {
                kind: XhtmlTokenKind::EndElement { name },
                source_range,
            });
        } else {
            let self_closing = inside.ends_with('/');
            let tag = inside.trim_end_matches('/').trim();
            let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
            let name = tag[..name_end].to_owned();
            if name.is_empty() {
                return Err(PageletError::Parse(ParseError::new(
                    "empty XHTML start tag",
                )));
            }
            tokens.push(XhtmlToken {
                kind: XhtmlTokenKind::StartElement {
                    name,
                    attrs: parse_attrs(&tag[name_end..]),
                    self_closing,
                },
                source_range,
            });
        }
        cursor = end + 1;
    }
    Ok(tokens)
}

/// Build an XHTML tree in the requested compatibility mode.
pub fn parse_xhtml_tree(
    input: &str,
    mode: CompatibilityMode,
    limits: ResourceLimits,
) -> Result<XhtmlDocument, PageletError> {
    let tokens = tokenize_xhtml_with_limits(input, limits)?;
    build_xhtml_tree(&tokens, mode, limits)
}

/// Parsed CSS stylesheet for the supported M2 subset.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct CssStylesheet {
    pub imports: Vec<CssImport>,
    pub rules: Vec<CssRule>,
    pub unsupported: Vec<CssUnsupportedDeclaration>,
}

/// One CSS `@import` rule.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CssImport {
    pub href: String,
    pub source_order: u32,
}

/// One parsed CSS rule.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CssRule {
    pub selectors: Vec<CssSelector>,
    pub declarations: Vec<CssDeclaration>,
    pub source_order: u32,
}

/// Descendant selector made of simple compounds.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CssSelector {
    pub parts: Vec<CssSimpleSelector>,
    pub specificity: CssSpecificity,
}

/// One simple type/class/id selector compound.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct CssSimpleSelector {
    pub element: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

/// CSS specificity tuple.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CssSpecificity {
    pub ids: u16,
    pub classes: u16,
    pub elements: u16,
}

/// CSS declaration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CssDeclaration {
    pub property: String,
    pub value: String,
    pub important: bool,
}

/// Unsupported declaration recorded as a diagnostic input.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CssUnsupportedDeclaration {
    pub property: String,
    pub value: String,
}

/// CSS element snapshot used by the cascade engine and fuzz targets.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct CssElementSnapshot {
    pub name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub inline_style: Option<String>,
}

/// Parse CSS supported by the M2 subset without regexes.
pub fn parse_css(input: &str, limits: ResourceLimits) -> Result<CssStylesheet, PageletError> {
    let input = strip_css_comments(input);
    let mut cursor = 0;
    let mut source_order = 0_u32;
    let mut stylesheet = CssStylesheet::default();

    while cursor < input.len() {
        skip_css_ws(&input, &mut cursor);
        if cursor >= input.len() {
            break;
        }
        if input[cursor..].starts_with("@import") {
            let start = cursor + "@import".len();
            let Some(relative_end) = input[start..].find(';') else {
                return Err(PageletError::Parse(ParseError::new(
                    "unterminated CSS @import",
                )));
            };
            let import_body = input[start..start + relative_end].trim();
            if let Some(href) = parse_css_import_href(import_body) {
                stylesheet.imports.push(CssImport { href, source_order });
                source_order = source_order.saturating_add(1);
            }
            cursor = start + relative_end + 1;
            continue;
        }

        let Some(open_relative) = input[cursor..].find('{') else {
            break;
        };
        let selector_text = input[cursor..cursor + open_relative].trim();
        let body_start = cursor + open_relative + 1;
        let Some(close_relative) = input[body_start..].find('}') else {
            return Err(PageletError::Parse(ParseError::new(
                "unterminated CSS rule",
            )));
        };
        let body_end = body_start + close_relative;
        let selectors = parse_css_selectors(selector_text, limits)?;
        if !selectors.is_empty() {
            let (declarations, unsupported) = parse_css_declarations(&input[body_start..body_end]);
            stylesheet.unsupported.extend(unsupported);
            stylesheet.rules.push(CssRule {
                selectors,
                declarations,
                source_order,
            });
            source_order = source_order.saturating_add(1);
        }
        cursor = body_end + 1;
    }

    Ok(stylesheet)
}

/// Compute a style for one element snapshot and its ancestors.
pub fn cascade_css_for_element(
    element: &CssElementSnapshot,
    ancestors: &[CssElementSnapshot],
    stylesheet: &CssStylesheet,
    inherited: &document::ComputedStyle,
) -> document::ComputedStyle {
    let mut winners = BTreeMap::<String, (CssSpecificity, u32, bool, String)>::new();
    for (name, value) in inherited
        .properties
        .iter()
        .filter(|(name, _)| is_inherited_css_property(name))
    {
        winners.insert(
            name.to_string(),
            (CssSpecificity::default(), 0, false, value.to_string()),
        );
    }

    for rule in &stylesheet.rules {
        for selector in &rule.selectors {
            if selector_matches(selector, element, ancestors) {
                for declaration in &rule.declarations {
                    if !is_supported_css_property(&declaration.property) {
                        continue;
                    }
                    let candidate = (
                        selector.specificity,
                        rule.source_order,
                        declaration.important,
                        declaration.value.clone(),
                    );
                    let replace = match winners.get(&declaration.property) {
                        Some(existing) => css_candidate_wins(&candidate, existing),
                        None => true,
                    };
                    if replace {
                        winners.insert(declaration.property.clone(), candidate);
                    }
                }
            }
        }
    }

    if let Some(inline_style) = &element.inline_style {
        let (declarations, _) = parse_css_declarations(inline_style);
        for (source_order, declaration) in declarations.into_iter().enumerate() {
            if !is_supported_css_property(&declaration.property) {
                continue;
            }
            winners.insert(
                declaration.property,
                (
                    CssSpecificity {
                        ids: u16::MAX,
                        classes: u16::MAX,
                        elements: u16::MAX,
                    },
                    u32::try_from(source_order).unwrap_or(u32::MAX),
                    declaration.important,
                    declaration.value,
                ),
            );
        }
    }

    let mut style = document::ComputedStyle::new();
    for (name, (_, _, _, value)) in winners {
        style = style.with_property(name, value);
    }
    style
}

fn strip_css_comments(input: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while cursor < input.len() {
        if input[cursor..].starts_with("/*") {
            if let Some(end) = input[cursor + 2..].find("*/") {
                cursor += 2 + end + 2;
            } else {
                break;
            }
        } else if let Some(ch) = input[cursor..].chars().next() {
            out.push(ch);
            cursor += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn skip_css_ws(input: &str, cursor: &mut usize) {
    while input
        .as_bytes()
        .get(*cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        *cursor += 1;
    }
}

fn parse_css_import_href(input: &str) -> Option<String> {
    let input = input.trim();
    if let Some(value) = input.strip_prefix("url(") {
        return value
            .split(')')
            .next()
            .map(|href| href.trim().trim_matches('"').trim_matches('\'').to_owned())
            .filter(|href| !href.is_empty());
    }
    input
        .trim_matches('"')
        .trim_matches('\'')
        .split_whitespace()
        .next()
        .map(ToOwned::to_owned)
        .filter(|href| !href.is_empty())
}

fn parse_css_selectors(
    input: &str,
    limits: ResourceLimits,
) -> Result<Vec<CssSelector>, PageletError> {
    let mut selectors = Vec::new();
    for raw in input.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let observed = u64::try_from(selectors.len().saturating_add(1)).unwrap_or(u64::MAX);
        if observed > u64::from(limits.max_css_selectors) {
            return Err(limit_error(
                ResourceLimitKind::CssSelectors,
                u64::from(limits.max_css_selectors),
                observed,
            ));
        }
        let mut parts = Vec::new();
        for compound in raw.split_whitespace() {
            if let Some(selector) = parse_css_simple_selector(compound) {
                parts.push(selector);
            }
        }
        if !parts.is_empty() {
            selectors.push(CssSelector {
                specificity: css_specificity(&parts),
                parts,
            });
        }
    }
    Ok(selectors)
}

fn parse_css_simple_selector(input: &str) -> Option<CssSimpleSelector> {
    let mut selector = CssSimpleSelector::default();
    let bytes = input.as_bytes();
    let mut cursor = 0;
    if bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'*')
    {
        let start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
        {
            cursor += 1;
        }
        if &input[start..cursor] != "*" {
            selector.element = Some(input[start..cursor].to_ascii_lowercase());
        }
    }

    while cursor < input.len() {
        let marker = bytes[cursor];
        if marker != b'.' && marker != b'#' {
            return None;
        }
        cursor += 1;
        let start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
        {
            cursor += 1;
        }
        if start == cursor {
            return None;
        }
        let value = input[start..cursor].to_owned();
        if marker == b'#' {
            selector.id = Some(value);
        } else {
            selector.classes.push(value);
        }
    }

    Some(selector)
}

fn css_specificity(parts: &[CssSimpleSelector]) -> CssSpecificity {
    let mut specificity = CssSpecificity::default();
    for part in parts {
        if part.id.is_some() {
            specificity.ids = specificity.ids.saturating_add(1);
        }
        specificity.classes = specificity
            .classes
            .saturating_add(u16::try_from(part.classes.len()).unwrap_or(u16::MAX));
        if part.element.is_some() {
            specificity.elements = specificity.elements.saturating_add(1);
        }
    }
    specificity
}

fn parse_css_declarations(input: &str) -> (Vec<CssDeclaration>, Vec<CssUnsupportedDeclaration>) {
    let mut declarations = Vec::new();
    let mut unsupported = Vec::new();
    for raw in input.split(';') {
        let Some((property, value)) = raw.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let mut value = value.trim().to_owned();
        let important = value.to_ascii_lowercase().ends_with("!important");
        if important {
            let len = value.len().saturating_sub("!important".len());
            value = value[..len].trim().to_owned();
        }
        if property.is_empty() || value.is_empty() {
            continue;
        }
        if is_supported_css_property(&property) {
            if matches!(property.as_str(), "margin" | "padding") {
                if let Some(expanded) = expand_box_shorthand(&property, &value, important) {
                    declarations.extend(expanded);
                } else {
                    declarations.push(CssDeclaration {
                        property,
                        value,
                        important,
                    });
                }
            } else {
                declarations.push(CssDeclaration {
                    property,
                    value,
                    important,
                });
            }
        } else {
            unsupported.push(CssUnsupportedDeclaration { property, value });
        }
    }
    (declarations, unsupported)
}

fn expand_box_shorthand(
    property: &str,
    value: &str,
    important: bool,
) -> Option<Vec<CssDeclaration>> {
    let values = value.split_ascii_whitespace().collect::<Vec<_>>();
    let [top, right, bottom, left] = match values.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return None,
    };
    Some(
        [
            ("top", top),
            ("right", right),
            ("bottom", bottom),
            ("left", left),
        ]
        .into_iter()
        .map(|(side, value)| CssDeclaration {
            property: format!("{property}-{side}"),
            value: value.to_owned(),
            important,
        })
        .collect(),
    )
}

fn is_supported_css_property(property: &str) -> bool {
    matches!(
        property,
        "display"
            | "visibility"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-stretch"
            | "line-height"
            | "letter-spacing"
            | "list-style"
            | "list-style-type"
            | "text-align"
            | "text-indent"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "width"
            | "height"
            | "max-width"
            | "max-height"
            | "break-before"
            | "break-after"
            | "break-inside"
            | "page-break-before"
            | "page-break-after"
            | "page-break-inside"
            | "widows"
            | "orphans"
            | "direction"
            | "writing-mode"
    )
}

fn is_inherited_css_property(property: &str) -> bool {
    matches!(
        property,
        "visibility"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-stretch"
            | "line-height"
            | "letter-spacing"
            | "text-align"
            | "text-indent"
            | "widows"
            | "orphans"
            | "direction"
            | "writing-mode"
    )
}

fn apply_inline_user_agent_defaults(element_name: &str, style: &mut document::ComputedStyle) {
    match element_name {
        "em" | "i" => {
            style
                .properties
                .insert(Arc::from("font-style"), Arc::from("italic"));
        }
        "strong" | "b" => {
            style
                .properties
                .insert(Arc::from("font-weight"), Arc::from("bold"));
        }
        _ => {}
    }
}

fn selector_matches(
    selector: &CssSelector,
    element: &CssElementSnapshot,
    ancestors: &[CssElementSnapshot],
) -> bool {
    let Some(last) = selector.parts.last() else {
        return false;
    };
    if !simple_selector_matches(last, element) {
        return false;
    }
    if selector.parts.len() == 1 {
        return true;
    }

    let mut ancestor_index = ancestors.len();
    for part in selector.parts[..selector.parts.len() - 1].iter().rev() {
        let mut found = false;
        while ancestor_index > 0 {
            ancestor_index -= 1;
            if simple_selector_matches(part, &ancestors[ancestor_index]) {
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

fn simple_selector_matches(selector: &CssSimpleSelector, element: &CssElementSnapshot) -> bool {
    if selector
        .element
        .as_deref()
        .is_some_and(|name| name != element.name)
    {
        return false;
    }
    if selector
        .id
        .as_deref()
        .is_some_and(|id| element.id.as_deref() != Some(id))
    {
        return false;
    }
    selector
        .classes
        .iter()
        .all(|class| element.classes.iter().any(|item| item == class))
}

fn css_candidate_wins(
    candidate: &(CssSpecificity, u32, bool, String),
    existing: &(CssSpecificity, u32, bool, String),
) -> bool {
    if candidate.2 != existing.2 {
        return candidate.2;
    }
    (candidate.0, candidate.1) >= (existing.0, existing.1)
}

fn build_xhtml_tree(
    tokens: &[XhtmlToken],
    mode: CompatibilityMode,
    limits: ResourceLimits,
) -> Result<XhtmlDocument, PageletError> {
    let root_range = SourceRange::new(0, 0).expect("root source range is valid");
    let mut nodes = vec![XhtmlNode {
        kind: XhtmlNodeKind::Element(XhtmlElement {
            name: "#document".to_owned(),
            attrs: BTreeMap::new(),
            children: Vec::new(),
        }),
        source_range: root_range,
    }];
    let mut stack = vec![0_usize];

    for token in tokens {
        match &token.kind {
            XhtmlTokenKind::StartElement {
                name,
                attrs,
                self_closing,
            } => {
                let observed = u64::try_from(nodes.len().saturating_add(1)).unwrap_or(u64::MAX);
                if observed > u64::from(limits.max_dom_nodes) {
                    return Err(limit_error(
                        ResourceLimitKind::DomNodes,
                        u64::from(limits.max_dom_nodes),
                        observed,
                    ));
                }
                let depth = u64::try_from(stack.len().saturating_add(1)).unwrap_or(u64::MAX);
                if depth > u64::from(limits.max_xml_depth) {
                    return Err(limit_error(
                        ResourceLimitKind::XmlDepth,
                        u64::from(limits.max_xml_depth),
                        depth,
                    ));
                }
                let node_id = nodes.len();
                nodes.push(XhtmlNode {
                    kind: XhtmlNodeKind::Element(XhtmlElement {
                        name: name.clone(),
                        attrs: attrs.clone(),
                        children: Vec::new(),
                    }),
                    source_range: token.source_range,
                });
                push_child(&mut nodes, *stack.last().unwrap_or(&0), node_id);
                if !*self_closing && !is_void_xhtml_element(name) {
                    stack.push(node_id);
                }
            }
            XhtmlTokenKind::EndElement { name } => {
                close_xhtml_element(&mut nodes, &mut stack, name, token.source_range, mode)?;
            }
            XhtmlTokenKind::Text(text) => {
                let observed = u64::try_from(nodes.len().saturating_add(1)).unwrap_or(u64::MAX);
                if observed > u64::from(limits.max_dom_nodes) {
                    return Err(limit_error(
                        ResourceLimitKind::DomNodes,
                        u64::from(limits.max_dom_nodes),
                        observed,
                    ));
                }
                let node_id = nodes.len();
                nodes.push(XhtmlNode {
                    kind: XhtmlNodeKind::Text(text.clone()),
                    source_range: token.source_range,
                });
                push_child(&mut nodes, *stack.last().unwrap_or(&0), node_id);
            }
            XhtmlTokenKind::Comment | XhtmlTokenKind::ProcessingInstruction => {}
        }
    }

    if stack.len() > 1 && mode == CompatibilityMode::Strict {
        return Err(PageletError::Parse(ParseError::new(format!(
            "unclosed XHTML element: {}",
            element_name(&nodes, *stack.last().unwrap_or(&0))
        ))));
    }
    if let Some(last) = tokens.last() {
        for node_id in stack.iter().copied().skip(1) {
            nodes[node_id].source_range.end = last.source_range.end;
        }
        nodes[0].source_range.end = last.source_range.end;
    }

    Ok(XhtmlDocument { nodes, root: 0 })
}

fn push_text_token(
    input: &str,
    start: usize,
    end: usize,
    tokens: &mut Vec<XhtmlToken>,
) -> Result<(), PageletError> {
    if start == end {
        return Ok(());
    }
    let text = unescape_xml(&input[start..end]);
    if text.is_empty() {
        return Ok(());
    }
    tokens.push(XhtmlToken {
        kind: XhtmlTokenKind::Text(text),
        source_range: source_range(start, end)?,
    });
    Ok(())
}

fn source_range(start: usize, end: usize) -> Result<SourceRange, PageletError> {
    SourceRange::new(
        u32::try_from(start)
            .map_err(|_| PageletError::Parse(ParseError::new("source range start exceeds u32")))?,
        u32::try_from(end)
            .map_err(|_| PageletError::Parse(ParseError::new("source range end exceeds u32")))?,
    )
    .ok_or_else(|| PageletError::Parse(ParseError::new("invalid source range")))
}

fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut cursor = start + 1;
    let bytes = input.as_bytes();
    while cursor < input.len() {
        let byte = *bytes.get(cursor)?;
        match quote {
            Some(active) if byte == active => quote = None,
            Some(_) => {}
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b'>' => return Some(cursor),
            None => {}
        }
        cursor += 1;
    }
    None
}

fn push_child(nodes: &mut [XhtmlNode], parent: usize, child: usize) {
    if let Some(XhtmlNode {
        kind: XhtmlNodeKind::Element(element),
        ..
    }) = nodes.get_mut(parent)
    {
        element.children.push(child);
    }
}

fn close_xhtml_element(
    nodes: &mut [XhtmlNode],
    stack: &mut Vec<usize>,
    name: &str,
    end_range: SourceRange,
    mode: CompatibilityMode,
) -> Result<(), PageletError> {
    let expected = stack
        .last()
        .copied()
        .map(|id| element_name(nodes, id).to_owned())
        .unwrap_or_default();
    if local_name(&expected) == local_name(name) {
        if let Some(node_id) = stack.pop() {
            nodes[node_id].source_range.end = end_range.end;
        }
        return Ok(());
    }

    if mode == CompatibilityMode::Strict {
        return Err(PageletError::Parse(ParseError::new(format!(
            "mismatched XHTML end tag: expected </{}>, got </{}>",
            expected, name
        ))));
    }

    if let Some(position) = stack
        .iter()
        .rposition(|node_id| local_name(element_name(nodes, *node_id)) == local_name(name))
    {
        for node_id in stack.drain(position..) {
            nodes[node_id].source_range.end = end_range.end;
        }
    }
    Ok(())
}

fn element_name(nodes: &[XhtmlNode], node_id: usize) -> &str {
    nodes
        .get(node_id)
        .and_then(XhtmlNode::element)
        .map_or("#text", |element| element.name.as_str())
}

fn is_void_xhtml_element(name: &str) -> bool {
    matches!(
        local_name(name),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn load_css_cascade(
    tree: &XhtmlDocument,
    document_href: &str,
    resources: &document::ResourceTable,
    store: Option<&ZipPublicationStore>,
    options: OpenOptions,
) -> Result<CssStylesheet, PageletError> {
    let mut cascade = CssStylesheet::default();
    let base_dir = parent_path(document_href);
    let mut linked = Vec::new();
    collect_linked_stylesheets(tree, tree.root, &mut linked);
    let mut visited = BTreeSet::new();
    for href in linked {
        let path = resolve_resource_path(&base_dir, &href)?;
        if resources.id_for_path(&path).is_none() {
            continue;
        }
        if let Some(store) = store {
            load_css_resource(store, &path, options, &mut visited, 0, &mut cascade)?;
        }
    }

    let mut inline_styles = Vec::new();
    collect_style_elements(tree, tree.root, &mut inline_styles);
    for css in inline_styles {
        merge_css_stylesheet(&mut cascade, parse_css(&css, options.limits)?);
    }
    Ok(cascade)
}

fn collect_linked_stylesheets(tree: &XhtmlDocument, node_id: usize, out: &mut Vec<String>) {
    let Some(node) = tree.node(node_id) else {
        return;
    };
    let Some(element) = node.element() else {
        return;
    };
    if element.local_name() == "link"
        && element
            .attr("rel")
            .is_some_and(|value| value.split_whitespace().any(|item| item == "stylesheet"))
        && element.attr("href").is_some()
    {
        out.push(element.attr("href").unwrap_or_default().to_owned());
    }
    for child in &element.children {
        collect_linked_stylesheets(tree, *child, out);
    }
}

fn collect_style_elements(tree: &XhtmlDocument, node_id: usize, out: &mut Vec<String>) {
    let Some(node) = tree.node(node_id) else {
        return;
    };
    let Some(element) = node.element() else {
        return;
    };
    if element.local_name() == "style" {
        let mut css = String::new();
        collect_raw_text(tree, node_id, &mut css);
        out.push(css);
    }
    for child in &element.children {
        collect_style_elements(tree, *child, out);
    }
}

fn collect_raw_text(tree: &XhtmlDocument, node_id: usize, out: &mut String) {
    let Some(node) = tree.node(node_id) else {
        return;
    };
    match &node.kind {
        XhtmlNodeKind::Text(text) => out.push_str(text),
        XhtmlNodeKind::Element(element) => {
            for child in &element.children {
                collect_raw_text(tree, *child, out);
            }
        }
    }
}

fn load_css_resource(
    store: &ZipPublicationStore,
    path: &str,
    options: OpenOptions,
    visited: &mut BTreeSet<String>,
    depth: u32,
    cascade: &mut CssStylesheet,
) -> Result<(), PageletError> {
    if depth > options.limits.max_css_import_depth {
        return Err(limit_error(
            ResourceLimitKind::CssImportDepth,
            u64::from(options.limits.max_css_import_depth),
            u64::from(depth),
        ));
    }
    if !visited.insert(path.to_owned()) {
        return Err(PageletError::Parse(ParseError::new(format!(
            "CSS import cycle detected at {path}"
        ))));
    }
    let bytes = store.read_path(path, options.compatibility_mode)?;
    let text = resource_text(&bytes)?;
    let stylesheet = parse_css(&text, options.limits)?;
    let imports = stylesheet.imports.clone();

    let base_dir = parent_path(path);
    for import in imports {
        let import_path = resolve_resource_path(&base_dir, &import.href)?;
        load_css_resource(
            store,
            &import_path,
            options,
            visited,
            depth.saturating_add(1),
            cascade,
        )?;
    }
    merge_css_stylesheet(cascade, stylesheet);
    visited.remove(path);
    Ok(())
}

fn merge_css_stylesheet(target: &mut CssStylesheet, mut source: CssStylesheet) {
    let base = u32::try_from(target.rules.len()).unwrap_or(u32::MAX);
    for rule in &mut source.rules {
        rule.source_order = rule.source_order.saturating_add(base);
    }
    target.imports.extend(source.imports);
    target.rules.extend(source.rules);
    target.unsupported.extend(source.unsupported);
}

#[derive(Clone, Copy)]
struct ChapterResourceContext<'a> {
    resources: &'a document::ResourceTable,
    cover_image: Option<&'a str>,
    store: Option<&'a ZipPublicationStore>,
    options: OpenOptions,
}

fn chapter_ir_from_xhtml(
    document_id: DocumentId,
    href: &str,
    title: &str,
    input: &str,
    context: ChapterResourceContext<'_>,
) -> Result<document::ChapterIr, PageletError> {
    let ChapterResourceContext {
        resources,
        cover_image,
        store,
        options,
    } = context;
    let content_hash = ContentHash::from_bytes(input.as_bytes());
    let tree = match parse_xhtml_tree(input, CompatibilityMode::Strict, options.limits) {
        Ok(tree) => tree,
        Err(error) if options.compatibility_mode == CompatibilityMode::Strict => return Err(error),
        Err(_) => match parse_xhtml_tree(input, CompatibilityMode::Compatible, options.limits) {
            Ok(tree) => tree,
            Err(_) => {
                return salvage_chapter_ir(document_id, href, title, input, content_hash);
            }
        },
    };
    let css = load_css_cascade(&tree, href, resources, store, options)?;
    let base_dir = parent_path(href);
    let mut referenced_footnote_keys = BTreeSet::new();
    collect_referenced_footnote_keys(
        &tree,
        tree.root,
        href,
        &base_dir,
        &mut referenced_footnote_keys,
    );

    let mut builder = ChapterBuilder {
        document_href: href,
        base_dir,
        resources,
        cover_image,
        store,
        options,
        css,
        referenced_footnote_keys,
        chapter: document::ChapterIr::empty(document_id, href, title, content_hash),
        default_style: StyleId::new(0),
        computed_styles: BTreeMap::new(),
        font_contexts: BTreeMap::new(),
        tree: &tree,
    };
    builder.build()
}

fn salvage_chapter_ir(
    document_id: DocumentId,
    href: &str,
    title: &str,
    input: &str,
    content_hash: ContentHash,
) -> Result<document::ChapterIr, PageletError> {
    let mut chapter = document::ChapterIr::empty(document_id, href, title, content_hash);
    let visible = strip_tags(input);
    let text = chapter.text_pool.push(visible.trim())?;
    let paragraph = chapter
        .nodes
        .push(document::DocumentNode::Paragraph(document::BlockText {
            text,
            style: StyleId::new(0),
            style_runs: Vec::new(),
        }))?;
    let root = chapter
        .nodes
        .push(document::DocumentNode::Container(document::ContainerNode {
            children: vec![paragraph],
            style: StyleId::new(0),
        }))?;
    chapter.root = root;
    chapter.source_map.insert(
        root,
        SourceRange::new(0, u32::try_from(input.len()).unwrap_or(u32::MAX)),
    );
    chapter.rebuild_utf16_index();
    chapter.rebuild_blocks();
    Ok(chapter)
}

struct ChapterBuilder<'a> {
    document_href: &'a str,
    base_dir: String,
    resources: &'a document::ResourceTable,
    cover_image: Option<&'a str>,
    store: Option<&'a ZipPublicationStore>,
    options: OpenOptions,
    css: CssStylesheet,
    referenced_footnote_keys: BTreeSet<String>,
    chapter: document::ChapterIr,
    default_style: StyleId,
    computed_styles: BTreeMap<usize, (StyleId, document::ComputedStyle)>,
    font_contexts: BTreeMap<usize, ResolvedFontContext>,
    tree: &'a XhtmlDocument,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedFontContext {
    font_size: LayoutUnit,
    line_height: ResolvedLineHeight,
}

#[derive(Debug, Clone, Copy)]
enum ResolvedLineHeight {
    Normal,
    Absolute(LayoutUnit),
    Multiplier(f64),
}

impl Default for ResolvedFontContext {
    fn default() -> Self {
        Self {
            font_size: LayoutUnit::from_px(16),
            line_height: ResolvedLineHeight::Normal,
        }
    }
}

impl ChapterBuilder<'_> {
    fn build(&mut self) -> Result<document::ChapterIr, PageletError> {
        for unsupported in &self.css.unsupported {
            self.chapter.diagnostics.push(Diagnostic::new(
                DiagnosticCode::UnsupportedFeature,
                Severity::Warning,
                format!("unsupported CSS property: {}", unsupported.property),
            ));
        }
        let content_root = self.content_root();
        let children = self.convert_children(content_root)?;
        let style = self.style_for_node(content_root)?;
        let root_range = self.tree.node(content_root).map(|node| node.source_range);
        let root = self.push_node(
            document::DocumentNode::Container(document::ContainerNode { children, style }),
            root_range,
        )?;
        self.chapter.root = root;
        self.resolve_footnotes()?;
        self.classify_standalone_image();
        self.chapter.rebuild_utf16_index();
        self.chapter.rebuild_blocks();
        Ok(self.chapter.clone())
    }

    fn content_root(&self) -> usize {
        self.find_first_element(self.tree.root, "body")
            .unwrap_or(self.tree.root)
    }

    fn find_first_element(&self, node_id: usize, name: &str) -> Option<usize> {
        let node = self.tree.node(node_id)?;
        let element = node.element()?;
        if element.local_name() == name {
            return Some(node_id);
        }
        element
            .children
            .iter()
            .find_map(|child| self.find_first_element(*child, name))
    }

    fn convert_children(&mut self, node_id: usize) -> Result<Vec<NodeId>, PageletError> {
        let Some(element) = self.tree.node(node_id).and_then(XhtmlNode::element) else {
            return Ok(Vec::new());
        };
        let children = element.children.clone();
        let mut out = Vec::new();
        for child in children {
            if let Some(node_id) = self.convert_node(child)? {
                out.push(node_id);
            }
        }
        Ok(out)
    }

    fn convert_flow_children(
        &mut self,
        node_id: usize,
        parent_style: StyleId,
    ) -> Result<Vec<NodeId>, PageletError> {
        let Some(element) = self.tree.node(node_id).and_then(XhtmlNode::element) else {
            return Ok(Vec::new());
        };
        let children = element.children.clone();
        let mut out = Vec::new();
        let mut inline_group = Vec::new();
        for child in children {
            if self.is_inline_flow_node(child) {
                inline_group.push(child);
                continue;
            }
            self.flush_inline_group(&mut inline_group, parent_style, &mut out)?;
            if let Some(converted) = self.convert_node(child)? {
                out.push(converted);
            }
        }
        self.flush_inline_group(&mut inline_group, parent_style, &mut out)?;
        Ok(out)
    }

    fn is_inline_flow_node(&self, node_id: usize) -> bool {
        let Some(node) = self.tree.node(node_id) else {
            return false;
        };
        match &node.kind {
            XhtmlNodeKind::Text(_) => true,
            XhtmlNodeKind::Element(element) => {
                if element.local_name() == "a" && self.sole_image_child(node_id).is_some() {
                    return false;
                }
                matches!(
                    element.local_name(),
                    "a" | "abbr"
                        | "b"
                        | "bdi"
                        | "bdo"
                        | "br"
                        | "cite"
                        | "code"
                        | "data"
                        | "dfn"
                        | "em"
                        | "i"
                        | "kbd"
                        | "mark"
                        | "q"
                        | "ruby"
                        | "s"
                        | "samp"
                        | "small"
                        | "span"
                        | "strong"
                        | "sub"
                        | "sup"
                        | "time"
                        | "u"
                        | "var"
                        | "wbr"
                )
            }
        }
    }

    fn flush_inline_group(
        &mut self,
        inline_group: &mut Vec<usize>,
        parent_style: StyleId,
        out: &mut Vec<NodeId>,
    ) -> Result<(), PageletError> {
        if inline_group.is_empty() {
            return Ok(());
        }
        let nodes = std::mem::take(inline_group);
        let content = self.inline_content_for_nodes(&nodes, parent_style)?;
        if content.text.is_empty() {
            return Ok(());
        }
        let source_range = nodes
            .iter()
            .filter_map(|node_id| self.tree.node(*node_id).map(|node| node.source_range))
            .reduce(|first, next| SourceRange {
                start: first.start.min(next.start),
                end: first.end.max(next.end),
            })
            .unwrap_or_default();
        let block_style = self.anonymous_block_style(parent_style)?;
        let node_id = self.push_inline_text_node(
            DocumentNodeKind::Paragraph,
            content,
            source_range,
            block_style,
        )?;
        out.push(node_id);
        Ok(())
    }

    fn convert_node(&mut self, node_id: usize) -> Result<Option<NodeId>, PageletError> {
        let Some(node) = self.tree.node(node_id) else {
            return Ok(None);
        };
        let source_range = node.source_range;
        let kind = node.kind.clone();
        match kind {
            XhtmlNodeKind::Text(text) => {
                let text = text.trim();
                if text.is_empty() {
                    return Ok(None);
                }
                let style = self
                    .parent_node(node_id)
                    .map_or(Ok(self.default_style), |parent| self.style_for_node(parent))?;
                Ok(Some(self.text_block_node(
                    DocumentNodeKind::Paragraph,
                    text,
                    source_range,
                    style,
                )?))
            }
            XhtmlNodeKind::Element(element) => self.convert_element(node_id, &element),
        }
    }

    fn convert_element(
        &mut self,
        node_id: usize,
        element: &XhtmlElement,
    ) -> Result<Option<NodeId>, PageletError> {
        let source_range = self.tree.node(node_id).map(|node| node.source_range);
        let style = self.style_for_node(node_id)?;
        let converted = match element.local_name() {
            "html" | "body" | "section" | "article" | "main" | "div" | "nav" => {
                let children = self.convert_flow_children(node_id, style)?;
                Some(self.push_node(
                    document::DocumentNode::Container(document::ContainerNode { children, style }),
                    source_range,
                )?)
            }
            "p" => Some(self.block_text_node(node_id, DocumentNodeKind::Paragraph, style)?),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = element.local_name()[1..].parse::<u8>().unwrap_or(1);
                Some(self.heading_node(node_id, level, style)?)
            }
            "ul" | "ol" => {
                let children = self.convert_children(node_id)?;
                Some(self.push_node(
                    document::DocumentNode::List(document::ListNode {
                        ordered: element.local_name() == "ol",
                        children,
                        style,
                    }),
                    source_range,
                )?)
            }
            "li" => {
                let children = self.convert_flow_children(node_id, style)?;
                Some(self.push_node(
                    document::DocumentNode::ListItem(document::ListItemNode { children, style }),
                    source_range,
                )?)
            }
            "blockquote" => {
                let children = self.convert_flow_children(node_id, style)?;
                Some(self.push_node(
                    document::DocumentNode::BlockQuote(document::ContainerNode { children, style }),
                    source_range,
                )?)
            }
            "figure" | "svg" => {
                let children = self.convert_flow_children(node_id, style)?;
                Some(self.push_node(
                    document::DocumentNode::Figure(document::ContainerNode { children, style }),
                    source_range,
                )?)
            }
            "table" => {
                let children = self.convert_children(node_id)?;
                Some(self.push_node(
                    document::DocumentNode::Table(document::ContainerNode { children, style }),
                    source_range,
                )?)
            }
            "img" | "image" => Some(self.image_node(element, source_range, style)?),
            "hr" => Some(self.push_node(document::DocumentNode::Divider, source_range)?),
            "br" => Some(self.push_node(document::DocumentNode::ForcedBreak, source_range)?),
            "aside" if is_footnote_element(element) && self.footnote_is_referenced(element) => {
                Some(self.footnote_node(node_id, element, style)?)
            }
            "aside" if is_footnote_element(element) => None,
            "a" if self.sole_image_child(node_id).is_some() => {
                self.linked_image_anchor_node(node_id, element)?
            }
            "a" | "span" | "em" | "strong" | "b" | "i" => {
                self.inline_element_text_node(node_id, style)?
            }
            _ => {
                let children = self.convert_children(node_id)?;
                Some(self.push_node(
                    document::DocumentNode::Unsupported(document::UnsupportedNode {
                        element: Arc::from(element.name.as_str()),
                        children,
                        style,
                    }),
                    source_range,
                )?)
            }
        };

        if let Some(converted) = converted {
            self.register_element_anchor(element, converted, source_range);
            Ok(Some(converted))
        } else {
            Ok(None)
        }
    }

    fn inline_element_text_node(
        &mut self,
        tree_node_id: usize,
        style: StyleId,
    ) -> Result<Option<NodeId>, PageletError> {
        let content = self.inline_content(tree_node_id)?;
        if content.text.is_empty() {
            return Ok(None);
        }
        let source_range = self
            .tree
            .node(tree_node_id)
            .map(|node| node.source_range)
            .unwrap_or_default();
        let node_id =
            self.push_inline_text_node(DocumentNodeKind::Paragraph, content, source_range, style)?;
        Ok(Some(node_id))
    }

    fn sole_image_child(&self, tree_node_id: usize) -> Option<usize> {
        let element = self.tree.node(tree_node_id)?.element()?;
        let mut image_child = None;
        for child_id in &element.children {
            let child = self.tree.node(*child_id)?;
            match &child.kind {
                XhtmlNodeKind::Text(text) if text.chars().all(is_collapsible_xhtml_whitespace) => {}
                XhtmlNodeKind::Element(child_element)
                    if matches!(child_element.local_name(), "img" | "image")
                        && image_child.is_none() =>
                {
                    image_child = Some(*child_id);
                }
                _ => return None,
            }
        }
        image_child
    }

    fn linked_image_anchor_node(
        &mut self,
        tree_node_id: usize,
        anchor: &XhtmlElement,
    ) -> Result<Option<NodeId>, PageletError> {
        let Some(image_tree_id) = self.sole_image_child(tree_node_id) else {
            return Ok(None);
        };
        let Some(image_tree_node) = self.tree.node(image_tree_id) else {
            return Ok(None);
        };
        let image_source_range = image_tree_node.source_range;
        let XhtmlNodeKind::Element(image_element) = image_tree_node.kind.clone() else {
            return Ok(None);
        };
        let image_style = self.style_for_node(image_tree_id)?;
        let image_node = self.image_node(&image_element, Some(image_source_range), image_style)?;
        self.register_element_anchor(&image_element, image_node, Some(image_source_range));

        if let Some(href) = anchor.attr("href") {
            let anchor_source_range = self.tree.node(tree_node_id).map(|node| node.source_range);
            self.chapter.links.push(resolve_link(
                self.document_href,
                &self.base_dir,
                image_node,
                LinkDraft {
                    href: Arc::from(href),
                    text_range: None,
                    source_range: anchor_source_range,
                    kind: if is_noteref_element(anchor) {
                        document::LinkKind::Footnote
                    } else {
                        document::LinkKind::Internal
                    },
                },
                self.resources,
            ));
        }

        Ok(Some(image_node))
    }

    fn block_text_node(
        &mut self,
        tree_node_id: usize,
        kind: DocumentNodeKind,
        style: StyleId,
    ) -> Result<NodeId, PageletError> {
        let content = self.inline_content(tree_node_id)?;
        let source_range = self.tree.node(tree_node_id).map(|node| node.source_range);
        self.push_inline_text_node(kind, content, source_range.unwrap_or_default(), style)
    }

    fn heading_node(
        &mut self,
        tree_node_id: usize,
        level: u8,
        style: StyleId,
    ) -> Result<NodeId, PageletError> {
        let content = self.inline_content(tree_node_id)?;
        let text = self.chapter.text_pool.push(&content.text)?;
        let node_id = self.push_node(
            document::DocumentNode::Heading(document::HeadingNode {
                level,
                content: document::BlockText {
                    text,
                    style,
                    style_runs: content.style_runs,
                },
            }),
            self.tree.node(tree_node_id).map(|node| node.source_range),
        )?;
        self.add_inline_metadata(node_id, content.links, content.anchors);
        Ok(node_id)
    }

    fn text_block_node(
        &mut self,
        kind: DocumentNodeKind,
        text: &str,
        source_range: SourceRange,
        style: StyleId,
    ) -> Result<NodeId, PageletError> {
        let text_end = u32::try_from(text.len()).unwrap_or(u32::MAX);
        let text = self.chapter.text_pool.push(text)?;
        let block = document::BlockText {
            text,
            style,
            style_runs: (text_end > 0)
                .then_some(document::InlineStyleRun {
                    start: 0,
                    end: text_end,
                    style,
                    source_range: Some(source_range),
                })
                .into_iter()
                .collect(),
        };
        let node = match kind {
            DocumentNodeKind::Paragraph => document::DocumentNode::Paragraph(block),
        };
        self.push_node(node, Some(source_range))
    }

    fn push_inline_text_node(
        &mut self,
        kind: DocumentNodeKind,
        content: InlineContent,
        source_range: SourceRange,
        style: StyleId,
    ) -> Result<NodeId, PageletError> {
        let text = self.chapter.text_pool.push(&content.text)?;
        let block = document::BlockText {
            text,
            style,
            style_runs: content.style_runs,
        };
        let node = match kind {
            DocumentNodeKind::Paragraph => document::DocumentNode::Paragraph(block),
        };
        let node_id = self.push_node(node, Some(source_range))?;
        self.add_inline_metadata(node_id, content.links, content.anchors);
        Ok(node_id)
    }

    fn image_node(
        &mut self,
        element: &XhtmlElement,
        source_range: Option<SourceRange>,
        style: StyleId,
    ) -> Result<NodeId, PageletError> {
        let src = element
            .attr("src")
            .or_else(|| element.attr("xlink:href"))
            .or_else(|| element.attr("href"))
            .unwrap_or_default();
        let resolved_path = resolve_resource_path(&self.base_dir, src).ok();
        let resource_id = resolved_path
            .as_deref()
            .and_then(|path| self.resources.id_for_path(path));
        let intrinsic_size = resource_id
            .and_then(|resource_id| self.resources.image(resource_id))
            .and_then(|resource| resource.intrinsic_size);
        let layout_role = if resolved_path.as_deref() == self.cover_image {
            document::ImageLayoutRole::Cover
        } else {
            document::ImageLayoutRole::Block
        };
        self.push_node(
            document::DocumentNode::Image(document::ImageNode {
                src: Arc::from(src),
                resolved_path: resolved_path.as_deref().map(Arc::from),
                resource_id,
                intrinsic_size,
                layout_role,
                alt: Arc::from(element.attr("alt").unwrap_or_default()),
                title: element.attr("title").map(Arc::from),
                style,
            }),
            source_range,
        )
    }

    fn classify_standalone_image(&mut self) {
        let mut image_ids = Vec::new();
        let mut has_other_visible_content = false;
        for (node_id, node) in self.chapter.nodes.iter_with_ids() {
            match node {
                document::DocumentNode::Image(_) => image_ids.push(node_id),
                document::DocumentNode::Paragraph(block) => {
                    has_other_visible_content |= self
                        .chapter
                        .text_pool
                        .get(block.text)
                        .is_some_and(|text| !text.trim().is_empty());
                }
                document::DocumentNode::Heading(heading) => {
                    has_other_visible_content |= self
                        .chapter
                        .text_pool
                        .get(heading.content.text)
                        .is_some_and(|text| !text.trim().is_empty());
                }
                document::DocumentNode::Divider | document::DocumentNode::ForcedBreak => {
                    has_other_visible_content = true;
                }
                document::DocumentNode::List(_)
                | document::DocumentNode::ListItem(_)
                | document::DocumentNode::BlockQuote(_)
                | document::DocumentNode::Figure(_)
                | document::DocumentNode::Table(_)
                | document::DocumentNode::Footnote(_)
                | document::DocumentNode::Container(_)
                | document::DocumentNode::Unsupported(_) => {}
            }
        }
        if has_other_visible_content || image_ids.len() != 1 {
            return;
        }
        let Some(document::DocumentNode::Image(image)) = self.chapter.nodes.get_mut(image_ids[0])
        else {
            return;
        };
        if matches!(
            image.layout_role,
            document::ImageLayoutRole::Inline | document::ImageLayoutRole::Block
        ) {
            image.layout_role = document::ImageLayoutRole::Standalone;
        }
    }

    fn footnote_node(
        &mut self,
        tree_node_id: usize,
        element: &XhtmlElement,
        style: StyleId,
    ) -> Result<NodeId, PageletError> {
        let mut children = self.convert_flow_children(tree_node_id, style)?;
        if children.is_empty() {
            let text = self.inline_content(tree_node_id)?.text;
            if !text.is_empty() {
                let source_range = self
                    .tree
                    .node(tree_node_id)
                    .map(|node| node.source_range)
                    .unwrap_or_default();
                children.push(self.text_block_node(
                    DocumentNodeKind::Paragraph,
                    &text,
                    source_range,
                    style,
                )?);
            }
        }
        self.push_node(
            document::DocumentNode::Footnote(document::FootnoteNode {
                note_id: xhtml_element_id(element).map(Arc::from),
                children,
                backlink: None,
                style,
            }),
            self.tree.node(tree_node_id).map(|node| node.source_range),
        )
    }

    fn footnote_is_referenced(&self, element: &XhtmlElement) -> bool {
        xhtml_element_id(element).is_some_and(|fragment| {
            self.referenced_footnote_keys
                .contains(&format!("{}#{fragment}", self.document_href))
        })
    }

    fn style_for_node(&mut self, tree_node_id: usize) -> Result<StyleId, PageletError> {
        if let Some((id, _)) = self.computed_styles.get(&tree_node_id) {
            return Ok(*id);
        }
        let Some(element) = self.tree.node(tree_node_id).and_then(XhtmlNode::element) else {
            return Ok(self.default_style);
        };

        let parent = self.parent_node(tree_node_id);
        let (inherited, parent_font) = if let Some(parent) = parent {
            let parent_id = self.style_for_node(parent)?;
            (
                self.chapter
                    .styles
                    .get(parent_id)
                    .cloned()
                    .unwrap_or_default(),
                self.font_contexts.get(&parent).copied().unwrap_or_default(),
            )
        } else {
            (
                document::ComputedStyle::new(),
                ResolvedFontContext::default(),
            )
        };
        let snapshot = self.snapshot_for_element(element);
        let ancestors = self.ancestor_snapshots(tree_node_id);
        let authored = cascade_css_for_element(
            &snapshot,
            &ancestors,
            &self.css,
            &document::ComputedStyle::new(),
        );
        let mut cascade_base = inherited.clone();
        apply_inline_user_agent_defaults(element.local_name(), &mut cascade_base);
        let mut style = cascade_css_for_element(&snapshot, &ancestors, &self.css, &cascade_base);
        if let Some(locale) = element
            .attr("xml:lang")
            .or_else(|| element.attr("lang"))
            .filter(|locale| !locale.trim().is_empty())
            .map(Arc::<str>::from)
            .or_else(|| inherited.properties.get("-pagelet-locale").cloned())
        {
            style
                .properties
                .insert(Arc::from("-pagelet-locale"), locale);
        }
        if !authored.properties.contains_key("direction") {
            if let Some(direction) = element
                .attr("dir")
                .filter(|direction| matches!(*direction, "ltr" | "rtl" | "auto"))
            {
                style
                    .properties
                    .insert(Arc::from("direction"), Arc::from(direction));
            }
        }
        let root_font_size = if element.local_name() == "html" {
            LayoutUnit::from_px(16)
        } else {
            self.find_first_element(self.tree.root, "html")
                .and_then(|root| self.font_contexts.get(&root))
                .map_or(LayoutUnit::from_px(16), |context| context.font_size)
        };
        let font_context = resolve_font_metrics(
            element.local_name(),
            &mut style,
            &authored,
            &inherited,
            parent_font,
            root_font_size,
        );
        resolve_font_relative_geometry(&mut style, font_context.font_size, root_font_size);
        let style_id = self.chapter.styles.intern(style.clone())?;
        self.computed_styles.insert(tree_node_id, (style_id, style));
        self.font_contexts.insert(tree_node_id, font_context);
        Ok(style_id)
    }

    fn anonymous_block_style(&mut self, parent_style: StyleId) -> Result<StyleId, PageletError> {
        let mut style = document::ComputedStyle::new();
        if let Some(parent) = self.chapter.styles.get(parent_style) {
            for (name, value) in &parent.properties {
                if is_inherited_css_property(name) || name.as_ref() == "-pagelet-locale" {
                    style.properties.insert(name.clone(), value.clone());
                }
            }
        }
        self.chapter.styles.intern(style)
    }

    fn snapshot_for_element(&self, element: &XhtmlElement) -> CssElementSnapshot {
        CssElementSnapshot {
            name: element.local_name().to_ascii_lowercase(),
            id: element.attr("id").map(ToOwned::to_owned),
            classes: element
                .attr("class")
                .unwrap_or_default()
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect(),
            inline_style: element.attr("style").map(ToOwned::to_owned),
        }
    }

    fn ancestor_snapshots(&self, tree_node_id: usize) -> Vec<CssElementSnapshot> {
        let mut ancestors = Vec::new();
        let mut current = tree_node_id;
        while let Some(parent) = self.parent_node(current) {
            if let Some(element) = self.tree.node(parent).and_then(XhtmlNode::element) {
                ancestors.push(self.snapshot_for_element(element));
            }
            current = parent;
        }
        ancestors.reverse();
        ancestors
    }

    fn parent_node(&self, child_id: usize) -> Option<usize> {
        self.parent_node_from(self.tree.root, child_id)
    }

    fn parent_node_from(&self, current: usize, child_id: usize) -> Option<usize> {
        let element = self.tree.node(current)?.element()?;
        if element.children.contains(&child_id) {
            return Some(current);
        }
        element
            .children
            .iter()
            .find_map(|child| self.parent_node_from(*child, child_id))
    }

    fn resolve_footnotes(&mut self) -> Result<(), PageletError> {
        let links = self
            .chapter
            .links
            .iter()
            .filter(|link| link.kind == document::LinkKind::Footnote)
            .cloned()
            .collect::<Vec<_>>();
        for link in links {
            let Some(fragment) = link.fragment.as_deref() else {
                continue;
            };
            if let Some(note_id) = self.find_footnote_by_fragment(fragment) {
                self.set_footnote_backlink(note_id, link);
                continue;
            }
            if link.resolved_document.as_deref() != Some(self.document_href) {
                self.add_external_footnote(&link)?;
            }
        }
        Ok(())
    }

    fn find_footnote_by_fragment(&self, fragment: &str) -> Option<NodeId> {
        self.chapter
            .nodes
            .iter_with_ids()
            .find_map(|(node_id, node)| {
                if let document::DocumentNode::Footnote(note) = node {
                    if note.note_id.as_deref() == Some(fragment) {
                        return Some(node_id);
                    }
                }
                None
            })
    }

    fn set_footnote_backlink(&mut self, node_id: NodeId, link: document::LinkTarget) {
        if let Some(document::DocumentNode::Footnote(note)) = self.chapter.nodes.get_mut(node_id) {
            note.backlink = Some(link);
        }
    }

    fn add_external_footnote(&mut self, link: &document::LinkTarget) -> Result<(), PageletError> {
        let Some(store) = self.store else {
            return Ok(());
        };
        let Some(document_href) = link.resolved_document.as_deref() else {
            return Ok(());
        };
        let Some(fragment) = link.fragment.as_deref() else {
            return Ok(());
        };
        let bytes = store.read_path(document_href, self.options.compatibility_mode)?;
        let text = resource_text(&bytes)?;
        let tree = parse_xhtml_tree(&text, CompatibilityMode::Compatible, self.options.limits)?;
        let Some(note_tree_id) = find_xhtml_element_by_id(&tree, tree.root, fragment) else {
            return Ok(());
        };
        let mut note_text = String::new();
        collect_visible_xhtml_text(&tree, note_tree_id, &mut note_text);
        let note_text = note_text.trim();
        if note_text.is_empty() {
            return Ok(());
        }
        let text_range = self.chapter.text_pool.push(note_text)?;
        let paragraph =
            self.chapter
                .nodes
                .push(document::DocumentNode::Paragraph(document::BlockText {
                    text: text_range,
                    style: self.default_style,
                    style_runs: Vec::new(),
                }))?;
        let footnote =
            self.chapter
                .nodes
                .push(document::DocumentNode::Footnote(document::FootnoteNode {
                    note_id: Some(Arc::from(fragment)),
                    children: vec![paragraph],
                    backlink: Some(link.clone()),
                    style: self.default_style,
                }))?;
        if let Some(document::DocumentNode::Container(root)) =
            self.chapter.nodes.get_mut(self.chapter.root)
        {
            root.children.push(footnote);
        }
        Ok(())
    }

    fn push_node(
        &mut self,
        node: document::DocumentNode,
        source_range: Option<SourceRange>,
    ) -> Result<NodeId, PageletError> {
        let node_id = self.chapter.nodes.push(node)?;
        self.chapter.source_map.insert(node_id, source_range);
        Ok(node_id)
    }

    fn inline_content(&mut self, node_id: usize) -> Result<InlineContent, PageletError> {
        let style = self.style_for_node(node_id)?;
        self.inline_content_for_nodes(&[node_id], style)
    }

    fn inline_content_for_nodes(
        &mut self,
        nodes: &[usize],
        inherited_style: StyleId,
    ) -> Result<InlineContent, PageletError> {
        let mut accumulator = InlineAccumulator::default();
        for node_id in nodes {
            self.collect_inline(*node_id, inherited_style, &mut accumulator)?;
        }
        Ok(accumulator.finish())
    }

    fn collect_inline(
        &mut self,
        node_id: usize,
        inherited_style: StyleId,
        accumulator: &mut InlineAccumulator,
    ) -> Result<(), PageletError> {
        let Some(node) = self.tree.node(node_id) else {
            return Ok(());
        };
        let source_range = node.source_range;
        let kind = node.kind.clone();
        match kind {
            XhtmlNodeKind::Text(value) => {
                accumulator.append_collapsible(&value, inherited_style, source_range);
            }
            XhtmlNodeKind::Element(element) => {
                let style = self.style_for_node(node_id)?;
                if let Some(id) = xhtml_element_id(&element) {
                    accumulator.anchors.push(AnchorDraft {
                        fragment: Arc::from(id),
                        utf8_byte_offset: accumulator.next_text_offset(),
                        source_range: Some(source_range),
                    });
                }
                match element.local_name() {
                    "br" => accumulator.append_break(style, source_range),
                    "img" => {
                        if let Some(alt) = element.attr("alt") {
                            accumulator.append_collapsible(alt, style, source_range);
                        }
                    }
                    "a" => {
                        accumulator.flush_pending_space();
                        let before = accumulator.text.len();
                        for child in &element.children {
                            self.collect_inline(*child, style, accumulator)?;
                        }
                        if let Some(href) = element.attr("href") {
                            if accumulator.text.len() == before {
                                accumulator.append_collapsible(href, style, source_range);
                            }
                            accumulator.links.push(LinkDraft {
                                href: Arc::from(href),
                                text_range: Some(
                                    u32::try_from(before).unwrap_or(u32::MAX)
                                        ..u32::try_from(accumulator.text.len()).unwrap_or(u32::MAX),
                                ),
                                source_range: Some(source_range),
                                kind: if is_noteref_element(&element) {
                                    document::LinkKind::Footnote
                                } else {
                                    document::LinkKind::Internal
                                },
                            });
                        }
                    }
                    _ => {
                        for child in &element.children {
                            self.collect_inline(*child, style, accumulator)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn add_inline_metadata(
        &mut self,
        node_id: NodeId,
        links: Vec<LinkDraft>,
        anchors: Vec<AnchorDraft>,
    ) {
        for link in links {
            self.chapter.links.push(resolve_link(
                self.document_href,
                &self.base_dir,
                node_id,
                link,
                self.resources,
            ));
        }
        for anchor in anchors {
            self.chapter.anchors.insert(document::Anchor {
                key: Arc::from(format!("{}#{}", self.document_href, anchor.fragment)),
                document_href: Arc::from(self.document_href),
                fragment: anchor.fragment,
                node_id,
                utf8_byte_offset: anchor.utf8_byte_offset,
                source_range: anchor.source_range,
            });
        }
    }

    fn register_element_anchor(
        &mut self,
        element: &XhtmlElement,
        node_id: NodeId,
        source_range: Option<SourceRange>,
    ) {
        if let Some(fragment) = xhtml_element_id(element) {
            self.chapter.anchors.insert(document::Anchor {
                key: Arc::from(format!("{}#{}", self.document_href, fragment)),
                document_href: Arc::from(self.document_href),
                fragment: Arc::from(fragment),
                node_id,
                utf8_byte_offset: 0,
                source_range,
            });
        }
    }
}

fn resolve_font_metrics(
    element_name: &str,
    style: &mut document::ComputedStyle,
    authored: &document::ComputedStyle,
    inherited: &document::ComputedStyle,
    parent: ResolvedFontContext,
    root_font_size: LayoutUnit,
) -> ResolvedFontContext {
    let authored_font_size = authored.properties.get("font-size").map(AsRef::as_ref);
    let inherits_font_size = inherited.properties.contains_key("font-size");
    let font_size = authored_font_size
        .and_then(|value| resolve_font_size(value, parent.font_size, root_font_size))
        .unwrap_or_else(|| {
            if inherits_font_size {
                parent.font_size
            } else {
                default_font_size_for_element(element_name)
            }
        });
    if authored_font_size.is_some() || inherits_font_size {
        style
            .properties
            .insert(Arc::from("font-size"), Arc::from(format_css_px(font_size)));
    }

    let authored_line_height = authored.properties.get("line-height").map(AsRef::as_ref);
    let inherits_line_height = inherited.properties.contains_key("line-height");
    let line_height = if let Some(value) = authored_line_height {
        resolve_line_height(value, font_size, root_font_size, parent.line_height).unwrap_or({
            if inherits_line_height {
                parent.line_height
            } else {
                ResolvedLineHeight::Normal
            }
        })
    } else if inherits_line_height {
        parent.line_height
    } else {
        ResolvedLineHeight::Normal
    };
    if authored_line_height.is_some() || inherits_line_height {
        let value = match line_height {
            ResolvedLineHeight::Normal => Arc::from("normal"),
            ResolvedLineHeight::Absolute(value) => Arc::from(format_css_px(value)),
            ResolvedLineHeight::Multiplier(multiplier) => {
                Arc::from(format_css_px(scale_layout_unit(font_size, multiplier)))
            }
        };
        style.properties.insert(Arc::from("line-height"), value);
    }

    ResolvedFontContext {
        font_size,
        line_height,
    }
}

fn resolve_font_relative_geometry(
    style: &mut document::ComputedStyle,
    font_size: LayoutUnit,
    root_font_size: LayoutUnit,
) {
    const GEOMETRY_PROPERTIES: [&str; 10] = [
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "text-indent",
        "letter-spacing",
    ];
    for property in GEOMETRY_PROPERTIES {
        let Some(value) = style.properties.get(property).cloned() else {
            continue;
        };
        let Some(resolved) = resolve_font_relative_geometry_length(
            &value,
            font_size,
            root_font_size,
            !property.starts_with("padding"),
        ) else {
            continue;
        };
        style
            .properties
            .insert(Arc::from(property), Arc::from(format_css_px(resolved)));
    }
}

fn resolve_font_relative_geometry_length(
    value: &str,
    em_base: LayoutUnit,
    rem_base: LayoutUnit,
    allow_negative: bool,
) -> Option<LayoutUnit> {
    let value = value.trim().to_ascii_lowercase();
    let parsed = if let Some(number) = value.strip_suffix("rem") {
        parse_signed_css_number_value(number).map(|factor| scale_layout_unit(rem_base, factor))
    } else if let Some(number) = value.strip_suffix("em") {
        parse_signed_css_number_value(number).map(|factor| scale_layout_unit(em_base, factor))
    } else if let Some(number) = value.strip_suffix("px") {
        parse_signed_css_number_value(number).map(LayoutUnit::from_f64_px)
    } else if value == "0" {
        Some(LayoutUnit::ZERO)
    } else {
        None
    }?;
    (allow_negative || parsed.raw() >= 0).then_some(parsed)
}

fn resolve_font_size(
    value: &str,
    parent_font_size: LayoutUnit,
    root_font_size: LayoutUnit,
) -> Option<LayoutUnit> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "inherit" | "unset" => Some(parent_font_size),
        "initial" | "medium" => Some(LayoutUnit::from_px(16)),
        "xx-small" => Some(LayoutUnit::from_px(9)),
        "x-small" => Some(LayoutUnit::from_px(10)),
        "small" => Some(LayoutUnit::from_px(13)),
        "large" => Some(LayoutUnit::from_px(18)),
        "x-large" => Some(LayoutUnit::from_px(24)),
        "xx-large" => Some(LayoutUnit::from_px(32)),
        "xxx-large" => Some(LayoutUnit::from_px(48)),
        "smaller" => Some(scale_layout_unit(parent_font_size, 0.8)),
        "larger" => Some(scale_layout_unit(parent_font_size, 1.2)),
        _ => parse_css_length(&value, parent_font_size, root_font_size, parent_font_size),
    }
}

fn resolve_line_height(
    value: &str,
    font_size: LayoutUnit,
    root_font_size: LayoutUnit,
    parent_line_height: ResolvedLineHeight,
) -> Option<ResolvedLineHeight> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "normal" | "initial" => Some(ResolvedLineHeight::Normal),
        "inherit" | "unset" => Some(parent_line_height),
        _ => {
            if let Some(multiplier) = parse_css_number(&value) {
                return Some(ResolvedLineHeight::Multiplier(multiplier));
            }
            parse_css_length(&value, font_size, root_font_size, font_size)
                .map(ResolvedLineHeight::Absolute)
        }
    }
}

fn parse_css_length(
    value: &str,
    em_base: LayoutUnit,
    rem_base: LayoutUnit,
    percent_base: LayoutUnit,
) -> Option<LayoutUnit> {
    if let Some(number) = value.strip_suffix("rem") {
        return parse_css_number_value(number).map(|factor| scale_layout_unit(rem_base, factor));
    }
    if let Some(number) = value.strip_suffix("em") {
        return parse_css_number_value(number).map(|factor| scale_layout_unit(em_base, factor));
    }
    if let Some(number) = value.strip_suffix('%') {
        return parse_css_number_value(number)
            .map(|percent| scale_layout_unit(percent_base, percent / 100.0));
    }
    if let Some(number) = value.strip_suffix("px") {
        return parse_css_number_value(number).map(LayoutUnit::from_f64_px);
    }
    (value.trim() == "0").then_some(LayoutUnit::ZERO)
}

fn parse_css_number(value: &str) -> Option<f64> {
    if value.ends_with(|ch: char| ch.is_ascii_alphabetic() || ch == '%') {
        return None;
    }
    parse_css_number_value(value)
}

fn parse_css_number_value(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn parse_signed_css_number_value(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn scale_layout_unit(value: LayoutUnit, factor: f64) -> LayoutUnit {
    LayoutUnit::from_f64_px(value.to_f64_px() * factor)
}

fn default_font_size_for_element(element_name: &str) -> LayoutUnit {
    let pixels = match element_name {
        "h1" => 30,
        "h2" => 27,
        "h3" => 24,
        "h4" => 21,
        "h5" | "h6" => 18,
        _ => 16,
    };
    LayoutUnit::from_px(pixels)
}

fn format_css_px(value: LayoutUnit) -> String {
    if value.raw() % LayoutUnit::SCALE == 0 {
        return format!("{}px", value.raw() / LayoutUnit::SCALE);
    }
    let mut number = format!("{:.6}", value.to_f64_px());
    while number.ends_with('0') {
        number.pop();
    }
    if number.ends_with('.') {
        number.pop();
    }
    format!("{number}px")
}

#[derive(Debug, Clone, Copy)]
enum DocumentNodeKind {
    Paragraph,
}

#[derive(Debug, Clone)]
struct LinkDraft {
    href: Arc<str>,
    text_range: Option<std::ops::Range<u32>>,
    source_range: Option<SourceRange>,
    kind: document::LinkKind,
}

#[derive(Debug, Clone)]
struct AnchorDraft {
    fragment: Arc<str>,
    utf8_byte_offset: u32,
    source_range: Option<SourceRange>,
}

#[derive(Debug, Default)]
struct InlineContent {
    text: String,
    style_runs: Vec<document::InlineStyleRun>,
    links: Vec<LinkDraft>,
    anchors: Vec<AnchorDraft>,
}

#[derive(Debug, Default)]
struct InlineAccumulator {
    text: String,
    style_runs: Vec<document::InlineStyleRun>,
    links: Vec<LinkDraft>,
    anchors: Vec<AnchorDraft>,
    pending_space: Option<(StyleId, SourceRange)>,
}

impl InlineAccumulator {
    fn append_collapsible(&mut self, value: &str, style: StyleId, source_range: SourceRange) {
        for character in value.chars() {
            if is_collapsible_xhtml_whitespace(character) {
                if !self.text.is_empty() && !self.text.ends_with('\n') {
                    self.pending_space.get_or_insert((style, source_range));
                }
                continue;
            }
            self.flush_pending_space();
            let mut buffer = [0_u8; 4];
            self.append_segment(character.encode_utf8(&mut buffer), style, source_range);
        }
    }

    fn append_break(&mut self, style: StyleId, source_range: SourceRange) {
        self.pending_space = None;
        self.append_segment("\n", style, source_range);
    }

    fn flush_pending_space(&mut self) {
        let Some((style, source_range)) = self.pending_space.take() else {
            return;
        };
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.append_segment(" ", style, source_range);
        }
    }

    fn append_segment(&mut self, value: &str, style: StyleId, source_range: SourceRange) {
        if value.is_empty() {
            return;
        }
        let start = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        self.text.push_str(value);
        let end = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        if let Some(last) = self.style_runs.last_mut() {
            if last.end == start && last.style == style && last.source_range == Some(source_range) {
                last.end = end;
                return;
            }
        }
        self.style_runs.push(document::InlineStyleRun {
            start,
            end,
            style,
            source_range: Some(source_range),
        });
    }

    fn next_text_offset(&self) -> u32 {
        let pending = u32::from(
            self.pending_space.is_some() && !self.text.is_empty() && !self.text.ends_with('\n'),
        );
        u32::try_from(self.text.len())
            .unwrap_or(u32::MAX)
            .saturating_add(pending)
    }

    fn finish(mut self) -> InlineContent {
        let text_end = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        for anchor in &mut self.anchors {
            anchor.utf8_byte_offset = anchor.utf8_byte_offset.min(text_end);
        }
        InlineContent {
            text: self.text,
            style_runs: self.style_runs,
            links: self.links,
            anchors: self.anchors,
        }
    }
}

fn is_collapsible_xhtml_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\n' | '\r' | '\t' | '\u{000c}')
}

fn resolve_link(
    document_href: &str,
    base_dir: &str,
    source_node: NodeId,
    draft: LinkDraft,
    resources: &document::ResourceTable,
) -> document::LinkTarget {
    let href_value = draft.href.clone();
    let href = href_value.as_ref();
    if href.starts_with("http:")
        || href.starts_with("https:")
        || href.starts_with("mailto:")
        || href.starts_with("data:")
    {
        return document::LinkTarget {
            source_node,
            text_range: draft.text_range,
            source_range: draft.source_range,
            href: href_value,
            resolved_document: None,
            fragment: None,
            kind: document::LinkKind::External,
        };
    }

    let (path, fragment) = href
        .split_once('#')
        .map_or((href, None), |(path, fragment)| {
            (path, Some(Arc::from(fragment)))
        });
    let resolved_document = if path.is_empty() {
        Some(Arc::from(document_href))
    } else {
        resolve_resource_path(base_dir, path).ok().map(Arc::from)
    };
    let kind = if draft.kind == document::LinkKind::Footnote {
        document::LinkKind::Footnote
    } else if resolved_document
        .as_deref()
        .and_then(|path| resources.id_for_path(path))
        .is_some()
        && !resolved_document
            .as_deref()
            .is_some_and(|path| path.ends_with(".xhtml") || path.ends_with(".html"))
    {
        document::LinkKind::Resource
    } else if resolved_document.is_some() {
        document::LinkKind::Internal
    } else {
        document::LinkKind::Unknown
    };

    document::LinkTarget {
        source_node,
        text_range: draft.text_range,
        source_range: draft.source_range,
        href: href_value,
        resolved_document,
        fragment,
        kind,
    }
}

fn is_footnote_element(element: &XhtmlElement) -> bool {
    element
        .attr("epub:type")
        .or_else(|| element.attr("role"))
        .is_some_and(|value| value.contains("footnote") || value.contains("endnote"))
        || element
            .attr("class")
            .is_some_and(|value| value.contains("footnote") || value.contains("endnote"))
}

fn is_noteref_element(element: &XhtmlElement) -> bool {
    element
        .attr("epub:type")
        .or_else(|| element.attr("role"))
        .is_some_and(|value| value.contains("noteref"))
}

fn xhtml_element_id(element: &XhtmlElement) -> Option<&str> {
    element.attr("id").or_else(|| element.attr("xml:id"))
}

fn collect_referenced_footnote_keys(
    tree: &XhtmlDocument,
    node_id: usize,
    document_href: &str,
    base_dir: &str,
    out: &mut BTreeSet<String>,
) {
    let Some(node) = tree.node(node_id) else {
        return;
    };
    let Some(element) = node.element() else {
        return;
    };
    if element.local_name() == "a" && is_noteref_element(element) {
        if let Some(href) = element.attr("href") {
            if let Some((resolved_document, fragment)) =
                resolve_footnote_href(document_href, base_dir, href)
            {
                out.insert(format!("{resolved_document}#{fragment}"));
            }
        }
    }
    for child in &element.children {
        collect_referenced_footnote_keys(tree, *child, document_href, base_dir, out);
    }
}

fn resolve_footnote_href(
    document_href: &str,
    base_dir: &str,
    href: &str,
) -> Option<(String, String)> {
    let (path, fragment) = href.split_once('#')?;
    if fragment.is_empty() {
        return None;
    }
    let resolved_document = if path.is_empty() {
        document_href.to_owned()
    } else {
        resolve_resource_path(base_dir, path).ok()?
    };
    Some((resolved_document, fragment.to_owned()))
}

fn find_xhtml_element_by_id(tree: &XhtmlDocument, node_id: usize, id: &str) -> Option<usize> {
    let node = tree.node(node_id)?;
    let element = node.element()?;
    if xhtml_element_id(element) == Some(id) {
        return Some(node_id);
    }
    element
        .children
        .iter()
        .find_map(|child| find_xhtml_element_by_id(tree, *child, id))
}

fn collect_visible_xhtml_text(tree: &XhtmlDocument, node_id: usize, out: &mut String) {
    let Some(node) = tree.node(node_id) else {
        return;
    };
    match &node.kind {
        XhtmlNodeKind::Text(text) => {
            let text = text.trim();
            if text.is_empty() {
                return;
            }
            if !out.is_empty() && !out.chars().last().is_some_and(char::is_whitespace) {
                out.push(' ');
            }
            out.push_str(text);
        }
        XhtmlNodeKind::Element(element) => {
            if matches!(
                element.local_name(),
                "head" | "script" | "style" | "title" | "meta" | "link"
            ) {
                return;
            }
            for child in &element.children {
                collect_visible_xhtml_text(tree, *child, out);
            }
        }
    }
}

fn parse_container(
    store: &ZipPublicationStore,
    options: OpenOptions,
    diagnostics: &mut DiagnosticCollector,
) -> Result<Vec<Rootfile>, PageletError> {
    let bytes = store.read_path("META-INF/container.xml", options.compatibility_mode)?;
    let text = resource_text(&bytes)?;
    let rootfiles = parse_container_xml(&text)?;
    if rootfiles.len() > 1 {
        diagnostics.push_warning(
            DiagnosticCode::InvalidContainer,
            "multiple rootfiles found; selecting first OPF package rootfile",
        )?;
    }
    Ok(rootfiles)
}

fn choose_rootfile(
    rootfiles: &[Rootfile],
    mode: CompatibilityMode,
) -> Result<&Rootfile, PageletError> {
    rootfiles
        .iter()
        .find(|rootfile| {
            rootfile.media_type == "application/oebps-package+xml"
                || rootfile.full_path.ends_with(".opf")
        })
        .or_else(|| {
            if mode == CompatibilityMode::Strict {
                None
            } else {
                rootfiles.first()
            }
        })
        .ok_or_else(|| invalid_container("no package rootfile found"))
}

fn parse_navigation(
    store: &ZipPublicationStore,
    options: OpenOptions,
    package: &PackageDocument,
    diagnostics: &mut DiagnosticCollector,
) -> Result<Navigation, PageletError> {
    if let Some(nav_item) = package.manifest.iter().find(|item| {
        item.properties.iter().any(|property| property == "nav")
            || item.id.eq_ignore_ascii_case("nav")
    }) {
        match store.read_path(&nav_item.resolved_path, options.compatibility_mode) {
            Ok(bytes) => return parse_epub3_nav(&resource_text(&bytes)?),
            Err(error) => diagnostics.push_warning(error.code(), error.to_string())?,
        }
    }

    if let Some(ncx_item) = package
        .spine_toc
        .as_ref()
        .and_then(|toc| package.manifest.iter().find(|item| item.id == *toc))
        .or_else(|| {
            package
                .manifest
                .iter()
                .find(|item| item.media_type == "application/x-dtbncx+xml")
        })
    {
        match store.read_path(&ncx_item.resolved_path, options.compatibility_mode) {
            Ok(bytes) => return parse_ncx(&resource_text(&bytes)?),
            Err(error) => diagnostics.push_warning(error.code(), error.to_string())?,
        }
    }

    if !package.guide.is_empty() {
        return Ok(Navigation {
            source: NavigationSource::Guide,
            toc: package.guide.clone(),
            page_list: Vec::new(),
            landmarks: package.guide.clone(),
        });
    }

    Ok(Navigation {
        source: NavigationSource::Spine,
        toc: spine_navigation(package),
        page_list: Vec::new(),
        landmarks: Vec::new(),
    })
}

fn parse_epub3_nav(input: &str) -> Result<Navigation, PageletError> {
    let mut navigation = Navigation {
        source: NavigationSource::Epub3Nav,
        toc: Vec::new(),
        page_list: Vec::new(),
        landmarks: Vec::new(),
    };
    for tag in scan_start_tags(input)
        .into_iter()
        .filter(|tag| tag.local_name() == "nav")
    {
        let nav_type = tag
            .attr("epub:type")
            .or_else(|| tag.attr("type"))
            .unwrap_or("toc");
        let section = input
            .get(tag.end..)
            .and_then(|rest| rest.split("</nav>").next())
            .unwrap_or_default();
        let links = parse_links(section)?;
        match nav_type {
            "page-list" => navigation.page_list = links,
            "landmarks" => navigation.landmarks = links,
            _ => navigation.toc = links,
        }
    }
    if navigation.toc.is_empty() {
        navigation.toc = parse_links(input)?;
    }
    Ok(navigation)
}

fn parse_ncx(input: &str) -> Result<Navigation, PageletError> {
    let mut toc = Vec::new();
    for point in input.split("<navPoint").skip(1) {
        let point = point.split("</navPoint>").next().unwrap_or(point);
        let label = text_of_first(point, &["text"]).unwrap_or_else(|| "Untitled".into());
        let href = scan_start_tags(point)
            .into_iter()
            .find(|tag| tag.local_name() == "content")
            .and_then(|tag| tag.attr("src").map(ToOwned::to_owned))
            .unwrap_or_default();
        if !href.is_empty() {
            toc.push(NavigationItem {
                label,
                href,
                children: Vec::new(),
            });
        }
    }
    Ok(Navigation {
        source: NavigationSource::Ncx,
        toc,
        page_list: Vec::new(),
        landmarks: Vec::new(),
    })
}

fn parse_guide(input: &str, package_dir: &str) -> Result<Vec<NavigationItem>, PageletError> {
    let mut guide = Vec::new();
    for tag in scan_start_tags(input)
        .into_iter()
        .filter(|tag| tag.local_name() == "reference")
    {
        let href = tag.attr("href").unwrap_or_default();
        if href.is_empty() {
            continue;
        }
        guide.push(NavigationItem {
            label: tag
                .attr("title")
                .or_else(|| tag.attr("type"))
                .unwrap_or("Guide")
                .to_owned(),
            href: resolve_resource_path(package_dir, href)?,
            children: Vec::new(),
        });
    }
    Ok(guide)
}

fn parse_links(input: &str) -> Result<Vec<NavigationItem>, PageletError> {
    let mut links = Vec::new();
    for tag in scan_start_tags(input)
        .into_iter()
        .filter(|tag| tag.local_name() == "a")
    {
        let href = tag.attr("href").unwrap_or_default().to_owned();
        if href.is_empty() {
            continue;
        }
        let label = input
            .get(tag.end..)
            .and_then(|rest| rest.split("</a>").next())
            .map(strip_tags)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| href.clone());
        links.push(NavigationItem {
            label,
            href,
            children: Vec::new(),
        });
    }
    Ok(links)
}

fn spine_navigation(package: &PackageDocument) -> Vec<NavigationItem> {
    package
        .spine
        .iter()
        .filter_map(|spine| {
            package
                .manifest
                .iter()
                .find(|item| item.id == spine.idref)
                .map(|item| NavigationItem {
                    label: item.id.clone(),
                    href: item.resolved_path.clone(),
                    children: Vec::new(),
                })
        })
        .collect()
}

fn index_zip_entries(bytes: &[u8], options: OpenOptions) -> Result<Vec<ZipEntry>, PageletError> {
    let eocd = find_eocd(bytes)?;
    let entry_count = read_u16(bytes, eocd + 10)? as usize;
    let central_size = read_u32(bytes, eocd + 12)? as usize;
    let central_offset = read_u32(bytes, eocd + 16)? as usize;
    let central_end = central_offset
        .checked_add(central_size)
        .ok_or_else(|| invalid_container("central directory range overflows"))?;
    if central_end > bytes.len() {
        return Err(invalid_container("central directory is outside archive"));
    }

    let mut entries = Vec::with_capacity(entry_count);
    let mut cursor = central_offset;
    let mut records_seen = 0_usize;
    while cursor < central_end {
        if read_u32(bytes, cursor)? != 0x0201_4b50 {
            return Err(invalid_container("invalid central directory header"));
        }
        records_seen = records_seen.saturating_add(1);
        let compression_method = read_u16(bytes, cursor + 10)?;
        let compressed_size = u64::from(read_u32(bytes, cursor + 20)?);
        let uncompressed_size = u64::from(read_u32(bytes, cursor + 24)?);
        let name_len = read_u16(bytes, cursor + 28)? as usize;
        let extra_len = read_u16(bytes, cursor + 30)? as usize;
        let comment_len = read_u16(bytes, cursor + 32)? as usize;
        let local_offset = read_u32(bytes, cursor + 42)? as usize;
        let name_start = cursor + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| invalid_container("ZIP entry name range overflows"))?;
        let path = std::str::from_utf8(
            bytes
                .get(name_start..name_end)
                .ok_or_else(|| invalid_container("ZIP entry name is outside archive"))?,
        )
        .map_err(|_| invalid_container("ZIP entry path is not UTF-8"))?
        .to_owned();
        let data_offset = local_data_offset(bytes, local_offset)?;
        if !path.ends_with('/') {
            entries.push(ZipEntry {
                path,
                compression_method,
                compressed_size,
                uncompressed_size,
                data_offset,
            });
        }
        cursor = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| invalid_container("central directory cursor overflows"))?;
    }

    if records_seen != entry_count && options.compatibility_mode == CompatibilityMode::Strict {
        return Err(invalid_container("central directory entry count mismatch"));
    }
    Ok(entries)
}

fn find_eocd(bytes: &[u8]) -> Result<usize, PageletError> {
    let min = bytes.len().saturating_sub(65_557);
    for offset in (min..bytes.len().saturating_sub(3)).rev() {
        if bytes.get(offset..offset + 4) == Some(&[0x50, 0x4b, 0x05, 0x06]) {
            return Ok(offset);
        }
    }
    Err(invalid_container("ZIP end of central directory not found"))
}

fn local_data_offset(bytes: &[u8], local_offset: usize) -> Result<usize, PageletError> {
    if read_u32(bytes, local_offset)? != 0x0403_4b50 {
        return Err(invalid_container("invalid local file header"));
    }
    let name_len = read_u16(bytes, local_offset + 26)? as usize;
    let extra_len = read_u16(bytes, local_offset + 28)? as usize;
    local_offset
        .checked_add(30)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len))
        .ok_or_else(|| invalid_container("local file data offset overflows"))
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ZipEntry {
    path: String,
    compression_method: u16,
    compressed_size: u64,
    uncompressed_size: u64,
    data_offset: usize,
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, PageletError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_container("unexpected end of ZIP data"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, PageletError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_container("unexpected end of ZIP data"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let bytes = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let bytes = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let bytes = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn validate_container_path(path: &str, mode: CompatibilityMode) -> Result<(), PageletError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(invalid_container(format!("unsafe ZIP path: {path}")));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid_container(format!("unsafe ZIP path: {path}")));
        }
    }
    if path.contains('%') && mode == CompatibilityMode::Strict {
        return Err(invalid_container(format!(
            "percent-encoded ZIP paths require compatible mode: {path}"
        )));
    }
    Ok(())
}

fn media_type_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "opf" => "application/oebps-package+xml",
        "xhtml" | "html" => "application/xhtml+xml",
        "ncx" => "application/x-dtbncx+xml",
        "css" => "text/css",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

fn resource_text(bytes: &ResourceBytes) -> Result<String, PageletError> {
    String::from_utf8(bytes.bytes.clone())
        .map_err(|_| PageletError::Parse(ParseError::new(format!("{} is not UTF-8", bytes.path))))
}

fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default()
}

fn split_properties(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|property| !property.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn identifier_for(input: &str, unique_identifier: &str) -> Option<String> {
    if !unique_identifier.is_empty() {
        for tag in scan_start_tags(input)
            .into_iter()
            .filter(|tag| tag.local_name() == "identifier")
        {
            if tag.attr("id") == Some(unique_identifier) {
                return text_after_tag(input, &tag, "identifier");
            }
        }
    }
    text_of_first(input, &["dc:identifier", "identifier"])
}

fn text_of_first(input: &str, names: &[&str]) -> Option<String> {
    let tags = scan_start_tags(input);
    for name in names {
        for tag in tags
            .iter()
            .filter(|tag| tag.name == *name || tag.local_name() == *name)
        {
            if let Some(text) = text_after_tag(input, tag, tag.local_name()) {
                return Some(text);
            }
        }
    }
    None
}

fn text_after_tag(input: &str, tag: &XmlStartTag, local_name: &str) -> Option<String> {
    let rest = input.get(tag.end..)?;
    let end_tag = format!("</{local_name}>");
    let prefixed_end_tag = format!("</{}>", tag.name);
    let raw = rest
        .split(&prefixed_end_tag)
        .next()
        .and_then(|candidate| {
            if candidate.len() < rest.len() {
                Some(candidate)
            } else {
                None
            }
        })
        .or_else(|| rest.split(&end_tag).next())?;
    Some(unescape_xml(&strip_tags(raw)))
}

fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    unescape_xml(out.trim())
}

fn unescape_xml(input: &str) -> String {
    input
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct XmlStartTag {
    name: String,
    attrs: BTreeMap<String, String>,
    end: usize,
}

impl XmlStartTag {
    fn local_name(&self) -> &str {
        self.name
            .rsplit_once(':')
            .map_or(&self.name, |(_, local)| local)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
}

fn scan_start_tags(input: &str) -> Vec<XmlStartTag> {
    let mut tags = Vec::new();
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while let Some(relative) = input[cursor..].find('<') {
        let start = cursor + relative;
        let Some(next) = bytes.get(start + 1) else {
            break;
        };
        if matches!(*next, b'/' | b'!' | b'?') {
            cursor = start + 1;
            continue;
        }
        let Some(close_relative) = input[start..].find('>') else {
            break;
        };
        let close = start + close_relative;
        let inside = input[start + 1..close].trim().trim_end_matches('/').trim();
        if inside.is_empty() {
            cursor = close + 1;
            continue;
        }
        let name_end = inside
            .find(|ch: char| ch.is_whitespace())
            .unwrap_or(inside.len());
        let name = inside[..name_end].to_owned();
        let attrs = parse_attrs(&inside[name_end..]);
        tags.push(XmlStartTag {
            name,
            attrs,
            end: close + 1,
        });
        cursor = close + 1;
    }
    tags
}

fn parse_attrs(input: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while cursor < input.len() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let key_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        {
            cursor += 1;
        }
        if key_start == cursor {
            break;
        }
        let key = input[key_start..cursor].trim();
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let Some(quote) = bytes.get(cursor).copied() else {
            break;
        };
        if quote != b'"' && quote != b'\'' {
            break;
        }
        cursor += 1;
        let value_start = cursor;
        while bytes.get(cursor).is_some_and(|byte| *byte != quote) {
            cursor += 1;
        }
        if cursor > value_start {
            attrs.insert(key.to_owned(), unescape_xml(&input[value_start..cursor]));
        } else {
            attrs.insert(key.to_owned(), String::new());
        }
        cursor += 1;
    }
    attrs
}

#[derive(Debug)]
struct DiagnosticCollector {
    max: u32,
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticCollector {
    fn new(max: u32) -> Self {
        Self {
            max,
            diagnostics: Vec::new(),
        }
    }

    fn push_warning(
        &mut self,
        code: DiagnosticCode,
        message: impl Into<Arc<str>>,
    ) -> Result<(), PageletError> {
        self.push(Diagnostic::new(code, Severity::Warning, message))
    }

    fn push_error(
        &mut self,
        code: DiagnosticCode,
        message: impl Into<Arc<str>>,
    ) -> Result<(), PageletError> {
        self.push(Diagnostic::new(code, Severity::Error, message))
    }

    fn push(&mut self, diagnostic: Diagnostic) -> Result<(), PageletError> {
        let next = self.diagnostics.len().saturating_add(1);
        if next > self.max as usize {
            return Err(limit_error(
                ResourceLimitKind::Diagnostics,
                u64::from(self.max),
                u64::try_from(next).unwrap_or(u64::MAX),
            ));
        }
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    fn into_vec(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

fn invalid_container(message: impl Into<Arc<str>>) -> PageletError {
    PageletError::InvalidContainer(crate::core::ContainerError::new(message))
}

fn invalid_package(message: impl Into<Arc<str>>) -> PageletError {
    PageletError::InvalidPackage(PackageError::new(message))
}

fn limit_error(kind: ResourceLimitKind, limit: u64, observed: u64) -> PageletError {
    PageletError::ResourceLimitExceeded(ResourceLimitError::new(kind, limit, observed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_text_line_rects(page: &crate::layout::PageScene) -> Vec<crate::layout::Rect> {
        let mut rects = Vec::new();
        for paint in &page.text_paints {
            let paragraph = page
                .paragraphs
                .iter()
                .find(|paragraph| paragraph.paragraph_id == paint.paragraph_id)
                .expect("paint paragraph");
            let first = usize::try_from(paint.first_line).expect("first line");
            let end = first + usize::try_from(paint.line_count).expect("line count");
            rects.extend(
                paragraph.lines[first..end]
                    .iter()
                    .map(|line| crate::layout::Rect {
                        x: paint.paint_origin.x + line.layout_rect.x,
                        y: paint.paint_origin.y + line.layout_rect.y,
                        width: line.layout_rect.width,
                        height: line.layout_rect.height,
                    }),
            );
        }
        rects
    }

    fn minimal_fixture_bytes() -> Vec<u8> {
        crate::testkit::GeneratedEpubFixture::preset(crate::testkit::FixtureKind::MinimalEpub3)
            .bytes()
            .to_vec()
    }

    #[test]
    fn open_book_reads_metadata_manifest_spine_and_nav() {
        let book = open_book(minimal_fixture_bytes()).expect("open book");

        assert_eq!(
            book.package.metadata.title.as_deref(),
            Some("Minimal EPUB 3")
        );
        assert_eq!(book.package.metadata.language.as_deref(), Some("en"));
        assert!(book
            .package
            .manifest
            .iter()
            .any(|item| item.properties.iter().any(|property| property == "nav")));
        assert_eq!(book.package.spine.len(), 1);
        assert_eq!(book.navigation.source, NavigationSource::Epub3Nav);
        assert_eq!(book.navigation.toc[0].href, "chapter-1.xhtml");
        assert!(book.store_stats.read_count < book.resources.len() as u64);
    }

    #[test]
    fn epub2_fixture_uses_ncx_fallback() {
        let bytes =
            crate::testkit::GeneratedEpubFixture::preset(crate::testkit::FixtureKind::Epub2WithNcx)
                .bytes()
                .to_vec();
        let book = open_book(bytes).expect("open book");

        assert_eq!(book.navigation.source, NavigationSource::Ncx);
        assert_eq!(book.navigation.toc[0].label, "Start");
    }

    #[test]
    fn strict_mode_rejects_path_escape() {
        assert!(resolve_resource_path("", "../evil.xhtml").is_err());
        assert!(resolve_resource_path("EPUB", "chapter.xhtml").is_ok());
    }

    #[test]
    fn zip_entry_limit_is_enforced() {
        let mut options = OpenOptions::compatible();
        options.limits.max_zip_entries = 1;

        let error =
            ZipPublicationStore::from_bytes(minimal_fixture_bytes(), options).expect_err("limit");
        assert_eq!(error.code(), DiagnosticCode::ResourceLimitExceeded);
    }

    #[test]
    fn open_book_ir_serializes_package_level_sections() {
        let ir = open_book_ir(minimal_fixture_bytes()).expect("book ir");
        let json = ir.to_golden_json();

        assert_eq!(ir.metadata.title.as_deref(), Some("Minimal EPUB 3"));
        assert!(json.contains(r#""manifest""#));
        assert!(json.contains(r#""resources""#));
        assert!(ir.resources.id_for_path("EPUB/chapter-1.xhtml").is_some());
    }

    #[test]
    fn xhtml_tokenizer_preserves_source_ranges() {
        let input = r#"<p id="a">Hi<br/>there</p>"#;
        let tokens = tokenize_xhtml(input).expect("tokens");

        assert!(tokens.len() >= 5);
        for token in tokens {
            assert!(token.source_range.end <= input.len() as u32);
            assert!(token.source_range.start < token.source_range.end);
        }
    }

    #[test]
    fn first_chapter_ir_maps_semantic_nodes_links_anchors_and_images() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::MinimalEpub3,
            "ChapterIR",
        )
        .feature("chapter-ir")
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r##"<h1 id="top">Title</h1><p>See <a href="#top">top</a>.</p><ul><li>One</li></ul><figure><img src="images/pic.png" alt="cover" /></figure>"##,
        )
        .add_entry("EPUB/images/pic.png", "image/png", vec![0; 32])
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let xhtml_len = fixture
            .entries()
            .iter()
            .find(|entry| entry.path.as_ref() == "EPUB/chapter-1.xhtml")
            .expect("chapter entry")
            .bytes
            .len() as u32;

        assert_eq!(chapter.href.as_ref(), "EPUB/chapter-1.xhtml");
        assert!(chapter.visible_text().contains("Title"));
        assert!(chapter.visible_text().contains("See top."));
        assert!(chapter.visible_text().contains("One"));
        assert!(chapter.anchors.get("EPUB/chapter-1.xhtml#top").is_some());
        assert_eq!(chapter.links.len(), 1);
        assert_eq!(
            chapter.links[0].resolved_document.as_deref(),
            Some("EPUB/chapter-1.xhtml")
        );
        assert_eq!(chapter.links[0].fragment.as_deref(), Some("top"));
        assert!(chapter.nodes.iter_with_ids().any(|(_, node)| matches!(
            node,
            document::DocumentNode::Image(image)
                if image.resolved_path.as_deref() == Some("EPUB/images/pic.png")
                    && image.resource_id.is_some()
        )));
        assert!(chapter
            .source_map
            .ranges
            .values()
            .all(|range| range.end <= xhtml_len));
    }

    #[test]
    fn mixed_inline_div_stays_one_paragraph_with_host_style_runs() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Mixed inline paragraph",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r##"<?xml version="1.0" encoding="utf-8"?>
            <html xmlns="http://www.w3.org/1999/xhtml">
              <head>
                <title>Chapter 1</title>
                <style>
                  .entry { font-family: "Body Serif"; font-size: 18px; }
                  .title { font-family: "Literata"; }
                  a { font-weight: 700; }
                </style>
              </head>
              <body>
                <div id="entry" class="entry">
                  They sent letters of
                  <span class="title"><em id="scum">Scum Family</em></span>
                  and <a href="#target">stone-shattered windows</a>.
                </div>
                <p id="target">Target.</p>
              </body>
            </html>"##,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let entry = chapter
            .anchors
            .get("EPUB/chapter-1.xhtml#entry")
            .expect("entry anchor");
        let container = match chapter.nodes.get(entry.node_id) {
            Some(document::DocumentNode::Container(container)) => container,
            other => panic!("expected entry container, got {other:?}"),
        };
        assert_eq!(container.children.len(), 1);
        let paragraph_id = container.children[0];
        let paragraph = match chapter.nodes.get(paragraph_id) {
            Some(document::DocumentNode::Paragraph(paragraph)) => paragraph,
            other => panic!("expected anonymous paragraph, got {other:?}"),
        };
        let text = chapter
            .text_pool
            .get(paragraph.text)
            .expect("paragraph text");
        assert_eq!(
            text,
            "They sent letters of Scum Family and stone-shattered windows."
        );
        assert_eq!(paragraph.style_runs.first().map(|run| run.start), Some(0));
        assert_eq!(
            paragraph.style_runs.last().map(|run| run.end),
            Some(u32::try_from(text.len()).expect("text length"))
        );
        for window in paragraph.style_runs.windows(2) {
            assert_eq!(window[0].end, window[1].start);
        }
        assert!(paragraph
            .style_runs
            .iter()
            .all(|run| run.source_range.is_some()));

        let scum_run = paragraph
            .style_runs
            .iter()
            .find(|run| slice_text_range(text, run.start..run.end) == "Scum Family")
            .expect("italic title run");
        let scum_style = chapter.styles.get(scum_run.style).expect("scum style");
        assert_eq!(style_value(scum_style, "font-family"), Some("\"Literata\""));
        assert_eq!(style_value(scum_style, "font-style"), Some("italic"));

        let scum_anchor = chapter
            .anchors
            .get("EPUB/chapter-1.xhtml#scum")
            .expect("inline anchor");
        assert_eq!(scum_anchor.node_id, paragraph_id);
        assert!(text
            .get(usize::try_from(scum_anchor.utf8_byte_offset).expect("anchor offset")..)
            .is_some_and(|suffix| suffix.starts_with("Scum Family")));

        let inline_link = chapter
            .links
            .iter()
            .find(|link| link.href.as_ref() == "#target")
            .expect("inline link");
        assert_eq!(inline_link.source_node, paragraph_id);
        assert_eq!(
            inline_link
                .text_range
                .clone()
                .map(|range| slice_text_range(text, range).to_owned())
                .as_deref(),
            Some("stone-shattered windows")
        );
        assert!(inline_link.source_range.is_some());
        assert!(chapter.source_map.get(paragraph_id).is_some());

        let constraints = crate::layout::LayoutConstraints::new(
            LayoutUnit::from_px(1_000),
            LayoutUnit::from_px(500),
        )
        .with_margin(LayoutUnit::from_px(24));
        let options = crate::layout::LayoutOptions::new(constraints);
        let batch = crate::layout::prepare_measure_batch(&chapter, options);
        let request = batch
            .requests
            .iter()
            .find(|request| request.text.as_ref() == text)
            .expect("mixed inline request");
        assert_eq!(
            batch
                .requests
                .iter()
                .filter(|request| request.text.as_ref() == text)
                .count(),
            1
        );
        let measured_scum = request
            .style_runs
            .iter()
            .find(|run| slice_text_range(text, run.start..run.end) == "Scum Family")
            .expect("measured italic run");
        assert_eq!(measured_scum.fonts.primary.family.as_ref(), "Literata");
        assert_eq!(
            measured_scum.fonts.primary.style,
            crate::text::FontStyle::Italic
        );
        let measured_link = request
            .style_runs
            .iter()
            .find(|run| slice_text_range(text, run.start..run.end) == "stone-shattered windows")
            .expect("measured link run");
        assert_eq!(measured_link.fonts.primary.weight, 700);

        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            options,
        )
        .expect("paginate mixed inline chapter");
        let page = pages
            .pages
            .iter()
            .find(|page| {
                page.links
                    .iter()
                    .any(|link| link.href.as_ref() == "#target")
            })
            .expect("page with inline link");
        let paint = page
            .text_paints
            .iter()
            .find(|paint| paint.node_id == paragraph_id)
            .expect("paragraph paint");
        let link_region = page
            .links
            .iter()
            .find(|link| link.href.as_ref() == "#target")
            .expect("precise link region");
        assert_eq!(link_region.node_id, paragraph_id);
        assert_eq!(link_region.kind, document::LinkKind::Internal);
        assert_eq!(link_region.text_range, inline_link.text_range);
        assert!(link_region.rect.x > paint.layout_rect.x);
        assert!(link_region.rect.width < paint.layout_rect.width);
        let link_range = inline_link.text_range.as_ref().expect("inline link range");
        let page_json = page.to_normalized_json();
        assert!(page_json.contains(&format!(r#""node_id": {}"#, paragraph_id.get())));
        assert!(page_json.contains(r#""resolved_document": "EPUB/chapter-1.xhtml""#));
        assert!(page_json.contains(r#""fragment": "target""#));
        assert!(page_json.contains(r#""kind": "internal""#));
        assert!(page_json.contains(&format!(
            r#""text_range": {{"start": {}, "end": {}}}"#,
            link_range.start, link_range.end
        )));
        let anchor_region = page
            .anchors
            .iter()
            .find(|anchor| anchor.key.as_ref() == "EPUB/chapter-1.xhtml#scum")
            .expect("inline anchor region");
        assert_eq!(anchor_region.rect.width, LayoutUnit::from_px(1));
    }

    #[test]
    fn standalone_anchor_elements_keep_link_metadata() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::FootnoteCollision,
            "Standalone links",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r##"
            <a id="jump" href="#target">Jump to target</a>
            <a href="https://example.com/reference">External reference</a>
            <a epub:type="noteref" href="#fn1">Footnote</a>
            <p id="target">Target paragraph.</p>
            <aside epub:type="footnote" id="fn1"><p>Footnote body.</p></aside>
            "##,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");

        assert_eq!(chapter.links.len(), 3);
        let internal = chapter
            .links
            .iter()
            .find(|link| link.href.as_ref() == "#target")
            .expect("internal link");
        assert_eq!(internal.kind, document::LinkKind::Internal);
        assert_eq!(internal.fragment.as_deref(), Some("target"));
        assert_eq!(
            internal.resolved_document.as_deref(),
            Some("EPUB/chapter-1.xhtml")
        );

        let external = chapter
            .links
            .iter()
            .find(|link| link.href.as_ref() == "https://example.com/reference")
            .expect("external link");
        assert_eq!(external.kind, document::LinkKind::External);
        assert!(external.resolved_document.is_none());

        let footnote = chapter
            .links
            .iter()
            .find(|link| link.href.as_ref() == "#fn1")
            .expect("footnote link");
        assert_eq!(footnote.kind, document::LinkKind::Footnote);
        assert_eq!(footnote.fragment.as_deref(), Some("fn1"));

        for link in &chapter.links {
            assert!(matches!(
                chapter.nodes.get(link.source_node),
                Some(document::DocumentNode::Paragraph(_))
            ));
            assert!(link.source_range.is_some());
        }
        assert!(chapter.anchors.get("EPUB/chapter-1.xhtml#jump").is_some());
    }

    #[test]
    fn compatible_mode_salvages_malformed_xhtml_without_panic() {
        let bytes = crate::testkit::GeneratedEpubFixture::preset(
            crate::testkit::FixtureKind::MalformedXhtml,
        )
        .bytes()
        .to_vec();
        let chapter = open_first_chapter_ir(bytes.clone()).expect("compatible chapter");
        assert!(chapter.visible_text().contains("Missing close"));

        let strict_error = open_spine_item_chapter_ir_with_options(bytes, 0, OpenOptions::strict())
            .expect_err("strict rejects malformed xhtml");
        assert_eq!(strict_error.code(), DiagnosticCode::Parse);
    }

    #[test]
    fn css_parser_and_cascade_apply_specificity_inheritance_and_inline_style() {
        let stylesheet = parse_css(
            r#"
            @import "theme.css";
            p.lead {
              font-weight: normal;
              color: red;
              margin: 1em 2em 3em 4em;
              margin-left: 5em;
              padding: 6px 7px;
            }
            #main .lead { font-weight: bold; }
            section p { text-indent: 2em; }
            "#,
            ResourceLimits::default(),
        )
        .expect("parse css");
        let element = CssElementSnapshot {
            name: "p".to_owned(),
            id: None,
            classes: vec!["lead".to_owned()],
            inline_style: Some("font-style: italic".to_owned()),
        };
        let ancestors = vec![CssElementSnapshot {
            name: "section".to_owned(),
            id: Some("main".to_owned()),
            classes: Vec::new(),
            inline_style: None,
        }];
        let inherited = document::ComputedStyle::new().with_property("font-family", "serif");
        let computed = cascade_css_for_element(&element, &ancestors, &stylesheet, &inherited);

        assert_eq!(stylesheet.imports[0].href, "theme.css");
        assert!(stylesheet
            .unsupported
            .iter()
            .any(|declaration| declaration.property == "color"));
        assert_eq!(style_value(&computed, "font-family"), Some("serif"));
        assert_eq!(style_value(&computed, "font-weight"), Some("bold"));
        assert_eq!(style_value(&computed, "font-style"), Some("italic"));
        assert_eq!(style_value(&computed, "text-indent"), Some("2em"));
        assert_eq!(style_value(&computed, "margin"), None);
        assert_eq!(style_value(&computed, "margin-top"), Some("1em"));
        assert_eq!(style_value(&computed, "margin-right"), Some("2em"));
        assert_eq!(style_value(&computed, "margin-bottom"), Some("3em"));
        assert_eq!(style_value(&computed, "margin-left"), Some("5em"));
        assert_eq!(style_value(&computed, "padding-top"), Some("6px"));
        assert_eq!(style_value(&computed, "padding-right"), Some("7px"));
        assert_eq!(style_value(&computed, "padding-bottom"), Some("6px"));
        assert_eq!(style_value(&computed, "padding-left"), Some("7px"));
    }

    #[test]
    fn css_box_shorthand_expands_one_to_four_values() {
        for (value, expected) in [
            ("1px", ["1px", "1px", "1px", "1px"]),
            ("1px 2px", ["1px", "2px", "1px", "2px"]),
            ("1px 2px 3px", ["1px", "2px", "3px", "2px"]),
            ("1px 2px 3px 4px", ["1px", "2px", "3px", "4px"]),
        ] {
            let (declarations, unsupported) = parse_css_declarations(&format!("margin: {value}"));
            assert!(unsupported.is_empty());
            let values = declarations
                .iter()
                .map(|declaration| declaration.value.as_str())
                .collect::<Vec<_>>();
            assert_eq!(values, expected);
        }
    }

    #[test]
    fn chapter_styles_resolve_relative_font_metrics_to_px() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Relative font metrics",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<?xml version="1.0" encoding="utf-8"?>
            <html xmlns="http://www.w3.org/1999/xhtml">
              <head>
                <title>Chapter 1</title>
                <style>
                  html { font-size: 20px; }
                  section { font-size: 150%; line-height: 1.5; }
                  p.em { font-size: 2em; line-height: 125%; }
                  p.rem { font-size: 2rem; }
                </style>
              </head>
              <body>
                <section>
                  <p class="em">EM paragraph.</p>
                  <p class="rem">REM paragraph.</p>
                </section>
              </body>
            </html>"#,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let styles = paragraph_styles(&chapter);

        assert_eq!(styles.len(), 2);
        assert_eq!(style_value(styles[0], "font-size"), Some("60px"));
        assert_eq!(style_value(styles[0], "line-height"), Some("75px"));
        assert_eq!(style_value(styles[1], "font-size"), Some("40px"));
        assert_eq!(style_value(styles[1], "line-height"), Some("60px"));

        let pages = crate::layout::paginate_chapter(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutConstraints::default(),
        )
        .expect("pagination");
        let line_heights = pages
            .pages
            .iter()
            .flat_map(visible_text_line_rects)
            .map(|rect| rect.height)
            .collect::<Vec<_>>();

        assert_eq!(line_heights.first(), Some(&LayoutUnit::from_px(75)));
        assert_eq!(line_heights.last(), Some(&LayoutUnit::from_px(60)));
    }

    #[test]
    fn chapter_styles_resolve_font_relative_geometry_and_preserve_percentages() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Relative geometry",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<?xml version="1.0" encoding="utf-8"?>
            <html xmlns="http://www.w3.org/1999/xhtml">
              <head>
                <title>Chapter 1</title>
                <style>
                  html { font-size: 20px; }
                  section {
                    font-size: 150%;
                    text-indent: 2em;
                    padding: .5em 2%;
                  }
                  section p {
                    font-size: 2em;
                    margin: 1em 10% 2rem 5%;
                  }
                </style>
              </head>
              <body>
                <section><p>Geometry paragraph.</p></section>
              </body>
            </html>"#,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let paragraph = first_paragraph_style(&chapter).expect("paragraph style");

        assert_eq!(style_value(paragraph, "font-size"), Some("60px"));
        assert_eq!(style_value(paragraph, "text-indent"), Some("60px"));
        assert_eq!(style_value(paragraph, "margin-top"), Some("60px"));
        assert_eq!(style_value(paragraph, "margin-right"), Some("10%"));
        assert_eq!(style_value(paragraph, "margin-bottom"), Some("40px"));
        assert_eq!(style_value(paragraph, "margin-left"), Some("5%"));

        let section = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(_, node)| match node {
                document::DocumentNode::Container(container)
                    if chapter
                        .styles
                        .get(container.style)
                        .is_some_and(|style| style.properties.contains_key("padding-top")) =>
                {
                    chapter.styles.get(container.style)
                }
                _ => None,
            })
            .expect("section style");
        assert_eq!(style_value(section, "padding-top"), Some("15px"));
        assert_eq!(style_value(section, "padding-right"), Some("2%"));
        assert_eq!(style_value(section, "padding-bottom"), Some("15px"));
        assert_eq!(style_value(section, "padding-left"), Some("2%"));

        let options = crate::layout::LayoutOptions::default();
        let content_width = options.constraints.content_width();
        let section_padding = LayoutUnit::from_f64_px(content_width.to_f64_px() * 2.0 / 100.0);
        let paragraph_containing_width = content_width - section_padding - section_padding;
        let paragraph_margin_right =
            LayoutUnit::from_f64_px(paragraph_containing_width.to_f64_px() * 10.0 / 100.0);
        let paragraph_margin_left =
            LayoutUnit::from_f64_px(paragraph_containing_width.to_f64_px() * 5.0 / 100.0);
        let expected_measurement_width = paragraph_containing_width
            - paragraph_margin_right
            - paragraph_margin_left
            - LayoutUnit::from_px(60);
        let batch = crate::layout::prepare_measure_batch(&chapter, options);

        assert_eq!(
            batch.requests[0].available_width,
            expected_measurement_width
        );

        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            options,
        )
        .expect("paginate geometry fixture");
        let first_line = visible_text_line_rects(&pages.pages[0])[0];
        assert_eq!(
            first_line.x,
            options.constraints.margin_start
                + section_padding
                + paragraph_margin_left
                + LayoutUnit::from_px(60)
        );
        assert_eq!(
            first_line.y,
            options.constraints.margin_top + LayoutUnit::from_px(75)
        );
    }

    #[test]
    fn linked_css_imports_are_loaded_and_errors_are_diagnosed() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Linked CSS",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title><link rel="stylesheet" href="styles/base.css"/></head><body><p class="lead">Styled.</p></body></html>"#,
        )
        .add_stylesheet("EPUB/styles/base.css", r#"@import "theme.css"; p { font-style: italic; }"#)
        .add_stylesheet("EPUB/styles/theme.css", ".lead { font-weight: bold; }")
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let paragraph_style = first_paragraph_style(&chapter).expect("paragraph style");

        assert_eq!(style_value(paragraph_style, "font-style"), Some("italic"));
        assert_eq!(style_value(paragraph_style, "font-weight"), Some("bold"));

        let mut options = OpenOptions::compatible();
        options.limits.max_css_import_depth = 0;
        let error = open_spine_item_chapter_ir_with_options(fixture.bytes().to_vec(), 0, options)
            .expect_err("depth limit");
        assert_eq!(error.code(), DiagnosticCode::ResourceLimitExceeded);

        let cycle = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "CSS Cycle",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title><link rel="stylesheet" href="styles/base.css"/></head><body><p>Styled.</p></body></html>"#,
        )
        .add_stylesheet("EPUB/styles/base.css", r#"@import "theme.css";"#)
        .add_stylesheet("EPUB/styles/theme.css", r#"@import "base.css";"#)
        .build();
        let error = open_first_chapter_ir(cycle.bytes().to_vec()).expect_err("cycle");
        assert_eq!(error.code(), DiagnosticCode::Parse);
    }

    #[test]
    fn unsupported_css_properties_are_recorded_as_chapter_diagnostics() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "CSS Diagnostics",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title><style>p { color: red; font-weight: bold; }</style></head><body><p>Styled.</p></body></html>"#,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");

        assert!(chapter.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == DiagnosticCode::UnsupportedFeature
                && diagnostic.message.contains("color")
        }));
    }

    #[test]
    fn authored_list_style_none_suppresses_ordered_list_markers() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Markerless contents",
        )
        .add_xhtml(
            "EPUB/contents.xhtml",
            "contents",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Contents</title><link rel="stylesheet" href="styles.css"/></head><body><ol class="contents"><li><a href="chapter.xhtml">Chapter</a></li></ol></body></html>"#,
        )
        .add_stylesheet("EPUB/styles.css", ".contents { list-style: none; }")
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("contents chapter ir");
        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::new(crate::layout::LayoutConstraints::default()),
        )
        .expect("paginate markerless contents");

        assert!(pages
            .pages
            .iter()
            .flat_map(|page| &page.fragments)
            .all(|fragment| fragment.kind != crate::layout::SceneFragmentKind::Marker));
    }

    #[test]
    fn footnote_noterefs_get_backlinks_and_unreferenced_notes_are_skipped() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::FootnoteCollision,
            "Footnotes",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r##"<p>See <a epub:type="noteref" href="#fn1">1</a>.</p><aside epub:type="footnote" id="fn1"><p>Referenced note.</p></aside><aside epub:type="footnote" id="fn2"><p>Unreferenced note.</p></aside>"##,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let footnotes = footnote_nodes(&chapter);

        assert_eq!(footnotes.len(), 1);
        assert_eq!(footnotes[0].note_id.as_deref(), Some("fn1"));
        let backlink = footnotes[0].backlink.as_ref().expect("backlink");
        assert_eq!(backlink.kind, document::LinkKind::Footnote);
        assert_eq!(backlink.fragment.as_deref(), Some("fn1"));
        assert!(chapter.visible_text().contains("Referenced note."));
        assert!(!chapter.visible_text().contains("Unreferenced note."));
    }

    #[test]
    fn external_footnotes_are_loaded_on_demand_by_fragment() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::FootnoteCollision,
            "External Footnotes",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r##"<p>See <a epub:type="noteref" href="notes.xhtml#fn1">1</a>.</p>"##,
        )
        .add_xhtml(
            "EPUB/notes.xhtml",
            "Notes",
            r##"<aside epub:type="footnote" id="fn1"><p>External note.</p></aside><aside epub:type="footnote" id="fn2"><p>Other note.</p></aside>"##,
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let footnotes = footnote_nodes(&chapter);

        assert_eq!(footnotes.len(), 1);
        assert_eq!(footnotes[0].note_id.as_deref(), Some("fn1"));
        assert!(chapter.visible_text().contains("External note."));
        assert!(!chapter.visible_text().contains("Other note."));
    }

    #[test]
    fn image_header_parser_handles_png_gif_and_jpeg() {
        assert_eq!(
            parse_image_header(&png_header(640, 480), "image/png"),
            Some(document::ImageSize {
                width: 640,
                height: 480
            })
        );
        assert_eq!(
            parse_image_header(b"GIF89a\x20\x00\x10\x00\x00\x00", "image/gif"),
            Some(document::ImageSize {
                width: 32,
                height: 16
            })
        );
        assert_eq!(
            parse_image_header(
                &[
                    0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x20, 0x00, 0x10, 0x03, 0x01,
                    0x11, 0x00,
                ],
                "image/jpeg",
            ),
            Some(document::ImageSize {
                width: 16,
                height: 32
            })
        );
        assert_eq!(parse_image_header(b"not an image", "image/png"), None);
    }

    #[test]
    fn linked_image_preserves_image_node_and_clickable_region() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::HugeImage,
            "Linked image",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r##"<a href="chapter-2.xhtml#target"><img src="images/part.jpg" alt="" /></a>"##,
        )
        .add_entry("EPUB/images/part.jpg", "image/jpeg", jpeg_header(600, 900))
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let (image_id, image) = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(node_id, node)| match node {
                document::DocumentNode::Image(image) => Some((node_id, image)),
                _ => None,
            })
            .expect("linked image node");

        assert_eq!(image.resolved_path.as_deref(), Some("EPUB/images/part.jpg"));
        assert_eq!(image.layout_role, document::ImageLayoutRole::Standalone);
        assert!(!chapter.visible_text().contains("chapter-2.xhtml"));
        assert!(!chapter
            .nodes
            .iter_with_ids()
            .any(|(_, node)| matches!(node, document::DocumentNode::Paragraph(_))));
        let link = chapter
            .links
            .iter()
            .find(|link| link.source_node == image_id)
            .expect("image link");
        assert_eq!(link.href.as_ref(), "chapter-2.xhtml#target");
        assert_eq!(
            link.resolved_document.as_deref(),
            Some("EPUB/chapter-2.xhtml")
        );
        assert_eq!(link.fragment.as_deref(), Some("target"));
        assert_eq!(link.kind, document::LinkKind::Internal);
        assert_eq!(link.text_range, None);

        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::default(),
        )
        .expect("paginate linked image");
        let page = pages.pages.first().expect("image page");
        let image_fragment = page
            .fragments
            .iter()
            .find(|fragment| {
                fragment.node_id == image_id
                    && fragment.kind == crate::layout::SceneFragmentKind::Image
            })
            .expect("image fragment");
        let link_region = page
            .links
            .iter()
            .find(|region| region.node_id == image_id)
            .expect("clickable image region");
        assert_eq!(link_region.rect, image_fragment.rect);
    }

    #[test]
    fn book_ir_indexes_lazy_image_dimensions_and_font_fingerprints() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::HugeImage,
            "Resources",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<figure><img src="images/pic.png" alt="pic" /></figure>"#,
        )
        .add_entry("EPUB/images/pic.png", "image/png", png_header(320, 200))
        .add_entry(
            "EPUB/fonts/body.ttf",
            "font/ttf",
            b"fake font bytes".to_vec(),
        )
        .build();
        let ir = open_book_ir(fixture.bytes().to_vec()).expect("book ir");

        let image = ir
            .resources
            .images
            .iter()
            .find(|image| image.path.as_ref() == "EPUB/images/pic.png")
            .expect("image resource");
        assert_eq!(
            image.intrinsic_size,
            Some(document::ImageSize {
                width: 320,
                height: 200
            })
        );
        assert_eq!(image.byte_length, png_header(320, 200).len() as u64);

        let font = ir
            .resources
            .fonts
            .iter()
            .find(|font| font.path.as_ref() == "EPUB/fonts/body.ttf")
            .expect("font resource");
        assert_ne!(
            font.fingerprint,
            crate::core::ContentHash::from_bytes(b"EPUB/fonts/body.ttf")
        );
    }

    #[test]
    fn intrinsic_image_dimensions_drive_pagination_geometry() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::HugeImage,
            "Intrinsic image layout",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<p>Before.</p><img src="images/pic.png" alt="intrinsic"/><p>After.</p>"#,
        )
        .add_entry("EPUB/images/pic.png", "image/png", png_header(240, 120))
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::new(crate::layout::LayoutConstraints::new(
                LayoutUnit::from_px(400),
                LayoutUnit::from_px(500),
            )),
        )
        .expect("paginate");
        let image = pages
            .pages
            .iter()
            .flat_map(|page| &page.fragments)
            .find(|fragment| fragment.kind == crate::layout::SceneFragmentKind::Image)
            .expect("image fragment");

        assert_eq!(image.rect.width, LayoutUnit::from_px(240));
        assert_eq!(image.rect.height, LayoutUnit::from_px(120));
    }

    #[test]
    fn authored_image_dimensions_and_maxima_shape_pagination_geometry() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Authored image layout",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title><link rel="stylesheet" href="styles/book.css"/></head><body><p>Before.</p><img src="images/pic.png" alt="styled"/><p>After.</p></body></html>"#,
        )
        .add_stylesheet(
            "EPUB/styles/book.css",
            "img { width: 50%; height: auto; max-width: 120px; max-height: 90px; }",
        )
        .add_entry("EPUB/images/pic.png", "image/png", png_header(400, 200))
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::new(crate::layout::LayoutConstraints::new(
                LayoutUnit::from_px(400),
                LayoutUnit::from_px(500),
            )),
        )
        .expect("paginate");
        let image = pages
            .pages
            .iter()
            .flat_map(|page| &page.fragments)
            .find(|fragment| fragment.kind == crate::layout::SceneFragmentKind::Image)
            .expect("image fragment");

        assert_eq!(image.rect.width, LayoutUnit::from_px(120));
        assert_eq!(image.rect.height, LayoutUnit::from_px(60));
    }

    #[test]
    fn part0010_full_width_banner_uses_authored_width() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::CssCascade,
            "Part 0010 banner",
        )
        .add_xhtml(
            "EPUB/part0010.xhtml",
            "part0010",
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>part0010</title><link rel="stylesheet" type="text/css" href="stylesheet.css"/></head><body><div class="class218-0"><img src="image217.jpg" alt="" class="class218-1"/></div><div class="class222">Pip knew where they lived.</div></body></html>"#,
        )
        .add_stylesheet(
            "EPUB/stylesheet.css",
            ".class218-0 { text-align: center; } .class218-1 { width: 100%; } .class222 { margin-top: 3.2em; }",
        )
        .add_entry(
            "EPUB/image217.jpg",
            "image/jpeg",
            jpeg_header(1522, 422),
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("part0010 chapter ir");
        let banner = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(_, node)| match node {
                document::DocumentNode::Image(image) => Some(image),
                _ => None,
            })
            .expect("part0010 banner node");
        assert_eq!(banner.layout_role, document::ImageLayoutRole::Block);
        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::new(crate::layout::LayoutConstraints::new(
                LayoutUnit::from_px(496),
                LayoutUnit::from_px(800),
            )),
        )
        .expect("paginate part0010");
        let image = pages
            .pages
            .iter()
            .flat_map(|page| &page.fragments)
            .find(|fragment| fragment.kind == crate::layout::SceneFragmentKind::Image)
            .expect("part0010 banner fragment");

        assert_eq!(image.rect.width, LayoutUnit::from_px(496));
        assert!((image.rect.height.to_f64_px() - 137.52).abs() < 0.02);
    }

    #[test]
    fn block_image_without_authored_width_keeps_safety_cap() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::HugeImage,
            "Unauthored block image",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<div><img src="images/banner.jpg" alt=""/></div><p>After the image.</p>"#,
        )
        .add_entry(
            "EPUB/images/banner.jpg",
            "image/jpeg",
            jpeg_header(1522, 422),
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::new(crate::layout::LayoutConstraints::new(
                LayoutUnit::from_px(496),
                LayoutUnit::from_px(800),
            )),
        )
        .expect("paginate");
        let image = pages
            .pages
            .iter()
            .flat_map(|page| &page.fragments)
            .find(|fragment| fragment.kind == crate::layout::SceneFragmentKind::Image)
            .expect("image fragment");

        assert_eq!(image.rect.width, LayoutUnit::from_px(280));
        assert!((image.rect.height.to_f64_px() - 77.64).abs() < 0.02);
    }

    #[test]
    fn image_only_page_treats_indefinite_percentage_height_as_auto() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::HugeImage,
            "Image-only page",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Chapter 1",
            r#"<img src="images/pic.png" alt="standalone" style="height: 70%"/>"#,
        )
        .add_entry("EPUB/images/pic.png", "image/png", png_header(600, 900))
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        assert!(chapter.nodes.iter_with_ids().any(|(_, node)| matches!(
            node,
            document::DocumentNode::Image(image)
                if image.layout_role == document::ImageLayoutRole::Standalone
                    && image.intrinsic_size
                        == Some(document::ImageSize { width: 600, height: 900 })
        )));
        let pages = crate::layout::paginate_chapter_with_options(
            &chapter,
            &crate::text::DefaultTextBackend::new(),
            crate::layout::LayoutOptions::new(crate::layout::LayoutConstraints::new(
                LayoutUnit::from_px(400),
                LayoutUnit::from_px(400),
            )),
        )
        .expect("paginate");
        let image = pages
            .pages
            .iter()
            .flat_map(|page| &page.fragments)
            .find(|fragment| fragment.kind == crate::layout::SceneFragmentKind::Image)
            .expect("image fragment");

        assert!((image.rect.width.to_f64_px() - 266.67).abs() < 0.05);
        assert_eq!(image.rect.height, LayoutUnit::from_px(400));
    }

    #[test]
    fn svg_wrapped_cover_surfaces_only_the_safe_bitmap_resource() {
        let fixture = crate::testkit::EpubFixtureBuilder::epub3(
            crate::testkit::FixtureKind::HugeImage,
            "SVG-wrapped cover",
        )
        .add_xhtml(
            "EPUB/chapter-1.xhtml",
            "Cover",
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 600 917"><image width="600" height="917" xlink:href="images/cover.jpg"/></svg>"#,
        )
        .add_entry(
            "EPUB/images/cover.jpg",
            "image/jpeg",
            jpeg_header(600, 917),
        )
        .build();
        let chapter = open_first_chapter_ir(fixture.bytes().to_vec()).expect("chapter ir");
        let image_node = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(_, node)| match node {
                document::DocumentNode::Image(image) => Some(image),
                _ => None,
            })
            .expect("sanitized bitmap image node");

        assert_eq!(
            image_node.resolved_path.as_deref(),
            Some("EPUB/images/cover.jpg")
        );
        assert_eq!(
            image_node.intrinsic_size,
            Some(document::ImageSize {
                width: 600,
                height: 917,
            })
        );
        assert!(!chapter.nodes.iter_with_ids().any(|(_, node)| matches!(
            node,
            document::DocumentNode::Unsupported(unsupported)
                if unsupported.element.as_ref() == "svg" || unsupported.element.as_ref() == "image"
        )));
    }

    #[test]
    fn publication_cover_path_marks_cover_image_role() {
        let resource_id = ResourceId::new(7);
        let mut resources = document::ResourceTable::new();
        resources.push(document::ResourceInfo {
            id: resource_id,
            path: Arc::from("EPUB/images/cover.jpg"),
            media_type: Arc::from("image/jpeg"),
            kind: document::ResourceKind::Image,
            compressed_size: 12,
            uncompressed_size: 12,
            compression_method: 0,
        });
        resources.set_image_size(
            resource_id,
            Some(document::ImageSize {
                width: 600,
                height: 917,
            }),
        );

        let chapter = chapter_ir_from_xhtml(
            DocumentId::new(0),
            "EPUB/titlepage.xhtml",
            "Cover",
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/cover.jpg"/></body></html>"#,
            ChapterResourceContext {
                resources: &resources,
                cover_image: Some("EPUB/images/cover.jpg"),
                store: None,
                options: OpenOptions::default(),
            },
        )
        .expect("chapter ir");
        let image = chapter
            .nodes
            .iter_with_ids()
            .find_map(|(_, node)| match node {
                document::DocumentNode::Image(image) => Some(image),
                _ => None,
            })
            .expect("cover image");

        assert_eq!(image.layout_role, document::ImageLayoutRole::Cover);
        assert_eq!(
            image.intrinsic_size,
            Some(document::ImageSize {
                width: 600,
                height: 917,
            })
        );
    }

    fn style_value<'a>(style: &'a document::ComputedStyle, property: &str) -> Option<&'a str> {
        style.properties.get(property).map(AsRef::as_ref)
    }

    fn slice_text_range(text: &str, range: std::ops::Range<u32>) -> &str {
        let start = usize::try_from(range.start).expect("range start");
        let end = usize::try_from(range.end).expect("range end");
        text.get(start..end).expect("valid text range")
    }

    fn first_paragraph_style(chapter: &document::ChapterIr) -> Option<&document::ComputedStyle> {
        chapter.nodes.iter_with_ids().find_map(|(_, node)| {
            if let document::DocumentNode::Paragraph(block) = node {
                return chapter.styles.get(block.style);
            }
            None
        })
    }

    fn paragraph_styles(chapter: &document::ChapterIr) -> Vec<&document::ComputedStyle> {
        chapter
            .nodes
            .iter_with_ids()
            .filter_map(|(_, node)| {
                if let document::DocumentNode::Paragraph(block) = node {
                    chapter.styles.get(block.style)
                } else {
                    None
                }
            })
            .collect()
    }

    fn footnote_nodes(chapter: &document::ChapterIr) -> Vec<&document::FootnoteNode> {
        chapter
            .nodes
            .iter_with_ids()
            .filter_map(|(_, node)| {
                if let document::DocumentNode::Footnote(note) = node {
                    Some(note)
                } else {
                    None
                }
            })
            .collect()
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes
    }

    fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
        const APP_PAYLOAD_BYTES: usize = 8 * 1024;
        let app_segment_len = u16::try_from(APP_PAYLOAD_BYTES + 2).expect("APP segment length");
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1];
        bytes.extend_from_slice(&app_segment_len.to_be_bytes());
        bytes.resize(bytes.len() + APP_PAYLOAD_BYTES, 0);
        bytes.extend_from_slice(&[0xff, 0xc0, 0x00, 0x0b, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[0x03, 0x01, 0x11, 0x00]);
        bytes
    }
}
