//! Strict shared text codec. Legacy detection is restricted to INI/CFG files.

use std::{fs, io::Read, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Utf16LeNoBom,
    Utf16BeNoBom,
    Utf32Le,
    Utf32Be,
    Windows1252,
}

const CP1252: [char; 32] = [
    '€', '\u{0081}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{008d}', 'Ž',
    '\u{008f}', '\u{0090}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{009d}',
    'ž', 'Ÿ',
];

fn decode_utf32(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err("UTF-32 file has an incomplete code unit".into());
    }
    let mut text = String::new();
    for unit in bytes.as_chunks::<4>().0 {
        let bytes = [unit[0], unit[1], unit[2], unit[3]];
        let value = if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        };
        text.push(char::from_u32(value).ok_or("File contains invalid UTF-32 text")?);
    }
    without_nul(text)
}

fn legacy_config(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".ini") || lower.ends_with(".cfg")
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("UTF-16 file has an incomplete code unit".into());
    }
    let units = bytes.as_chunks::<2>().0.iter().map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    String::from_utf16(&units.collect::<Vec<_>>())
        .map_err(|_| "File contains invalid UTF-16 text".into())
}

fn without_nul(content: String) -> Result<String, String> {
    if content.contains('\0') {
        return Err("File contains binary NUL bytes".into());
    }
    Ok(content)
}

fn utf16_without_bom(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let pairs = bytes.len() / 2;
    let even_zero = bytes.iter().step_by(2).filter(|byte| **byte == 0).count();
    let odd_zero = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    if odd_zero * 4 >= pairs * 3 && even_zero * 4 < pairs {
        Some(true)
    } else if even_zero * 4 >= pairs * 3 && odd_zero * 4 < pairs {
        Some(false)
    } else {
        None
    }
}

pub fn decode_text(bytes: &[u8], path: &str) -> Result<(String, TextEncoding), String> {
    if let Some(content) = bytes.strip_prefix(&[0xff, 0xfe, 0, 0]) {
        return Ok((decode_utf32(content, true)?, TextEncoding::Utf32Le));
    }
    if let Some(content) = bytes.strip_prefix(&[0, 0, 0xfe, 0xff]) {
        return Ok((decode_utf32(content, false)?, TextEncoding::Utf32Be));
    }
    if let Some(content) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        let text = String::from_utf8(content.to_vec()).map_err(|_| "File is not UTF-8 text")?;
        return Ok((without_nul(text)?, TextEncoding::Utf8Bom));
    }
    if let Some(content) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return Ok((
            without_nul(decode_utf16(content, true)?)?,
            TextEncoding::Utf16Le,
        ));
    }
    if let Some(content) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return Ok((
            without_nul(decode_utf16(content, false)?)?,
            TextEncoding::Utf16Be,
        ));
    }
    if legacy_config(path) {
        if let Some(little_endian) = utf16_without_bom(bytes) {
            return Ok((
                without_nul(decode_utf16(bytes, little_endian)?)?,
                if little_endian {
                    TextEncoding::Utf16LeNoBom
                } else {
                    TextEncoding::Utf16BeNoBom
                },
            ));
        }
    }
    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
        return Ok((without_nul(text)?, TextEncoding::Utf8));
    }
    if legacy_config(path) && !bytes.contains(&0) {
        let text = bytes
            .iter()
            .map(|&byte| {
                if (0x80..=0x9f).contains(&byte) {
                    CP1252[usize::from(byte - 0x80)]
                } else {
                    char::from(byte)
                }
            })
            .collect();
        return Ok((without_nul(text)?, TextEncoding::Windows1252));
    }
    Err("File is not supported text (UTF-8, UTF-16, UTF-32 or Windows-1252 config)".into())
}

