use std::sync::Arc;

/// Generated fixture category used by the testkit presets.
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
    /// All generated fixture presets.
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

/// Single point mutation that turns a valid fixture into a targeted defect.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum EpubMutation {
    MalformedContainerXml,
    MissingPackageDocument,
    DuplicateResourcePath,
    PathCaseCollision,
    DuplicateIds,
    MalformedXhtml,
    ZipBombLikeResource,
}

/// One deterministic fixture resource.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FixtureEntry {
    /// Stable entry path.
    pub path: Arc<str>,
    /// Media type used by the fixture manifest.
    pub media_type: Arc<str>,
    /// Entry bytes.
    pub bytes: Vec<u8>,
}

impl FixtureEntry {
    /// Create a fixture entry.
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

/// Deterministic generated EPUB fixture.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Fixture {
    /// Stable case id.
    pub id: Arc<str>,
    /// Expected feature markers.
    pub features: Vec<Arc<str>>,
    /// Entries in deterministic order.
    pub entries: Vec<FixtureEntry>,
    bytes: Vec<u8>,
}

impl Fixture {
    /// Deterministic EPUB ZIP bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return true when an archive entry exists.
    #[must_use]
    pub fn contains_entry(&self, path: &str) -> bool {
        self.entries.iter().any(|entry| entry.path.as_ref() == path)
    }

    /// Return one entry by path.
    #[must_use]
    pub fn entry(&self, path: &str) -> Option<&FixtureEntry> {
        self.entries
            .iter()
            .find(|entry| entry.path.as_ref() == path)
    }
}

/// Builder that always emits a structurally valid EPUB package.
#[derive(Debug, Clone)]
pub struct ValidEpubBuilder {
    id: Arc<str>,
    title: Arc<str>,
    package_version: PackageVersion,
    entries: Vec<FixtureEntry>,
    features: Vec<Arc<str>>,
}

impl ValidEpubBuilder {
    /// Create an EPUB 3 builder.
    #[must_use]
    pub fn epub3(id: impl Into<Arc<str>>) -> Self {
        let id = id.into();
        Self {
            title: id.clone(),
            id,
            package_version: PackageVersion::Epub3,
            entries: Vec::new(),
            features: vec!["package".into(), "nav".into()],
        }
    }

    /// Create an EPUB 2 builder with NCX navigation.
    #[must_use]
    pub fn epub2(id: impl Into<Arc<str>>) -> Self {
        let id = id.into();
        Self {
            title: id.clone(),
            id,
            package_version: PackageVersion::Epub2,
            entries: Vec::new(),
            features: vec!["opf-2".into(), "ncx".into()],
        }
    }

