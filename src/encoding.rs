//! Text encoding detection, decoding, and encoding.
//!
//! Supported: UTF-8 (with or without BOM), UTF-16 LE/BE (with or without BOM),
//! and Windows-1252 as the legacy single-byte fallback. Decoding never hides
//! damage: every undecodable sequence is counted and the first offset is kept
//! so the editor can warn before a save would make the replacement permanent.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
}

impl TextEncoding {
    pub const ALL: [TextEncoding; 5] = [
        TextEncoding::Utf8,
        TextEncoding::Utf8Bom,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
        TextEncoding::Windows1252,
    ];

    /// Human-readable name for menus and the status bar.
    pub fn label(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8Bom => "UTF-8 with BOM",
            TextEncoding::Utf16Le => "UTF-16 LE",
            TextEncoding::Utf16Be => "UTF-16 BE",
            TextEncoding::Windows1252 => "Windows-1252",
        }
    }

    /// Stable identifier used in the recovery journal and the CLI.
    pub fn id(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "utf-8",
            TextEncoding::Utf8Bom => "utf-8-bom",
            TextEncoding::Utf16Le => "utf-16le",
            TextEncoding::Utf16Be => "utf-16be",
            TextEncoding::Windows1252 => "windows-1252",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|enc| enc.id() == id)
    }

    /// Whether every Unicode scalar value can be written in this encoding.
    pub fn is_unicode(self) -> bool {
        !matches!(self, TextEncoding::Windows1252)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: TextEncoding,
    /// Number of byte sequences that could not be decoded and were shown as U+FFFD.
    pub invalid_sequences: usize,
    /// Byte offset of the first undecodable sequence.
    pub first_invalid_offset: Option<usize>,
    /// The decoded text contains NUL characters (often a sign of a binary file).
    pub contains_nul: bool,
    /// The input started with the byte-order mark of `encoding`.
    pub had_bom: bool,
}

impl Decoded {
    pub fn is_lossless(&self) -> bool {
        self.invalid_sequences == 0
    }
}

/// Characters that the target encoding cannot represent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unrepresentable {
    pub encoding: TextEncoding,
    pub count: usize,
    pub first: char,
    pub first_char_index: usize,
}

impl std::fmt::Display for Unrepresentable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} character(s) cannot be written as {} (first: {:?} at character {})",
            self.count,
            self.encoding.label(),
            self.first,
            self.first_char_index + 1
        )
    }
}

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

/// Detect the encoding of `bytes` and decode them.
///
/// Order: BOM, then strict UTF-8, then BOM-less UTF-16 (by NUL-byte pattern),
/// then "mostly UTF-8 with damage" (reported, not hidden), then Windows-1252.
pub fn detect_and_decode(bytes: &[u8]) -> Decoded {
    if bytes.starts_with(UTF8_BOM) {
        return decode_as(bytes, TextEncoding::Utf8Bom);
    }
    if bytes.starts_with(UTF16LE_BOM) {
        return decode_as(bytes, TextEncoding::Utf16Le);
    }
    if bytes.starts_with(UTF16BE_BOM) {
        return decode_as(bytes, TextEncoding::Utf16Be);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        // Valid UTF-8 might still be BOM-less UTF-16 made of ASCII (NUL-heavy).
        if let Some(enc) = sniff_utf16(bytes) {
            return decode_as(bytes, enc);
        }
        return Decoded {
            contains_nul: text.contains('\0'),
            text: text.to_owned(),
            encoding: TextEncoding::Utf8,
            invalid_sequences: 0,
            first_invalid_offset: None,
            had_bom: false,
        };
    }
    if let Some(enc) = sniff_utf16(bytes) {
        return decode_as(bytes, enc);
    }
    // Invalid UTF-8. If the file still contains real multi-byte UTF-8 sequences
    // it is most likely damaged UTF-8; decoding it as Windows-1252 would turn
    // every accented letter into mojibake. Report the damage instead.
    let has_multibyte = bytes
        .utf8_chunks()
        .any(|chunk| chunk.valid().chars().any(|c| c as u32 >= 0x80));
    if has_multibyte {
        decode_as(bytes, TextEncoding::Utf8)
    } else {
        decode_as(bytes, TextEncoding::Windows1252)
    }
}

/// Decode `bytes` with an explicit encoding ("Reopen with encoding").
/// A matching BOM is skipped; damage is counted rather than hidden.
pub fn decode_as(bytes: &[u8], encoding: TextEncoding) -> Decoded {
    let (text, invalid, first) = match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => {
            let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
            let skipped = bytes.len() - body.len();
            let (text, invalid, first) = decode_utf8_lossy_counted(body);
            (text, invalid, first.map(|offset| offset + skipped))
        }
        TextEncoding::Utf16Le => decode_utf16(bytes, UTF16LE_BOM, u16::from_le_bytes),
        TextEncoding::Utf16Be => decode_utf16(bytes, UTF16BE_BOM, u16::from_be_bytes),
        TextEncoding::Windows1252 => (bytes.iter().map(|&b| cp1252_to_char(b)).collect(), 0, None),
    };
    // A file opened as plain UTF-8 that starts with a BOM is really UTF-8+BOM.
    let encoding = if encoding == TextEncoding::Utf8 && bytes.starts_with(UTF8_BOM) {
        TextEncoding::Utf8Bom
    } else {
        encoding
    };
    let had_bom = match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => bytes.starts_with(UTF8_BOM),
        TextEncoding::Utf16Le => bytes.starts_with(UTF16LE_BOM),
        TextEncoding::Utf16Be => bytes.starts_with(UTF16BE_BOM),
        TextEncoding::Windows1252 => false,
    };
    Decoded {
        contains_nul: text.contains('\0'),
        text,
        encoding,
        invalid_sequences: invalid,
        first_invalid_offset: first,
        had_bom,
    }
}

