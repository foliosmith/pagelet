//! Generated fixture and golden-test helpers.

use std::{fmt, sync::Arc};

use crate::{
    core::{CancellationToken, LayoutUnit, PageletError},
    text::{
        FontSetFingerprint, LineMetrics, MeasureBatch, MeasuredBatch, MeasuredText, TextBackend,
        TextBackendId, TextCluster,
    },
};

/// Generated EPUB fixture category.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum FixtureKind {
    MinimalEpub3,
    Epub2WithNcx,
    FallbackChain,
    FootnoteCollision,
    PathCaseCollision,
    CssCascade,
    MalformedXhtml,
    HugeImage,
    Rtl,
    DataUri,
    DuplicateIds,
    ZipBombLike,
}

impl FixtureKind {
    /// All baseline generated fixture kinds.
    pub const ALL: [Self; 12] = [
        Self::MinimalEpub3,
        Self::Epub2WithNcx,
        Self::FallbackChain,
        Self::FootnoteCollision,
        Self::PathCaseCollision,
        Self::CssCascade,
        Self::MalformedXhtml,
        Self::HugeImage,
        Self::Rtl,
        Self::DataUri,
        Self::DuplicateIds,
        Self::ZipBombLike,
    ];

    /// Stable fixture id.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::MinimalEpub3 => "minimal-epub3",
            Self::Epub2WithNcx => "epub2-with-ncx",
            Self::FallbackChain => "fallback-chain",
            Self::FootnoteCollision => "footnote-collision",
            Self::PathCaseCollision => "path-case-collision",
            Self::CssCascade => "css-cascade",
            Self::MalformedXhtml => "malformed-xhtml",
            Self::HugeImage => "huge-image",
            Self::Rtl => "rtl",
            Self::DataUri => "data-uri",
            Self::DuplicateIds => "duplicate-ids",
            Self::ZipBombLike => "zip-bomb-like",
        }
    }
}

/// Benchmark fixture category.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum BenchmarkFixtureKind {
    TinyText,
    SmallNovel,
    LargeNovel,
    ImageHeavy,
    FootnoteHeavy,
    CssHeavy,
    CjkRtl,
    Pathological,
}

impl BenchmarkFixtureKind {
    /// All baseline benchmark fixture kinds.
    pub const ALL: [Self; 8] = [
        Self::TinyText,
        Self::SmallNovel,
        Self::LargeNovel,
        Self::ImageHeavy,
        Self::FootnoteHeavy,
        Self::CssHeavy,
        Self::CjkRtl,
        Self::Pathological,
    ];

    /// Stable benchmark fixture id.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::TinyText => "tiny-text",
            Self::SmallNovel => "small-novel",
            Self::LargeNovel => "large-novel",
            Self::ImageHeavy => "image-heavy",
            Self::FootnoteHeavy => "footnote-heavy",
            Self::CssHeavy => "css-heavy",
            Self::CjkRtl => "cjk-rtl",
            Self::Pathological => "pathological",
        }
    }
}

/// One generated EPUB archive entry before ZIP packaging.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FixtureEntry {
    pub path: Arc<str>,
    pub media_type: Arc<str>,
    pub bytes: Vec<u8>,
}

impl FixtureEntry {
    /// Create an archive entry.
    #[must_use]
    pub fn new(
        path: impl Into<Arc<str>>,
        media_type: impl Into<Arc<str>>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            path: path.into(),
            media_type: media_type.into(),
            bytes: bytes.into(),
        }
    }
}

/// Generated EPUB fixture with deterministic ZIP bytes.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedEpubFixture {
    pub kind: FixtureKind,
    pub id: Arc<str>,
    pub expected_features: Vec<Arc<str>>,
    entries: Vec<FixtureEntry>,
    bytes: Vec<u8>,
}

impl GeneratedEpubFixture {
    /// Generate one of the baseline fixture presets.
    #[must_use]
    pub fn preset(kind: FixtureKind) -> Self {
        EpubFixtureBuilder::preset(kind).build()
    }

    /// Deterministic `.epub` bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Generated archive entries.
    #[must_use]
    pub fn entries(&self) -> &[FixtureEntry] {
        &self.entries
    }

    /// Return true if an entry path exists.
    #[must_use]
    pub fn contains_entry(&self, path: &str) -> bool {
        self.entries.iter().any(|entry| entry.path.as_ref() == path)
    }
}

/// Builder for deterministic generated EPUB fixtures.
#[derive(Debug, Clone)]
pub struct EpubFixtureBuilder {
    kind: FixtureKind,
    title: Arc<str>,
    package_version: EpubPackageVersion,
    entries: Vec<FixtureEntry>,
    expected_features: Vec<Arc<str>>,
}

