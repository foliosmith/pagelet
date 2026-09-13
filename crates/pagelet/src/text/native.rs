//! Feature-gated OpenType font loading for the native text backend.

use crate::{
    core::{ContentHash, PageletError, ParseError},
    text::FontSetFingerprint,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
/// Font-wide metrics read from required OpenType tables.
pub struct OpenTypeMetrics {
    /// Design units per em.
    pub units_per_em: u16,
    /// Recommended typographic ascender in design units.
    pub ascender: i16,
    /// Recommended typographic descender in design units.
    pub descender: i16,
    /// Recommended additional line gap in design units.
    pub line_gap: i16,
    /// Number of glyphs declared by the font.
    pub glyph_count: u16,
}

#[derive(Debug)]
/// Validated OpenType face borrowing its source bytes.
pub struct OpenTypeFont<'a> {
    face: ttf_parser::Face<'a>,
    fingerprint: FontSetFingerprint,
}

impl<'a> OpenTypeFont<'a> {
    /// Parse one face from OpenType font or collection bytes.
    pub fn parse(bytes: &'a [u8], face_index: u32) -> Result<Self, PageletError> {
        let face = ttf_parser::Face::parse(bytes, face_index)
            .map_err(|_| PageletError::Parse(ParseError::new("invalid OpenType font")))?;
        let mut fingerprint = [0_u8; 8];
        fingerprint.copy_from_slice(&ContentHash::from_bytes(bytes).as_bytes()[..8]);
        Ok(Self {
            face,
            fingerprint: FontSetFingerprint(
                u64::from_le_bytes(fingerprint) ^ u64::from(face_index).rotate_left(32),
            ),
        })
    }

    /// Stable fingerprint covering the source bytes and collection face index.
    #[must_use]
    pub const fn fingerprint(&self) -> FontSetFingerprint {
        self.fingerprint
    }

    /// Read the font-wide metrics required by future shaping and scaling.
    #[must_use]
    pub fn metrics(&self) -> OpenTypeMetrics {
        OpenTypeMetrics {
            units_per_em: self.face.units_per_em(),
            ascender: self.face.ascender(),
            descender: self.face.descender(),
            line_gap: self.face.line_gap(),
            glyph_count: self.face.number_of_glyphs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_required_opentype_tables_and_fingerprints_bytes() {
        let bytes = minimal_font();
        let font = OpenTypeFont::parse(&bytes, 0).expect("font");

        assert_eq!(
            font.metrics(),
            OpenTypeMetrics {
                units_per_em: 1_000,
                ascender: 800,
                descender: -200,
                line_gap: 20,
                glyph_count: 1,
            }
        );
        assert_eq!(
            font.fingerprint(),
            OpenTypeFont::parse(&bytes, 0)
                .expect("same font")
                .fingerprint()
        );
        let mut changed = bytes.clone();
        changed.push(0);
        assert_ne!(
            font.fingerprint(),
            OpenTypeFont::parse(&changed, 0)
                .expect("changed font")
                .fingerprint()
        );
        assert!(OpenTypeFont::parse(b"not a font", 0).is_err());
        assert!(OpenTypeFont::parse(&bytes, 1).is_err());
    }

    fn minimal_font() -> Vec<u8> {
        let mut head = vec![0_u8; 54];
        head[18..20].copy_from_slice(&1_000_u16.to_be_bytes());
        let mut hhea = vec![0_u8; 36];
        hhea[4..6].copy_from_slice(&800_i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200_i16).to_be_bytes());
        hhea[8..10].copy_from_slice(&20_i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&1_u16.to_be_bytes());
        let mut maxp = Vec::from(0x0000_5000_u32.to_be_bytes());
        maxp.extend_from_slice(&1_u16.to_be_bytes());

        let tables = [(b"head", head), (b"hhea", hhea), (b"maxp", maxp)];
        let directory_len = 12 + tables.len() * 16;
        let mut bytes = Vec::with_capacity(directory_len + 96);
        bytes.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
        bytes.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&[0_u8; 6]);
        let mut offset = directory_len;
        for (tag, table) in &tables {
            bytes.extend_from_slice(*tag);
            bytes.extend_from_slice(&[0_u8; 4]);
            bytes.extend_from_slice(&(offset as u32).to_be_bytes());
            bytes.extend_from_slice(&(table.len() as u32).to_be_bytes());
            offset = (offset + table.len() + 3) & !3;
        }
        for (_, table) in tables {
            bytes.extend_from_slice(&table);
            while bytes.len() % 4 != 0 {
                bytes.push(0);
            }
        }
        bytes
    }
}