pub fn encode_text(content: &str, encoding: TextEncoding) -> Result<Vec<u8>, String> {
    if content.contains('\0') {
        return Err("Text cannot contain binary NUL bytes".into());
    }
    let mut bytes = Vec::new();
    match encoding {
        TextEncoding::Utf8 => bytes.extend_from_slice(content.as_bytes()),
        TextEncoding::Utf8Bom => {
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            bytes.extend_from_slice(content.as_bytes());
        }
        TextEncoding::Utf16Le
        | TextEncoding::Utf16Be
        | TextEncoding::Utf16LeNoBom
        | TextEncoding::Utf16BeNoBom => {
            if encoding == TextEncoding::Utf16Le {
                bytes.extend_from_slice(&[0xff, 0xfe]);
            } else if encoding == TextEncoding::Utf16Be {
                bytes.extend_from_slice(&[0xfe, 0xff]);
            }
            for unit in content.encode_utf16() {
                bytes.extend_from_slice(
                    if matches!(encoding, TextEncoding::Utf16Le | TextEncoding::Utf16LeNoBom) {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    }
                    .as_slice(),
                );
            }
        }
        TextEncoding::Utf32Le | TextEncoding::Utf32Be => {
            bytes.extend_from_slice(if encoding == TextEncoding::Utf32Le {
                &[0xff, 0xfe, 0, 0]
            } else {
                &[0, 0, 0xfe, 0xff]
            });
            for character in content.chars() {
                let encoded = if encoding == TextEncoding::Utf32Le {
                    u32::from(character).to_le_bytes()
                } else {
                    u32::from(character).to_be_bytes()
                };
                bytes.extend_from_slice(&encoded);
            }
        }
        TextEncoding::Windows1252 => {
            for character in content.chars() {
                let value = u32::from(character);
                if value <= 0x7f || (0xa0..=0xff).contains(&value) {
                    bytes.push(u8::try_from(value).map_err(|e| e.to_string())?);
                } else if let Some(index) = CP1252.iter().position(|&mapped| mapped == character) {
                    bytes.push(u8::try_from(index).map_err(|e| e.to_string())? + 0x80);
                } else {
                    return Err("Text contains characters not representable in Windows-1252".into());
                }
            }
        }
    }
    Ok(bytes)
}