    /// Return a named fixture preset.
    #[must_use]
    pub fn preset(kind: FixtureKind) -> Self {
        match kind {
            FixtureKind::MinimalEpub3 => Self::epub3(kind.id()).xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                "<p>Hello pagelet.</p>",
            ),
            FixtureKind::Epub2WithNcx => Self::epub2(kind.id()).xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                "<p>Legacy nav.</p>",
            ),
            FixtureKind::FallbackChain => Self::epub3(kind.id())
                .feature("fallback")
                .xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    "<p>Fallback body.</p>",
                )
                .entry(FixtureEntry::new(
                    "EPUB/audio/chapter.mp3",
                    "audio/mpeg",
                    b"fake audio fallback".to_vec(),
                )),
            FixtureKind::FootnoteCollision => Self::epub3(kind.id())
                .feature("footnotes")
                .xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r##"<p><a href="#fn1">note</a></p><aside id="fn1">A</aside>"##,
                )
                .xhtml(
                    "EPUB/chapter-2.xhtml",
                    "Chapter 2",
                    r##"<p><a href="#fn1">note</a></p><aside id="fn1">B</aside>"##,
                ),
            FixtureKind::PathCaseCollision => Self::epub3(kind.id())
                .feature("path-case")
                .xhtml("EPUB/Text.xhtml", "Upper", "<p>Upper path.</p>")
                .xhtml("EPUB/text.xhtml", "Lower", "<p>Lower path.</p>"),
            FixtureKind::CssCascade => Self::epub3(kind.id())
                .feature("css")
                .stylesheet(
                    "EPUB/styles/base.css",
                    "p { margin: 1em; } .lead { font-weight: bold; }",
                )
                .xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r#"<p class="lead">Styled text.</p>"#,
                ),
            FixtureKind::MalformedXhtml => Self::epub3(kind.id())
                .feature("malformed-xhtml")
                .xhtml("EPUB/chapter-1.xhtml", "Chapter 1", "<p>Missing close"),
            FixtureKind::HugeImage => Self::epub3(kind.id())
                .feature("huge-image")
                .xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    r#"<figure><img src="images/huge.png" /></figure>"#,
                )
                .entry(FixtureEntry::new(
                    "EPUB/images/huge.png",
                    "image/png",
                    vec![0; 128 * 1024],
                )),
            FixtureKind::Rtl => Self::epub3(kind.id()).feature("rtl").xhtml_raw(
                "EPUB/chapter-1.xhtml",
                r#"<html xmlns="http://www.w3.org/1999/xhtml" dir="rtl"><body><p>مرحبا</p></body></html>"#,
            ),
            FixtureKind::DataUri => Self::epub3(kind.id()).feature("data-uri").xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                r#"<img src="data:image/png;base64,AAAA" />"#,
            ),
            FixtureKind::DuplicateIds => Self::epub3(kind.id()).feature("duplicate-ids").xhtml(
                "EPUB/chapter-1.xhtml",
                "Chapter 1",
                r#"<p id="dup">A</p><p id="dup">B</p>"#,
            ),
            FixtureKind::ZipBombLike => Self::epub3(kind.id())
                .feature("zip-bomb")
                .xhtml(
                    "EPUB/chapter-1.xhtml",
                    "Chapter 1",
                    "<p>Compressed risk.</p>",
                )
                .entry(FixtureEntry::new(
                    "EPUB/repetitive.txt",
                    "text/plain",
                    "0".repeat(512 * 1024).into_bytes(),
                )),
        }
    }

    /// Add a feature marker.
    #[must_use]
    pub fn feature(mut self, feature: impl Into<Arc<str>>) -> Self {
        self.features.push(feature.into());
        self
    }

    /// Add a valid XHTML content document.
    #[must_use]
    pub fn xhtml(self, path: impl Into<Arc<str>>, title: &str, body: &str) -> Self {
        let content = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>{}</title></head><body>{body}</body></html>"#,
            escape_xml(title),
        );
        self.entry(FixtureEntry::new(path, "application/xhtml+xml", content))
    }

    /// Add a raw XHTML content document.
    #[must_use]
    pub fn xhtml_raw(self, path: impl Into<Arc<str>>, raw: &str) -> Self {
        self.entry(FixtureEntry::new(
            path,
            "application/xhtml+xml",
            raw.as_bytes().to_vec(),
        ))
    }

    /// Add a CSS stylesheet.
    #[must_use]
    pub fn stylesheet(self, path: impl Into<Arc<str>>, css: &str) -> Self {
        self.entry(FixtureEntry::new(path, "text/css", css.as_bytes().to_vec()))
    }

    /// Add a raw resource.
    #[must_use]
    pub fn entry(mut self, entry: FixtureEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Build deterministic EPUB bytes.
    #[must_use]
    pub fn build(self) -> Fixture {
        let mut raw = RawEpubBuilder::new(self.id.clone())
            .feature_many(self.features.clone())
            .entry(FixtureEntry::new(
                "mimetype",
                "text/plain",
                b"application/epub+zip".to_vec(),
            ))
            .entry(FixtureEntry::new(
                "META-INF/container.xml",
                "application/xml",
                container_xml().into_bytes(),
            ))
            .entry(FixtureEntry::new(
                "EPUB/package.opf",
                "application/oebps-package+xml",
                package_document(&self).into_bytes(),
            ));

        raw = match self.package_version {
            PackageVersion::Epub2 => raw.entry(FixtureEntry::new(
                "EPUB/toc.ncx",
                "application/x-dtbncx+xml",
                ncx_document(&self.title).into_bytes(),
            )),
            PackageVersion::Epub3 => raw.entry(FixtureEntry::new(
                "EPUB/nav.xhtml",
                "application/xhtml+xml",
                nav_document(&self.title).into_bytes(),
            )),
        };

        for entry in self.entries {
            raw = raw.entry(entry);
        }

        raw.build()
    }
}

/// Builder that permits arbitrary archive/XML entries, including invalid cases.
#[derive(Debug, Clone)]
pub struct RawEpubBuilder {
    id: Arc<str>,
    features: Vec<Arc<str>>,
    entries: Vec<FixtureEntry>,
}

impl RawEpubBuilder {
    /// Create a raw fixture builder.
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            features: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Add a feature marker.
    #[must_use]
    pub fn feature(mut self, feature: impl Into<Arc<str>>) -> Self {
        self.features.push(feature.into());
        self
    }

    fn feature_many(mut self, features: Vec<Arc<str>>) -> Self {
        self.features.extend(features);
        self
    }

    /// Add an arbitrary entry.
    #[must_use]
    pub fn entry(mut self, entry: FixtureEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Add an invalid XML entry.
    #[must_use]
    pub fn malformed_xml(self, path: impl Into<Arc<str>>) -> Self {
        self.entry(FixtureEntry::new(
            path,
            "application/xml",
            b"<broken".to_vec(),
        ))
    }

    /// Build deterministic ZIP bytes from entries exactly as provided.
    #[must_use]
    pub fn build(mut self) -> Fixture {
        self.entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        let bytes = write_stored_zip(&self.entries);
        Fixture {
            id: self.id,
            features: self.features,
            entries: self.entries,
            bytes,
        }
    }
}

/// Mutates a valid EPUB fixture by applying a single targeted defect.
#[derive(Debug, Clone)]
pub struct EpubMutator {
    base: Fixture,
}

impl EpubMutator {
    /// Create a mutator from a valid fixture.
    #[must_use]
    pub fn new(base: Fixture) -> Self {
        Self { base }
    }