impl EpubFixtureBuilder {
    /// Create an EPUB 3 fixture builder.
    #[must_use]
    pub fn epub3(kind: FixtureKind, title: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            title: title.into(),
            package_version: EpubPackageVersion::Epub3,
            entries: Vec::new(),
            expected_features: Vec::new(),
        }
    }

    /// Create an EPUB 2 fixture builder.
    #[must_use]
    pub fn epub2(kind: FixtureKind, title: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            title: title.into(),
            package_version: EpubPackageVersion::Epub2,
            entries: Vec::new(),
            expected_features: Vec::new(),
        }
    }

    /// Return a baseline fixture preset builder.
    #[must_use]
    pub fn preset(kind: FixtureKind) -> Self {
        match kind {
            FixtureKind::MinimalEpub3 => Self::epub3(kind, "Minimal EPUB 3")
                .feature("package")
                .feature("nav")
                .add_xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>Hello pagelet.</p>"),
            FixtureKind::Epub2WithNcx => Self::epub2(kind, "EPUB 2 With NCX")
                .feature("opf-2")
                .feature("ncx")
                .add_xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>Legacy nav.</p>"),
            FixtureKind::FallbackChain => Self::epub3(kind, "Fallback Chain")
                .feature("fallback")
                .add_xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>Fallback body.</p>")
                .add_entry(
                    "EPUB/audio/chapter.mp3",
                    "audio/mpeg",
                    b"fake audio fallback".to_vec(),
                ),
            FixtureKind::FootnoteCollision => Self::epub3(kind, "Footnote Collision")
                .feature("footnotes")
                .add_xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r##"<p><a href="#fn1">note</a></p><aside id="fn1">A</aside>"##,
                )
                .add_xhtml(
                    "EPUB/chapter-2.xhtml",
                    "Chapter 2",
                    r##"<p><a href="#fn1">note</a></p><aside id="fn1">B</aside>"##,
                ),
            FixtureKind::PathCaseCollision => Self::epub3(kind, "Path Case Collision")
                .feature("path-case")
                .add_xhtml("EPUB/Text.xhtml", "Upper", "<p>Upper path.</p>")
                .add_xhtml("EPUB/text.xhtml", "Lower", "<p>Lower path.</p>"),
            FixtureKind::CssCascade => Self::epub3(kind, "CSS Cascade")
                .feature("css")
                .add_stylesheet(
                    "EPUB/styles/base.css",
                    "p { margin: 1em; } .lead { font-weight: bold; }",
                )
                .add_xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r#"<p class="lead">Styled text.</p>"#,
                ),
            FixtureKind::MalformedXhtml => Self::epub3(kind, "Malformed XHTML")
                .feature("malformed-xhtml")
                .add_xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>Missing close"),
            FixtureKind::HugeImage => Self::epub3(kind, "Huge Image")
                .feature("huge-image")
                .add_xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r#"<figure><img src="images/huge.png" /></figure>"#,
                )
                .add_entry("EPUB/images/huge.png", "image/png", vec![0; 128 * 1024]),
            FixtureKind::Rtl => Self::epub3(kind, "RTL").feature("rtl").add_xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                r#"<html dir="rtl"><body><p>مرحبا</p></body></html>"#,
            ),
            FixtureKind::DataUri => Self::epub3(kind, "Data URI").feature("data-uri").add_xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                r#"<img src="data:image/png;base64,AAAA" />"#,
            ),
            FixtureKind::DuplicateIds => Self::epub3(kind, "Duplicate IDs")
                .feature("duplicate-ids")
                .add_xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r#"<p id="dup">A</p><p id="dup">B</p>"#,
                ),
            FixtureKind::ZipBombLike => Self::epub3(kind, "Zip Bomb Like")
                .feature("zip-bomb")
                .add_xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    "<p>Compressed risk.</p>",
                )
                .add_entry(
                    "EPUB/repetitive.txt",
                    "text/plain",
                    "0".repeat(512 * 1024).into_bytes(),
                ),
        }
    }

    /// Add an expected feature marker.
    #[must_use]
    pub fn feature(mut self, feature: impl Into<Arc<str>>) -> Self {
        self.expected_features.push(feature.into());
        self
    }

    /// Add an XHTML content document.
    #[must_use]
    pub fn add_xhtml(
        self,
        path: impl Into<Arc<str>>,
        title: impl AsRef<str>,
        body: impl AsRef<str>,
    ) -> Self {
        let title = title.as_ref();
        let body = body.as_ref();
        let content = if body.contains("<html") {
            body.to_owned()
        } else {
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>{}</title></head><body>{}</body></html>"#,
                escape_xml(title),
                body
            )
        };

        self.add_entry(path, "application/xhtml+xml", content.into_bytes())
    }

    /// Add a CSS stylesheet.
    #[must_use]
    pub fn add_stylesheet(self, path: impl Into<Arc<str>>, css: impl AsRef<str>) -> Self {
        self.add_entry(path, "text/css", css.as_ref().as_bytes().to_vec())
    }

    /// Add a raw resource.
    #[must_use]
    pub fn add_entry(
        mut self,
        path: impl Into<Arc<str>>,
        media_type: impl Into<Arc<str>>,
        bytes: Vec<u8>,
    ) -> Self {
        self.entries
            .push(FixtureEntry::new(path, media_type, bytes));
        self
    }

    /// Build deterministic EPUB bytes.
    #[must_use]
    pub fn build(self) -> GeneratedEpubFixture {
        let mut entries = vec![
            FixtureEntry::new("mimetype", "text/plain", b"application/epub+zip".to_vec()),
            FixtureEntry::new(
                "META-INF/container.xml",
                "application/xml",
                container_xml().into_bytes(),
            ),
        ];

        let opf = package_document(&self);
        entries.push(FixtureEntry::new(
            "EPUB/package.opf",
            "application/oebps-package+xml",
            opf.into_bytes(),
        ));

        if self.package_version == EpubPackageVersion::Epub3 {
            entries.push(FixtureEntry::new(
                "EPUB/nav.xhtml",
                "application/xhtml+xml",
                nav_document(&self.title).into_bytes(),
            ));
        } else {
            entries.push(FixtureEntry::new(
                "EPUB/toc.ncx",
                "application/x-dtbncx+xml",
                ncx_document(&self.title).into_bytes(),
            ));
        }

        entries.extend(self.entries);

        let bytes = write_stored_zip(&entries);

        GeneratedEpubFixture {
            kind: self.kind,
            id: self.kind.id().into(),
            expected_features: self.expected_features,
            entries,
            bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum EpubPackageVersion {
    Epub2,
    Epub3,
}

/// Generate a benchmark fixture.
#[must_use]
pub fn benchmark_fixture(kind: BenchmarkFixtureKind) -> GeneratedEpubFixture {
    match kind {
        BenchmarkFixtureKind::TinyText => GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3),
        BenchmarkFixtureKind::SmallNovel => multi_chapter_fixture(kind, 5, 8),
        BenchmarkFixtureKind::LargeNovel => multi_chapter_fixture(kind, 24, 12),
        BenchmarkFixtureKind::ImageHeavy => EpubFixtureBuilder::preset(FixtureKind::HugeImage)
            .feature(kind.id())
            .build(),
        BenchmarkFixtureKind::FootnoteHeavy => {
            EpubFixtureBuilder::preset(FixtureKind::FootnoteCollision)
                .feature(kind.id())
                .build()
        }
        BenchmarkFixtureKind::CssHeavy => EpubFixtureBuilder::preset(FixtureKind::CssCascade)
            .feature(kind.id())
            .build(),
        BenchmarkFixtureKind::CjkRtl => EpubFixtureBuilder::preset(FixtureKind::Rtl)
            .feature(kind.id())
            .add_xhtml("EPUB/cjk.xhtml", "CJK", "<p>你好，pagelet。</p>")
            .build(),
        BenchmarkFixtureKind::Pathological => EpubFixtureBuilder::preset(FixtureKind::ZipBombLike)
            .feature(kind.id())
            .build(),
    }
}

fn multi_chapter_fixture(
    kind: BenchmarkFixtureKind,
    chapters: u32,
    paragraphs_per_chapter: u32,
) -> GeneratedEpubFixture {
    let mut builder = EpubFixtureBuilder::epub3(FixtureKind::MinimalEpub3, kind.id())
        .feature(kind.id())
        .feature("multi-chapter");

    for chapter in 0..chapters {
        let mut body = String::new();
        for paragraph in 0..paragraphs_per_chapter {
            body.push_str(&format!(
                "<p>Chapter {} paragraph {} generated benchmark text.</p>",
                chapter + 1,
                paragraph + 1
            ));
        }
        builder = builder.add_xhtml(
            format!("EPUB/chapter-{}.xhtml", chapter + 1),
            format!("Chapter {}", chapter + 1),
            body,
        );
    }

    builder.build()
}

fn package_document(builder: &EpubFixtureBuilder) -> String {
    let version = match builder.package_version {
        EpubPackageVersion::Epub2 => "2.0",
        EpubPackageVersion::Epub3 => "3.0",
    };

    let mut manifest = String::new();
    let mut spine = String::new();

    if builder.package_version == EpubPackageVersion::Epub3 {
        manifest.push_str(
            r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#,
        );
    } else {
        manifest
            .push_str(r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#);
    }

    for (index, entry) in builder.entries.iter().enumerate() {
        let id = format!("item{}", index + 1);
        let href = entry
            .path
            .strip_prefix("EPUB/")
            .unwrap_or(entry.path.as_ref());
        manifest.push_str(&format!(
            r#"<item id="{id}" href="{}" media-type="{}"/>"#,
            escape_xml(href),
            escape_xml(&entry.media_type)
        ));
        if entry.media_type.as_ref() == "application/xhtml+xml" {
            spine.push_str(&format!(r#"<itemref idref="{id}"/>"#));
        }
    }

    let spine_attrs = if builder.package_version == EpubPackageVersion::Epub2 {
        r#" toc="ncx""#
    } else {
        ""
    };

    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="bookid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="bookid">urn:pagelet:{}</dc:identifier><dc:title>{}</dc:title><dc:language>en</dc:language></metadata><manifest>{manifest}</manifest><spine{spine_attrs}>{spine}</spine></package>"#,
        builder.kind.id(),
        escape_xml(&builder.title)
    )
}

fn container_xml() -> String {
    r#"<?xml version="1.0" encoding="utf-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.to_owned()
}

fn nav_document(title: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>{}</title></head><body><nav epub:type="toc"><ol><li><a href="chapter-1.xhtml">Start</a></li></ol></nav></body></html>"#,
        escape_xml(title)
    )
}

fn ncx_document(title: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><docTitle><text>{}</text></docTitle><navMap><navPoint id="navPoint-1" playOrder="1"><navLabel><text>Start</text></navLabel><content src="chapter-1.xhtml"/></navPoint></navMap></ncx>"#,
        escape_xml(title)
    )
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn write_stored_zip(entries: &[FixtureEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();

    for entry in entries {
        let offset = u32::try_from(out.len()).expect("fixture zip offset fits in u32");
        let name = entry.path.as_bytes();
        let size = u32::try_from(entry.bytes.len()).expect("fixture entry fits in u32");
        let crc = crc32(&entry.bytes);

        write_u32(&mut out, 0x0403_4b50);
        write_u16(&mut out, 20);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u32(&mut out, crc);
        write_u32(&mut out, size);
        write_u32(&mut out, size);
        write_u16(
            &mut out,
            u16::try_from(name.len()).expect("fixture path fits in u16"),
        );
        write_u16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(&entry.bytes);

        write_u32(&mut central, 0x0201_4b50);
        write_u16(&mut central, 20);
        write_u16(&mut central, 20);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, crc);
        write_u32(&mut central, size);
        write_u32(&mut central, size);
        write_u16(
            &mut central,
            u16::try_from(name.len()).expect("fixture path fits in u16"),
        );
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u16(&mut central, 0);
        write_u32(&mut central, 0);
        write_u32(&mut central, offset);
        central.extend_from_slice(name);
    }

    let central_offset = u32::try_from(out.len()).expect("fixture central offset fits in u32");
    let central_size = u32::try_from(central.len()).expect("fixture central size fits in u32");
    out.extend_from_slice(&central);
    write_u32(&mut out, 0x0605_4b50);
    write_u16(&mut out, 0);
    write_u16(&mut out, 0);
    write_u16(
        &mut out,
        u16::try_from(entries.len()).expect("fixture entry count fits in u16"),
    );
    write_u16(
        &mut out,
        u16::try_from(entries.len()).expect("fixture entry count fits in u16"),
    );
    write_u32(&mut out, central_size);
    write_u32(&mut out, central_offset);
    write_u16(&mut out, 0);
    out
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Deterministic fake text backend for layout tests.
#[derive(Debug, Clone, Copy)]
pub struct FakeTextBackend {
    backend_id: TextBackendId,
    font_fingerprint: FontSetFingerprint,
}

impl FakeTextBackend {
    /// Create a fake backend with stable identifiers.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            backend_id: TextBackendId(1),
            font_fingerprint: FontSetFingerprint(1),
        }
    }
}

impl Default for FakeTextBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBackend for FakeTextBackend {
    fn backend_id(&self) -> TextBackendId {
        self.backend_id
    }

    fn font_fingerprint(&self) -> FontSetFingerprint {
        self.font_fingerprint
    }

    fn measure_batch(
        &self,
        request: &MeasureBatch,
        cancel: &CancellationToken,
    ) -> Result<MeasuredBatch, PageletError> {
        let mut results = Vec::with_capacity(request.requests.len());

        for item in &request.requests {
            if cancel.is_cancelled() {
                return Err(PageletError::Cancelled);
            }

            let font_raw = item.font_size.raw().max(LayoutUnit::SCALE);
            let char_width = (font_raw / 2).max(1);
            let line_height = ((font_raw * 6) / 5).max(1);
            let char_count = u32::try_from(item.text.chars().count()).unwrap_or(u32::MAX);
            let max_width_raw = item.max_width.raw().max(char_width);
            let chars_per_line = (max_width_raw / char_width).max(1);
            let line_count =
                u32::try_from((i64::from(char_count) + chars_per_line - 1) / chars_per_line)
                    .unwrap_or(u32::MAX)
                    .max(1);
            let natural_width = i64::from(char_count).saturating_mul(char_width);
            let width = LayoutUnit::from_raw(natural_width.min(max_width_raw));
            let height = LayoutUnit::from_raw(i64::from(line_count).saturating_mul(line_height));

            let (lines, clusters) = fake_lines(
                &item.text,
                LayoutUnit::from_raw(char_width),
                LayoutUnit::from_raw(line_height),
                item.max_width,
            );

            results.push(MeasuredText::new(
                item.id,
                item.request_fingerprint,
                width,
                height,
                u32::try_from(item.text.len()).unwrap_or(u32::MAX),
                lines,
                clusters,
                item.request_fingerprint ^ self.font_fingerprint.0,
            ));
        }

        Ok(MeasuredBatch::new(
            self.backend_id,
            self.font_fingerprint,
            results,
        ))
    }
}

fn fake_lines(
    text: &str,
    char_width: LayoutUnit,
    line_height: LayoutUnit,
    max_width: LayoutUnit,
) -> (Vec<LineMetrics>, Vec<TextCluster>) {
    let chars_per_line = (max_width.raw().max(char_width.raw()) / char_width.raw()).max(1);
    let mut lines = Vec::new();
    let mut clusters = Vec::new();
    let mut line_start = 0_usize;
    let mut line_width = LayoutUnit::ZERO;
    let mut cluster_x = LayoutUnit::ZERO;
    let mut line_index = 0_u32;
    let mut count = 0_i64;

    for (offset, ch) in text.char_indices() {
        if count >= chars_per_line && offset > line_start {
            lines.push(fake_line(line_start, offset, line_width, line_height));
            line_start = offset;
            line_width = LayoutUnit::ZERO;
            cluster_x = LayoutUnit::ZERO;
            line_index = line_index.saturating_add(1);
            count = 0;
        }
        let end = offset + ch.len_utf8();
        clusters.push(TextCluster {
            text_start: u32::try_from(offset).unwrap_or(u32::MAX),
            text_end: u32::try_from(end).unwrap_or(u32::MAX),
            line_index,
            x_start: cluster_x,
            x_end: cluster_x + char_width,
        });
        line_width += char_width;
        cluster_x += char_width;
        count += 1;
    }

    if line_start < text.len() || lines.is_empty() {
        lines.push(fake_line(line_start, text.len(), line_width, line_height));
    }

    (lines, clusters)
}

fn fake_line(start: usize, end: usize, width: LayoutUnit, line_height: LayoutUnit) -> LineMetrics {
    LineMetrics {
        text_start: u32::try_from(start).unwrap_or(u32::MAX),
        text_end: u32::try_from(end).unwrap_or(u32::MAX),
        baseline: LayoutUnit::from_raw((line_height.raw() * 4) / 5),
        ascent: LayoutUnit::from_raw((line_height.raw() * 4) / 5),
        descent: LayoutUnit::from_raw(line_height.raw() / 5),
        line_height,
        width,
        hard_break: false,
    }
}

/// Normalized golden section names.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum GoldenSectionName {
    BookSummary,
    Manifest,
    Spine,
    Navigation,
    Diagnostics,
    ChapterIr,
    VisibleText,
    SourceRanges,
    PageAnchors,
    BreakTokens,
    PageScene,
}

