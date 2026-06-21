use std::{fmt, sync::Arc};

/// Normalized golden sections shared by structural and layout tests.
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
    /// All normalized sections in stable order.
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

    /// Stable JSON field name.
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
            value: normalize_value(&value.into()),
        }
    }
}

/// One normalized golden section.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenSection {
    /// Stable section name.
    pub name: GoldenSectionName,
    /// Entries sorted by key.
    pub entries: Vec<GoldenEntry>,
}

/// Deterministic normalized golden document.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GoldenDocument {
    /// Sections sorted by the canonical section order.
    pub sections: Vec<GoldenSection>,
}

impl GoldenDocument {
    /// Create an empty golden document with every section present.
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

    /// Add an entry and keep deterministic ordering.
    #[must_use]
    pub fn entry(
        mut self,
        section: GoldenSectionName,
        key: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Self {
        let section = self
            .sections
            .iter_mut()
            .find(|candidate| candidate.name == section)
            .expect("all sections are present");
        section.entries.push(GoldenEntry::new(key, value));
        section
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
        out.push_str("]}\n");
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

fn normalize_value(value: &Arc<str>) -> Arc<str> {
    let mut out = String::new();
    for token in value.split_whitespace() {
        if token.starts_with("0x") && token.len() >= 6 {
            out.push_str("<addr>");
        } else if token.contains('/') && token.starts_with('/') {
            out.push_str("<path>");
        } else if looks_like_timestamp(token) {
            out.push_str("<timestamp>");
        } else if is_float_literal(token) {
            let value = token
                .parse::<f64>()
                .expect("float literal predicate only accepts parseable values");
            out.push_str(&format!("{value:.3}"));
        } else {
            out.push_str(token);
        }
        out.push(' ');
    }
    if out.is_empty() {
        value.clone()
    } else {
        out.pop();
        out.into()
    }
}

fn looks_like_timestamp(token: &str) -> bool {
    token.len() >= 20
        && token.as_bytes().get(4) == Some(&b'-')
        && token.as_bytes().get(7) == Some(&b'-')
        && token.contains('T')
}

fn is_float_literal(token: &str) -> bool {
    token.contains(['.', 'e', 'E']) && token.parse::<f64>().is_ok()
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

struct GoldenParser<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> GoldenParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }

    fn parse(mut self) -> Result<GoldenDocument, GoldenParseError> {
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
        Ok(GoldenDocument { sections })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_document_round_trips_and_normalizes() {
        let golden = GoldenDocument::empty()
            .entry(GoldenSectionName::BookSummary, "title", "Example")
            .entry(GoldenSectionName::Manifest, "b", "/tmp/private/book.epub")
            .entry(GoldenSectionName::Manifest, "a", "3.14159")
            .entry(
                GoldenSectionName::Diagnostics,
                "diag",
                "0x1234abcd 2026-06-21T10:00:00Z",
            );

        let json = golden.to_json();
        let parsed = GoldenDocument::from_json(&json).expect("parse golden");

        assert_eq!(parsed, golden);
        assert!(json.contains(r#""a","value":"3.142""#));
        assert!(json.contains("<path>"));
        assert!(json.contains("<addr> <timestamp>"));
    }
}
