use std::sync::Arc;

/// One normalized golden entry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenEntry {
    /// Stable key inside a section.
    pub key: Arc<str>,
    /// Normalized value.
    pub value: Arc<str>,
}

impl GoldenEntry {
    /// Create a golden entry.
    #[must_use]
    pub fn new(key: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// One normalized golden section.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenSection {
    /// Stable section name.
    pub name: Arc<str>,
    /// Entries sorted by key.
    pub entries: Vec<GoldenEntry>,
}

impl GoldenSection {
    /// Create a golden section.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    /// Add an entry and keep deterministic ordering.
    #[must_use]
    pub fn entry(mut self, key: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        self.entries.push(GoldenEntry::new(key, value));
        self.entries.sort_by(|left, right| left.key.cmp(&right.key));
        self
    }
}

/// Deterministic normalized golden document.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenDocument {
    /// Sections sorted by name.
    pub sections: Vec<GoldenSection>,
}

impl GoldenDocument {
    /// Create an empty golden document.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// Add a section and keep deterministic ordering.
    #[must_use]
    pub fn section(mut self, section: GoldenSection) -> Self {
        self.sections.push(section);
        self.sections
            .sort_by(|left, right| left.name.cmp(&right.name));
        self
    }

    /// Serialize as stable line-oriented text.
    #[must_use]
    pub fn to_normalized_text(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            out.push('[');
            out.push_str(&section.name);
            out.push_str("]\n");
            for entry in &section.entries {
                out.push_str(&entry.key);
                out.push('=');
                out.push_str(&entry.value);
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_document_sorts_sections_and_entries() {
        let golden = GoldenDocument::empty()
            .section(
                GoldenSection::new("manifest")
                    .entry("b", "second")
                    .entry("a", "first"),
            )
            .section(GoldenSection::new("book").entry("title", "Example"));

        assert_eq!(
            golden.to_normalized_text(),
            "[book]\ntitle=Example\n[manifest]\na=first\nb=second\n"
        );
    }
}
