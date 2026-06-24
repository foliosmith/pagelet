//! EPUB container, package, navigation, diagnostics, and inspect support.

use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

pub use crate::core::ResourceId;
use crate::core::{
    Diagnostic, DiagnosticCode, PackageError, PageletError, ParseError, ResourceLimitError,
    ResourceLimitKind, ResourceLimits, Severity, UnsupportedFeature,
};

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
            if entry.compressed_size > 0 {
                let ratio = (entry
                    .uncompressed_size
                    .saturating_add(entry.compressed_size - 1))
                    / entry.compressed_size;
                if ratio > u64::from(options.limits.max_compression_ratio) {
                    return Err(limit_error(
                        ResourceLimitKind::CompressionRatio,
                        u64::from(options.limits.max_compression_ratio),
                        ratio,
                    ));
                }
            } else if entry.uncompressed_size > 0 {
                return Err(limit_error(
                    ResourceLimitKind::CompressionRatio,
                    u64::from(options.limits.max_compression_ratio),
                    u64::MAX,
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
        let index = usize::try_from(resource_id.get()).map_err(|_| {
            PageletError::InvalidContainer(crate::core::ContainerError::new("invalid resource id"))
        })?;
        let entry = self.entries.get(index).ok_or_else(|| {
            PageletError::InvalidContainer(crate::core::ContainerError::new("unknown resource id"))
        })?;
        if entry.compression_method != 0 {
            return Err(PageletError::UnsupportedFeature(UnsupportedFeature::new(
                "compressed ZIP entries are not supported by the M1 store yet",
            )));
        }
        let start = entry.data_offset;
        let end = start
            .checked_add(usize::try_from(entry.compressed_size).unwrap_or(usize::MAX))
            .ok_or_else(|| invalid_container("ZIP entry range overflows"))?;
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or_else(|| invalid_container("ZIP entry range is outside archive"))?
            .to_vec();
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
            bytes,
        })
    }

    fn open_stream(
        &self,
        resource_id: ResourceId,
    ) -> Result<Box<dyn Read + Send + 'static>, PageletError> {
        Ok(Box::new(Cursor::new(self.read(resource_id)?.bytes)))
    }
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
            CapabilityStatus::UnsupportedDiagnosed,
            "deflate entries are diagnosed until the full ZIP backend lands",
        );
        report.push(
            "epub/package",
            CapabilityStatus::SupportedWithLimitations,
            "metadata, manifest, spine, nav, NCX and guide are parsed for M1",
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

/// Open an EPUB and parse metadata/navigation with compatible defaults.
pub fn open_book(bytes: impl Into<Vec<u8>>) -> Result<BookSummary, PageletError> {
    open_book_with_options(bytes, OpenOptions::default())
}

/// Open an EPUB and parse metadata/navigation using explicit options.
pub fn open_book_with_options(
    bytes: impl Into<Vec<u8>>,
    options: OpenOptions,
) -> Result<BookSummary, PageletError> {
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

    Ok(BookSummary {
        rootfiles,
        package,
        navigation,
        diagnostics: diagnostics.into_vec(),
        capability_report,
        resources,
        store_stats,
    })
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

    let cover_image = manifest
        .iter()
        .find(|item| {
            item.properties
                .iter()
                .any(|property| property == "cover-image")
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
    while cursor < central_end {
        if read_u32(bytes, cursor)? != 0x0201_4b50 {
            return Err(invalid_container("invalid central directory header"));
        }
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
        entries.push(ZipEntry {
            path,
            compression_method,
            compressed_size,
            uncompressed_size,
            data_offset,
        });
        cursor = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| invalid_container("central directory cursor overflows"))?;
    }

    if entries.len() != entry_count && options.compatibility_mode == CompatibilityMode::Strict {
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
}