impl GoldenSectionName {
    /// All normalized golden sections.
    pub const ALL: [Self; 11] = [
        Self::BookSummary,
        Self::Manifest,
        Self::Spine,
        Self::Navigation,
        Self::Diagnostics,
        Self::ChapterIr,
        Self::VisibleText,
        Self::SourceRanges,
        Self::PageAnchors,
        Self::BreakTokens,
        Self::PageScene,
    ];

    /// Stable JSON section name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BookSummary => "book_summary",
            Self::Manifest => "manifest",
            Self::Spine => "spine",
            Self::Navigation => "navigation",
            Self::Diagnostics => "diagnostics",
            Self::ChapterIr => "chapter_ir",
            Self::VisibleText => "visible_text",
            Self::SourceRanges => "source_ranges",
            Self::PageAnchors => "page_anchors",
            Self::BreakTokens => "break_tokens",
            Self::PageScene => "page_scene",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.as_str() == value)
    }
}

/// One normalized key/value entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenEntry {
    pub key: Arc<str>,
    pub value: Arc<str>,
}

/// One normalized golden section.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenSection {
    pub name: GoldenSectionName,
    pub entries: Vec<GoldenEntry>,
}

/// Normalized JSON golden document.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NormalizedGolden {
    pub sections: Vec<GoldenSection>,
}

