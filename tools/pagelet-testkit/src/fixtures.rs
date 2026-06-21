use std::sync::Arc;

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

/// Deterministic generated fixture used by integration tests and xtask.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Fixture {
    /// Stable case id.
    pub id: Arc<str>,
    /// Entries in deterministic order.
    pub entries: Vec<FixtureEntry>,
}

impl Fixture {
    /// Serialize the fixture into stable bytes for snapshot smoke tests.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::from("PAGELET-FIXTURE\n");
        out.extend_from_slice(self.id.as_bytes());
        out.push(b'\n');
        for entry in &self.entries {
            out.extend_from_slice(entry.path.as_bytes());
            out.push(b'\0');
            out.extend_from_slice(entry.media_type.as_bytes());
            out.push(b'\0');
            out.extend_from_slice(entry.bytes.len().to_string().as_bytes());
            out.push(b'\n');
            out.extend_from_slice(&entry.bytes);
            out.push(b'\n');
        }
        out
    }
}

/// Builder for deterministic generated fixtures.
#[derive(Debug, Clone)]
pub struct FixtureBuilder {
    id: Arc<str>,
    entries: Vec<FixtureEntry>,
}

impl FixtureBuilder {
    /// Create a fixture builder.
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            entries: Vec::new(),
        }
    }

    /// Add a raw entry.
    #[must_use]
    pub fn entry(mut self, entry: FixtureEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// Add a UTF-8 text entry.
    #[must_use]
    pub fn text_entry(
        self,
        path: impl Into<Arc<str>>,
        media_type: impl Into<Arc<str>>,
        text: impl AsRef<str>,
    ) -> Self {
        self.entry(FixtureEntry::new(
            path,
            media_type,
            text.as_ref().as_bytes().to_vec(),
        ))
    }

    /// Build a minimal EPUB-like structural fixture.
    #[must_use]
    pub fn minimal_epub3(id: impl Into<Arc<str>>) -> Self {
        Self::new(id)
            .text_entry("mimetype", "text/plain", "application/epub+zip")
            .text_entry(
                "META-INF/container.xml",
                "application/xml",
                r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="EPUB/package.opf"/></rootfiles></container>"#,
            )
            .text_entry(
                "EPUB/package.opf",
                "application/oebps-package+xml",
                r#"<?xml version="1.0"?><package version="3.0"><manifest/><spine/></package>"#,
            )
            .text_entry(
                "EPUB/nav.xhtml",
                "application/xhtml+xml",
                r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><nav><ol/></nav></body></html>"#,
            )
    }

    /// Build the fixture.
    #[must_use]
    pub fn build(mut self) -> Fixture {
        self.entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        Fixture {
            id: self.id,
            entries: self.entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_bytes_are_deterministic() {
        let first = FixtureBuilder::minimal_epub3("minimal").build().to_bytes();
        let second = FixtureBuilder::minimal_epub3("minimal").build().to_bytes();

        assert_eq!(first, second);
        assert!(first.starts_with(b"PAGELET-FIXTURE\nminimal\n"));
    }
}