pub fn read_text_file(path: &Path, max_bytes: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err("Text file exceeds size limit".into());
    }
    decode_text(&bytes, &path.to_string_lossy()).map(|(text, _)| text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_config_encodings_without_changing_bom_or_code_page() {
        for encoding in [
            TextEncoding::Utf8,
            TextEncoding::Utf8Bom,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
            TextEncoding::Utf16LeNoBom,
            TextEncoding::Utf16BeNoBom,
            TextEncoding::Utf32Le,
            TextEncoding::Utf32Be,
            TextEncoding::Windows1252,
        ] {
            let bytes = encode_text("[Section]\r\nName=Größe\r\n", encoding).unwrap();
            let (decoded, detected) = decode_text(&bytes, "Config/DefaultGame.ini").unwrap();
            assert_eq!(decoded, "[Section]\r\nName=Größe\r\n");
            assert_eq!(detected, encoding);
            assert_eq!(encode_text(&decoded, detected).unwrap(), bytes);
        }
    }

    #[test]
    fn rejects_binary_and_unrepresentable_legacy_text() {
        assert!(decode_text(&[0, 1, 2, 3], "Config/Test.cfg").is_err());
        assert!(decode_text(&[0xff], "DataForge/pack.yml").is_err());
        assert!(encode_text("emoji 😀", TextEncoding::Windows1252).is_err());
        assert!(encode_text("binary\0text", TextEncoding::Utf8).is_err());
    }

    #[test]
    fn known_bom_vectors_and_malformed_unicode() {
        for (bytes, encoding) in [
            (vec![0xef, 0xbb, 0xbf, b'A'], TextEncoding::Utf8Bom),
            (vec![0xff, 0xfe, b'A', 0], TextEncoding::Utf16Le),
            (vec![0xfe, 0xff, 0, b'A'], TextEncoding::Utf16Be),
            (vec![0xff, 0xfe, 0, 0, b'A', 0, 0, 0], TextEncoding::Utf32Le),
            (vec![0, 0, 0xfe, 0xff, 0, 0, 0, b'A'], TextEncoding::Utf32Be),
        ] {
            assert_eq!(
                decode_text(&bytes, "pack.yml").unwrap(),
                ("A".into(), encoding)
            );
            assert_eq!(encode_text("A", encoding).unwrap(), bytes);
        }
        for bytes in [
            vec![0xff, 0xfe, b'A'],
            vec![0xff, 0xfe, 0, 0xd8],
            vec![0xfe, 0xff, 0xdc, 0],
            vec![0xff, 0xfe, 0, 0, 0x41],
            vec![0xff, 0xfe, 0, 0, 0, 0xd8, 0, 0],
            vec![0, 0, 0xfe, 0xff, 0, 0x11, 0, 0],
            vec![0xef, 0xbb, 0xbf, 0xff],
            vec![0xff, 0xfe, 0, 0, 0, 0, 0, 0],
        ] {
            assert!(decode_text(&bytes, "pack.yml").is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn legacy_detection_is_config_only_and_cp1252_is_lossless() {
        let bytes = b"[Section]\r\nName=\x80 \x93Gr\xf6\xdfe\x94\r\n";
        let (text, encoding) = decode_text(bytes, "SETTINGS.CFG").unwrap();
        assert_eq!(text, "[Section]\r\nName=€ “Größe”\r\n");
        assert_eq!(encoding, TextEncoding::Windows1252);
        assert_eq!(encode_text(&text, encoding).unwrap(), bytes);
        assert!(decode_text(bytes, "pack.yml").is_err());
        for encoding in [TextEncoding::Utf16LeNoBom, TextEncoding::Utf16BeNoBom] {
            let bytes = encode_text("[Section]\nName=Test\n", encoding).unwrap();
            assert_eq!(decode_text(&bytes, "Game.ini").unwrap().1, encoding);
            assert!(decode_text(&bytes, "pack.yml").is_err());
        }
        assert!(encode_text("emoji 😀", encoding).is_err());
    }

    #[test]
    fn validation_loads_bom_encoded_yaml_through_shared_codec() {
        for encoding in [
            TextEncoding::Utf8Bom,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
            TextEncoding::Utf32Le,
            TextEncoding::Utf32Be,
        ] {
            let temp = tempfile::tempdir().unwrap();
            fs::write(
                temp.path().join("pack.yml"),
                "ref: Encoding\nname: Encoding\nversion: 1.0.0\n",
            )
            .unwrap();
            let path = temp.path().join("test.cdo.yml");
            let text = "type: cdo\npatches:\n  - target: /Game/A.A_C\n    properties:\n      - path: mValue\n        op: multiplly\n        value: 1\n";
            fs::write(&path, encode_text(text, encoding).unwrap()).unwrap();
            let result = crate::lint_path(temp.path(), &crate::LintOptions::default()).unwrap();
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "semantic.invalid-op"),
                "{encoding:?}: {:?}",
                result.diagnostics
            );
            assert!(
                !result.diagnostics.iter().any(|d| d.code == "root.read"),
                "{encoding:?}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn bounded_reader_rejects_oversize_and_nonunicode_yaml() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("pack.yml");
        fs::write(&path, b"abcd").unwrap();
        assert_eq!(read_text_file(&path, 4).unwrap(), "abcd");
        assert!(read_text_file(&path, 3).is_err());
        fs::write(&path, [0x93, b'x', 0x94]).unwrap();
        assert!(read_text_file(&path, 3).is_err());
    }

    #[test]
    fn non_bmp_unicode_survives_utf16_and_utf32_without_replacement() {
        for encoding in [
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
            TextEncoding::Utf32Le,
            TextEncoding::Utf32Be,
        ] {
            let bytes = encode_text("unicode 😀", encoding).unwrap();
            assert_eq!(
                decode_text(&bytes, "pack.yml").unwrap(),
                ("unicode 😀".into(), encoding)
            );
        }
    }
}