impl NormalizedGolden {
    /// Create a golden document with every section present in stable order.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            sections: GoldenSectionName::ALL
                .into_iter()
                .map(|name| GoldenSection {
                    name,
                    entries: Vec::new(),
                })
                .collect(),
        }
    }

    /// Add a normalized entry.
    #[must_use]
    pub fn with_entry(
        mut self,
        section: GoldenSectionName,
        key: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Self {
        let target = self
            .sections
            .iter_mut()
            .find(|item| item.name == section)
            .expect("all golden sections are present");
        target.entries.push(GoldenEntry {
            key: key.into(),
            value: value.into(),
        });
        target
            .entries
            .sort_by(|left, right| left.key.cmp(&right.key));
        self
    }

    /// Serialize as deterministic normalized JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::from(r#"{"sections":["#);
        for (section_index, section) in self.sections.iter().enumerate() {
            if section_index > 0 {
                out.push(',');
            }
            out.push_str(r#"{"name":""#);
            out.push_str(section.name.as_str());
            out.push_str(r#"","entries":["#);
            for (entry_index, entry) in section.entries.iter().enumerate() {
                if entry_index > 0 {
                    out.push(',');
                }
                out.push_str(r#"{"key":""#);
                out.push_str(&escape_json(&entry.key));
                out.push_str(r#"","value":""#);
                out.push_str(&escape_json(&entry.value));
                out.push_str(r#""}"#);
            }
            out.push_str("]}");
        }
        out.push_str("]}");
        out
    }

    /// Parse JSON produced by [`Self::to_json`].
    pub fn from_json(input: &str) -> Result<Self, GoldenParseError> {
        GoldenParser::new(input).parse()
    }
}

/// Error returned when normalized golden JSON cannot be parsed.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenParseError {
    message: Arc<str>,
}

impl GoldenParseError {
    fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GoldenParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GoldenParseError {}

struct GoldenParser<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> GoldenParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    fn parse(mut self) -> Result<NormalizedGolden, GoldenParseError> {
        self.object_start()?;
        self.key("sections")?;
        self.array_start()?;

        let mut sections = Vec::new();
        if !self.consume(']') {
            loop {
                sections.push(self.section()?);
                if self.consume(']') {
                    break;
                }
                self.char(',')?;
            }
        }

        self.char('}')?;
        self.end()?;
        Ok(NormalizedGolden { sections })
    }

    fn section(&mut self) -> Result<GoldenSection, GoldenParseError> {
        self.object_start()?;
        self.key("name")?;
        let name = self.string()?;
        let name = GoldenSectionName::from_str(&name)
            .ok_or_else(|| GoldenParseError::new(format!("unknown section {name}")))?;
        self.char(',')?;
        self.key("entries")?;
        self.array_start()?;

        let mut entries = Vec::new();
        if !self.consume(']') {
            loop {
                entries.push(self.entry()?);
                if self.consume(']') {
                    break;
                }
                self.char(',')?;
            }
        }

        self.char('}')?;
        Ok(GoldenSection { name, entries })
    }

    fn entry(&mut self) -> Result<GoldenEntry, GoldenParseError> {
        self.object_start()?;
        self.key("key")?;
        let key = self.string()?;
        self.char(',')?;
        self.key("value")?;
        let value = self.string()?;
        self.char('}')?;
        Ok(GoldenEntry {
            key: key.into(),
            value: value.into(),
        })
    }

    fn key(&mut self, expected: &str) -> Result<(), GoldenParseError> {
        let key = self.string()?;
        if key == expected {
            self.char(':')
        } else {
            Err(GoldenParseError::new(format!(
                "expected key {expected}, got {key}"
            )))
        }
    }

    fn object_start(&mut self) -> Result<(), GoldenParseError> {
        self.char('{')
    }

    fn array_start(&mut self) -> Result<(), GoldenParseError> {
        self.char('[')
    }

    fn char(&mut self, expected: char) -> Result<(), GoldenParseError> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(GoldenParseError::new(format!("expected {expected}")))
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        self.skip_ws();
        if self.input[self.cursor..].starts_with(expected) {
            self.cursor += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> Result<String, GoldenParseError> {
        self.char('"')?;
        let mut out = String::new();
        while self.cursor < self.input.len() {
            let ch = self.next_char()?;
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let escaped = self.next_char()?;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        other => {
                            return Err(GoldenParseError::new(format!(
                                "unsupported escape {other}"
                            )));
                        }
                    }
                }
                other => out.push(other),
            }
        }
        Err(GoldenParseError::new("unterminated string"))
    }

    fn next_char(&mut self) -> Result<char, GoldenParseError> {
        let mut chars = self.input[self.cursor..].chars();
        let ch = chars
            .next()
            .ok_or_else(|| GoldenParseError::new("unexpected end"))?;
        self.cursor += ch.len_utf8();
        Ok(ch)
    }

    fn skip_ws(&mut self) {
        while self.cursor < self.input.len() {
            let Some(ch) = self.input[self.cursor..].chars().next() else {
                break;
            };
            if ch.is_whitespace() {
                self.cursor += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn end(&mut self) -> Result<(), GoldenParseError> {
        self.skip_ws();
        if self.cursor == self.input.len() {
            Ok(())
        } else {
            Err(GoldenParseError::new("trailing content"))
        }
    }
}

fn escape_json(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::MeasureRequest;

    #[test]
    fn every_generated_fixture_kind_has_assertions() {
        for kind in FixtureKind::ALL {
            let fixture = GeneratedEpubFixture::preset(kind);

            assert_eq!(fixture.kind, kind);
            assert_eq!(fixture.id.as_ref(), kind.id());
            assert!(fixture.contains_entry("mimetype"));
            assert!(fixture.contains_entry("META-INF/container.xml"));
            assert!(fixture.contains_entry("EPUB/package.opf"));
            assert!(fixture.bytes().starts_with(b"PK\x03\x04"));
            assert!(fixture.bytes().ends_with(&[0, 0]));
            assert!(!fixture.expected_features.is_empty());

            match kind {
                FixtureKind::MinimalEpub3 => {
                    assert_has_feature(&fixture, "package");
                    assert_has_feature(&fixture, "nav");
                    assert!(fixture.contains_entry("EPUB/nav.xhtml"));
                }
                FixtureKind::Epub2WithNcx => {
                    assert_has_feature(&fixture, "opf-2");
                    assert_has_feature(&fixture, "ncx");
                    assert!(fixture.contains_entry("EPUB/toc.ncx"));
                }
                FixtureKind::FallbackChain => {
                    assert_has_feature(&fixture, "fallback");
                    assert!(fixture.contains_entry("EPUB/audio/chapter.mp3"));
                }
                FixtureKind::FootnoteCollision => {
                    assert_has_feature(&fixture, "footnotes");
                    assert!(entry_text(&fixture, "EPUB/chapter-1.xhtml").contains(r##"#fn1"##));
                    assert!(entry_text(&fixture, "EPUB/chapter-2.xhtml").contains(r##"#fn1"##));
                }
                FixtureKind::PathCaseCollision => {
                    assert_has_feature(&fixture, "path-case");
                    assert!(fixture.contains_entry("EPUB/Text.xhtml"));
                    assert!(fixture.contains_entry("EPUB/text.xhtml"));
                }
                FixtureKind::CssCascade => {
                    assert_has_feature(&fixture, "css");
                    assert!(entry_text(&fixture, "EPUB/styles/base.css").contains(".lead"));
                }
                FixtureKind::MalformedXhtml => {
                    assert_has_feature(&fixture, "malformed-xhtml");
                    assert!(entry_text(&fixture, "EPUB/chapter-1.xhtml").contains("Missing close"));
                }
                FixtureKind::HugeImage => {
                    assert_has_feature(&fixture, "huge-image");
                    assert_eq!(
                        entry_bytes(&fixture, "EPUB/images/huge.png").len(),
                        128 * 1024
                    );
                }
                FixtureKind::Rtl => {
                    assert_has_feature(&fixture, "rtl");
                    assert!(entry_text(&fixture, "EPUB/chapter-1.xhtml").contains(r#"dir="rtl""#));
                }
                FixtureKind::DataUri => {
                    assert_has_feature(&fixture, "data-uri");
                    assert!(entry_text(&fixture, "EPUB/chapter-1.xhtml").contains("data:image/png"));
                }
                FixtureKind::DuplicateIds => {
                    assert_has_feature(&fixture, "duplicate-ids");
                    let chapter = entry_text(&fixture, "EPUB/chapter-1.xhtml");
                    assert_eq!(chapter.matches(r#"id="dup""#).count(), 2);
                }
                FixtureKind::ZipBombLike => {
                    assert_has_feature(&fixture, "zip-bomb");
                    assert_eq!(
                        entry_bytes(&fixture, "EPUB/repetitive.txt").len(),
                        512 * 1024
                    );
                }
            }
        }
    }

    #[test]
    fn epub2_fixture_contains_ncx_and_epub3_fixture_contains_nav() {
        let epub2 = GeneratedEpubFixture::preset(FixtureKind::Epub2WithNcx);
        let epub3 = GeneratedEpubFixture::preset(FixtureKind::MinimalEpub3);

        assert!(epub2.contains_entry("EPUB/toc.ncx"));
        assert!(!epub2.contains_entry("EPUB/nav.xhtml"));
        assert!(epub3.contains_entry("EPUB/nav.xhtml"));
    }

    #[test]
    fn benchmark_fixture_definitions_are_available() {
        for kind in BenchmarkFixtureKind::ALL {
            let fixture = benchmark_fixture(kind);

            assert!(fixture
                .expected_features
                .iter()
                .any(|feature| feature.as_ref() == kind.id()
                    || kind == BenchmarkFixtureKind::TinyText));
            assert!(fixture.bytes().len() > 100);
        }
    }

    #[test]
    fn fake_text_backend_is_deterministic() {
        let backend = FakeTextBackend::new();
        let batch = MeasureBatch::new(vec![
            MeasureRequest::new(
                1,
                "deterministic text",
                LayoutUnit::from_px(16),
                LayoutUnit::from_px(80),
            ),
            MeasureRequest::new(
                2,
                "wrapped deterministic text",
                LayoutUnit::from_px(16),
                LayoutUnit::from_px(48),
            ),
        ]);
        let cancel = CancellationToken::new();

        let first = backend.measure_batch(&batch, &cancel).expect("measure");
        let second = backend.measure_batch(&batch, &cancel).expect("measure");

        assert_eq!(backend.backend_id(), TextBackendId(1));
        assert_eq!(backend.font_fingerprint(), FontSetFingerprint(1));
        assert_eq!(first, second);
    }

    #[test]
    fn fake_text_backend_honors_cancellation() {
        let backend = FakeTextBackend::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let batch = MeasureBatch::new(vec![MeasureRequest::new(
            1,
            "cancel",
            LayoutUnit::from_px(16),
            LayoutUnit::from_px(80),
        )]);

        assert_eq!(
            backend.measure_batch(&batch, &cancel),
            Err(PageletError::Cancelled)
        );
    }

    #[test]
    fn normalized_golden_round_trip_preserves_entries() {
        let golden = NormalizedGolden::empty()
            .with_entry(GoldenSectionName::BookSummary, "title", "Example")
            .with_entry(GoldenSectionName::Manifest, "item-1", "chapter.xhtml")
            .with_entry(
                GoldenSectionName::VisibleText,
                "chapter-1",
                "Hello \"pagelet\"",
            )
            .with_entry(
                GoldenSectionName::PageAnchors,
                "page-1",
                "doc=1,node=2,off=3",
            )
            .with_entry(GoldenSectionName::BreakTokens, "page-1-end", "token:1");

        let json = golden.to_json();
        let parsed = NormalizedGolden::from_json(&json).expect("parse normalized golden");

        assert_eq!(parsed, golden);
        assert!(json.contains("book_summary"));
        assert!(json.contains("page_scene"));
    }

    fn assert_has_feature(fixture: &GeneratedEpubFixture, expected: &str) {
        assert!(fixture
            .expected_features
            .iter()
            .any(|feature| feature.as_ref() == expected));
    }

    fn entry_bytes<'a>(fixture: &'a GeneratedEpubFixture, path: &str) -> &'a [u8] {
        fixture
            .entries()
            .iter()
            .find(|entry| entry.path.as_ref() == path)
            .map(|entry| entry.bytes.as_slice())
            .unwrap_or_else(|| panic!("missing fixture entry {path}"))
    }

    fn entry_text(fixture: &GeneratedEpubFixture, path: &str) -> String {
        String::from_utf8(entry_bytes(fixture, path).to_vec()).expect("fixture text is utf-8")
    }
}