/// Encode `text` for writing. Fails (writing nothing) if a character cannot be
/// represented, so a save can never silently replace characters with `?`.
///
/// `utf16_bom` controls whether UTF-16 output starts with a byte-order mark
/// (kept as found when a file is reopened and saved). The UTF-8 BOM is part of
/// the encoding choice itself (`Utf8` vs `Utf8Bom`).
pub fn encode(
    text: &str,
    encoding: TextEncoding,
    utf16_bom: bool,
) -> Result<Vec<u8>, Unrepresentable> {
    match encoding {
        TextEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        TextEncoding::Utf8Bom => {
            let mut out = Vec::with_capacity(text.len() + 3);
            out.extend_from_slice(UTF8_BOM);
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let le = encoding == TextEncoding::Utf16Le;
            let mut out = Vec::with_capacity(text.len() * 2 + 2);
            if utf16_bom {
                out.extend_from_slice(if le { UTF16LE_BOM } else { UTF16BE_BOM });
            }
            for unit in text.encode_utf16() {
                out.extend_from_slice(&if le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            Ok(out)
        }
        TextEncoding::Windows1252 => {
            let mut out = Vec::with_capacity(text.len());
            let mut bad: Option<(char, usize)> = None;
            let mut count = 0;
            for (index, c) in text.chars().enumerate() {
                match char_to_cp1252(c) {
                    Some(b) => out.push(b),
                    None => {
                        count += 1;
                        bad.get_or_insert((c, index));
                    }
                }
            }
            match bad {
                None => Ok(out),
                Some((first, first_char_index)) => Err(Unrepresentable {
                    encoding,
                    count,
                    first,
                    first_char_index,
                }),
            }
        }
    }
}

fn decode_utf8_lossy_counted(bytes: &[u8]) -> (String, usize, Option<usize>) {
    let mut text = String::with_capacity(bytes.len());
    let mut invalid = 0;
    let mut first = None;
    let mut offset = 0;
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        offset += chunk.valid().len();
        if !chunk.invalid().is_empty() {
            invalid += 1;
            first.get_or_insert(offset);
            text.push(char::REPLACEMENT_CHARACTER);
            offset += chunk.invalid().len();
        }
    }
    (text, invalid, first)
}

fn decode_utf16(
    bytes: &[u8],
    bom: &[u8],
    unit: fn([u8; 2]) -> u16,
) -> (String, usize, Option<usize>) {
    let body = bytes.strip_prefix(bom).unwrap_or(bytes);
    let skipped = bytes.len() - body.len();
    let units: Vec<u16> = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| unit(*pair))
        .collect();
    let mut text = String::with_capacity(units.len());
    let mut invalid = 0;
    let mut first = None;
    let mut unit_index = 0;
    for result in char::decode_utf16(units.iter().copied()) {
        match result {
            Ok(c) => {
                unit_index += c.len_utf16();
                text.push(c);
            }
            Err(_) => {
                invalid += 1;
                first.get_or_insert(skipped + unit_index * 2);
                unit_index += 1;
                text.push(char::REPLACEMENT_CHARACTER);
            }
        }
    }
    if body.len() % 2 == 1 {
        invalid += 1;
        first.get_or_insert(bytes.len() - 1);
        text.push(char::REPLACEMENT_CHARACTER);
    }
    (text, invalid, first)
}

/// Recognise BOM-less UTF-16 by its NUL-byte pattern (ASCII-heavy text).
fn sniff_utf16(bytes: &[u8]) -> Option<TextEncoding> {
    if bytes.len() < 4 || bytes.len() % 2 == 1 {
        return None;
    }
    let pairs = bytes.len() / 2;
    let (mut even_zero, mut odd_zero) = (0usize, 0usize);
    for pair in bytes.as_chunks::<2>().0 {
        even_zero += usize::from(pair[0] == 0);
        odd_zero += usize::from(pair[1] == 0);
    }
    let strong = pairs * 3 / 10;
    let weak = pairs / 20;
    if odd_zero > strong && even_zero <= weak {
        Some(TextEncoding::Utf16Le)
    } else if even_zero > strong && odd_zero <= weak {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

/// Windows-1252 bytes 0x80..=0x9F. The five bytes undefined by the code page
/// map to the matching C1 control (as WHATWG does), so decoding is lossless
/// and re-encoding returns the original byte.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

fn cp1252_to_char(byte: u8) -> char {
    match byte {
        0x80..=0x9F => CP1252_HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

fn char_to_cp1252(c: char) -> Option<u8> {
    let code = c as u32;
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    CP1252_HIGH
        .iter()
        .position(|&mapped| mapped == c)
        .map(|index| 0x80 + index as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1252_round_trips_every_byte() {
        let bytes: Vec<u8> = (0..=255).collect();
        let decoded = decode_as(&bytes, TextEncoding::Windows1252);
        assert!(decoded.is_lossless());
        assert_eq!(
            encode(&decoded.text, TextEncoding::Windows1252, false).unwrap(),
            bytes
        );
    }

    #[test]
    fn sniffs_bomless_utf16() {
        let le: Vec<u8> = "hello world"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let be: Vec<u8> = "hello world"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(detect_and_decode(&le).encoding, TextEncoding::Utf16Le);
        assert_eq!(detect_and_decode(&be).encoding, TextEncoding::Utf16Be);
        assert_eq!(detect_and_decode(&be).text, "hello world");
    }
}