    /// Apply one mutation and rebuild deterministic bytes.
    #[must_use]
    pub fn apply(mut self, mutation: EpubMutation) -> Fixture {
        match mutation {
            EpubMutation::MalformedContainerXml => {
                replace_or_push(
                    &mut self.base.entries,
                    FixtureEntry::new("META-INF/container.xml", "application/xml", b"<container"),
                );
                self.base.features.push("malformed-container".into());
            }
            EpubMutation::MissingPackageDocument => {
                self.base
                    .entries
                    .retain(|entry| entry.path.as_ref() != "EPUB/package.opf");
                self.base.features.push("missing-package".into());
            }
            EpubMutation::DuplicateResourcePath => {
                self.base.entries.push(FixtureEntry::new(
                    "EPUB/chapter-1.xhtml",
                    "application/xhtml+xml",
                    b"<html><body>duplicate</body></html>".to_vec(),
                ));
                self.base.features.push("duplicate-path".into());
            }
            EpubMutation::PathCaseCollision => {
                self.base.entries.push(FixtureEntry::new(
                    "EPUB/Chapter-1.xhtml",
                    "application/xhtml+xml",
                    b"<html><body>case collision</body></html>".to_vec(),
                ));
                self.base.features.push("path-case".into());
            }
            EpubMutation::DuplicateIds => {
                replace_or_push(
                    &mut self.base.entries,
                    FixtureEntry::new(
                        "EPUB/chapter-1.xhtml",
                        "application/xhtml+xml",
                        r#"<html><body><p id="dup">A</p><p id="dup">B</p></body></html>"#,
                    ),
                );
                self.base.features.push("duplicate-ids".into());
            }
            EpubMutation::MalformedXhtml => {
                replace_or_push(
                    &mut self.base.entries,
                    FixtureEntry::new(
                        "EPUB/chapter-1.xhtml",
                        "application/xhtml+xml",
                        b"<p>broken",
                    ),
                );
                self.base.features.push("malformed-xhtml".into());
            }
            EpubMutation::ZipBombLikeResource => {
                self.base.entries.push(FixtureEntry::new(
                    "EPUB/repetitive.txt",
                    "text/plain",
                    "0".repeat(512 * 1024).into_bytes(),
                ));
                self.base.features.push("zip-bomb".into());
            }
        }

        self.base.bytes = write_stored_zip(&self.base.entries);
        self.base
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PackageVersion {
    Epub2,
    Epub3,
}

fn replace_or_push(entries: &mut Vec<FixtureEntry>, replacement: FixtureEntry) {
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.path == replacement.path)
    {
        *entry = replacement;
    } else {
        entries.push(replacement);
    }
}

fn package_document(builder: &ValidEpubBuilder) -> String {
    let version = match builder.package_version {
        PackageVersion::Epub2 => "2.0",
        PackageVersion::Epub3 => "3.0",
    };
    let mut manifest = String::new();
    let mut spine = String::new();

    if builder.package_version == PackageVersion::Epub3 {
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

    let spine_attrs = if builder.package_version == PackageVersion::Epub2 {
        r#" toc="ncx""#
    } else {
        ""
    };

    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="bookid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="bookid">urn:pagelet:{}</dc:identifier><dc:title>{}</dc:title><dc:language>en</dc:language></metadata><manifest>{manifest}</manifest><spine{spine_attrs}>{spine}</spine></package>"#,
        builder.id,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_presets_cover_required_fixture_kinds() {
        for kind in FixtureKind::ALL {
            let fixture = ValidEpubBuilder::preset(kind).build();

            assert_eq!(fixture.id.as_ref(), kind.id());
            assert!(fixture.bytes().starts_with(b"PK\x03\x04"));
            assert!(fixture.contains_entry("mimetype"));
            assert!(fixture.contains_entry("META-INF/container.xml"));
            assert!(fixture.contains_entry("EPUB/package.opf"));
        }
    }

    #[test]
    fn raw_builder_can_create_invalid_xml_fixture() {
        let fixture = RawEpubBuilder::new("invalid")
            .malformed_xml("META-INF/container.xml")
            .build();

        assert_eq!(
            fixture
                .entry("META-INF/container.xml")
                .expect("container")
                .bytes,
            b"<broken"
        );
    }

    #[test]
    fn mutator_applies_single_targeted_defect() {
        let valid = ValidEpubBuilder::preset(FixtureKind::MinimalEpub3).build();
        let mutated = EpubMutator::new(valid).apply(EpubMutation::MissingPackageDocument);

        assert!(!mutated.contains_entry("EPUB/package.opf"));
        assert!(mutated
            .features
            .iter()
            .any(|feature| feature.as_ref() == "missing-package"));
    }
}
