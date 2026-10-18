//! Encoding detection, damaged input, and byte-exact round trips through
//! load -> save for every supported encoding and line ending.

mod common;

use std::fs;

use common::TempDir;
use notestrike::document::{self, LoadOptions, SaveError, SaveRequest};
use notestrike::encoding::{self, TextEncoding};
use notestrike::eol::LineEnding;

fn utf16(text: &str, le: bool, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
    }
    for unit in text.encode_utf16() {
        out.extend_from_slice(&if le {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    out
}

/// Load `bytes` from disk, save unchanged text back, and require identical bytes.
fn assert_round_trip(bytes: &[u8], expected_encoding: TextEncoding, expected_eol: LineEnding) {
    let dir = TempDir::new("roundtrip");
    let path = dir.join("file.txt");
    fs::write(&path, bytes).unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    assert_eq!(loaded.encoding, expected_encoding);
    assert_eq!(loaded.eol, expected_eol);
    assert_eq!(loaded.invalid_sequences, 0);
    assert!(
        !loaded.text.contains('\r'),
        "editor text is normalised to \\n"
    );
    document::save(&SaveRequest {
        path: &path,
        text: &loaded.text,
        encoding: loaded.encoding,
        utf16_bom: loaded.utf16_bom,
        eol: loaded.eol,
        expected: Some(&loaded.snapshot),
        overwrite_external_changes: false,
    })
    .unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "save must reproduce the original bytes"
    );
}

#[test]
fn round_trips_every_encoding_and_line_ending_byte_for_byte() {
    let text_lf = "Sample: Grüße, café, naïve — €5\nline two\nline three\n";
    for eol in LineEnding::ALL {
        let text = text_lf.replace('\n', eol.as_str());
        assert_round_trip(text.as_bytes(), TextEncoding::Utf8, eol);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(text.as_bytes());
        assert_round_trip(&bom, TextEncoding::Utf8Bom, eol);
        assert_round_trip(&utf16(&text, true, true), TextEncoding::Utf16Le, eol);
        assert_round_trip(&utf16(&text, false, true), TextEncoding::Utf16Be, eol);
        let cp1252 = encoding::encode(&text, TextEncoding::Windows1252, false).unwrap();
        assert_round_trip(&cp1252, TextEncoding::Windows1252, eol);
    }
    // BOM-less UTF-16 stays BOM-less.
    let plain = "plain ascii text\r\nsecond line\r\n";
    assert_round_trip(
        &utf16(plain, true, false),
        TextEncoding::Utf16Le,
        LineEnding::CrLf,
    );
    assert_round_trip(
        &utf16(plain, false, false),
        TextEncoding::Utf16Be,
        LineEnding::CrLf,
    );
}

#[test]
fn damaged_utf8_is_reported_not_silently_converted() {
    // Valid UTF-8 (é) followed by stray bytes 0xFF and a truncated sequence.
    let bytes = b"caf\xC3\xA9 ok\n\xFF bad\n\xE2\x82".to_vec();
    let decoded = encoding::detect_and_decode(&bytes);
    assert_eq!(decoded.encoding, TextEncoding::Utf8);
    assert_eq!(decoded.invalid_sequences, 2);
    assert_eq!(decoded.first_invalid_offset, Some(9));
    assert!(decoded.text.starts_with("café ok\n\u{FFFD} bad\n"));
    // The user can reopen it as Windows-1252 and keep every byte.
    let legacy = encoding::decode_as(&bytes, TextEncoding::Windows1252);
    assert!(legacy.is_lossless());
    assert_eq!(
        encoding::encode(&legacy.text, TextEncoding::Windows1252, false).unwrap(),
        bytes
    );
}

#[test]
fn legacy_single_byte_text_falls_back_to_windows_1252() {
    let bytes = b"Price: 5\x80 \x93quoted\x94 caf\xE9".to_vec();
    let decoded = encoding::detect_and_decode(&bytes);
    assert_eq!(decoded.encoding, TextEncoding::Windows1252);
    assert!(decoded.is_lossless());
    assert_eq!(decoded.text, "Price: 5€ \u{201C}quoted\u{201D} café");
}

#[test]
fn damaged_utf16_is_reported() {
    // Unpaired high surrogate, then an odd trailing byte.
    let mut bytes = vec![0xFF, 0xFE, b'A', 0, 0x00, 0xD8, b'B', 0];
    bytes.push(0x41);
    let decoded = encoding::detect_and_decode(&bytes);
    assert_eq!(decoded.encoding, TextEncoding::Utf16Le);
    assert_eq!(decoded.invalid_sequences, 2);
    assert_eq!(decoded.first_invalid_offset, Some(4));
    assert_eq!(decoded.text, "A\u{FFFD}B\u{FFFD}");
}

#[test]
fn binary_looking_file_is_flagged() {
    let dir = TempDir::new("nul");
    let path = dir.join("blob.bin");
    fs::write(&path, b"MZ\x00\x01\x02 header\x00\x00 text").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    assert!(loaded.contains_nul);
}

#[test]
fn mixed_line_endings_are_detected_and_unified_to_the_dominant_one() {
    let dir = TempDir::new("mixed");
    let path = dir.join("mixed.txt");
    fs::write(&path, b"a\r\nb\r\nc\nd").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    assert!(loaded.eol_counts.is_mixed());
    assert_eq!(loaded.eol, LineEnding::CrLf);
    assert_eq!(loaded.text, "a\nb\nc\nd");
}

#[test]
fn saving_unrepresentable_characters_as_windows_1252_writes_nothing() {
    let dir = TempDir::new("unrepresentable");
    let path = dir.join("legacy.txt");
    fs::write(&path, b"caf\xE9").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    assert_eq!(loaded.encoding, TextEncoding::Windows1252);
    let err = document::save(&SaveRequest {
        path: &path,
        text: "café 日本",
        encoding: loaded.encoding,
        utf16_bom: false,
        eol: loaded.eol,
        expected: Some(&loaded.snapshot),
        overwrite_external_changes: false,
    })
    .unwrap_err();
    match err {
        SaveError::Unrepresentable(details) => {
            assert_eq!(details.count, 2);
            assert_eq!(details.first, '日');
            assert_eq!(details.first_char_index, 5);
        }
        other => panic!("expected Unrepresentable, got {other:?}"),
    }
    assert_eq!(fs::read(&path).unwrap(), b"caf\xE9", "original untouched");
}

#[test]
fn forced_encoding_reopen() {
    let dir = TempDir::new("forced");
    let path = dir.join("f.txt");
    fs::write(&path, "Grüße".as_bytes()).unwrap();
    let as_1252 = document::load(
        &path,
        LoadOptions {
            allow_large: false,
            forced_encoding: Some(TextEncoding::Windows1252),
        },
    )
    .unwrap();
    assert_eq!(as_1252.encoding, TextEncoding::Windows1252);
    assert_eq!(as_1252.text, "Gr\u{C3}\u{BC}\u{C3}\u{178}e");
}
