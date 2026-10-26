use kdf_validation::{lint_path, LintOptions};
use std::fs;
use tempfile::tempdir;

fn wave(channels: u16, rate: u32, bits: u16, tag: u16) -> Vec<u8> {
    let align = channels * bits / 8;
    let data = vec![0; usize::from(align) * 16];
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(tag.to_le_bytes());
    bytes.extend(channels.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * u32::from(align)).to_le_bytes());
    bytes.extend(align.to_le_bytes());
    bytes.extend(bits.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

fn lint_sound(file: &str, data: Option<&[u8]>, extra: &str) -> kdf_validation::LintResult {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("pack.yml"),
        "ref: SoundExample\nname: Sound Example\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::create_dir(root.path().join("audio")).unwrap();
    if let Some(data) = data {
        let output = if file.to_ascii_lowercase().ends_with(".flac") {
            "audio/welcome.flac"
        } else {
            "audio/welcome.wav"
        };
        fs::write(root.path().join(output), data).unwrap();
    }
    let explicit_type = if extra.contains("---") {
        "type: sound\n"
    } else {
        ""
    };
    fs::write(
        root.path().join("welcome.sound.yml"),
        format!("{explicit_type}sounds:\n  - id: AdaWelcome\n    file: {file}\n{extra}"),
    )
    .unwrap();
    lint_path(root.path(), &LintOptions::default()).unwrap()
}

fn flac(bits: u64, frames: u64) -> Vec<u8> {
    let mut bytes = b"fLaC\x80\0\0\x22".to_vec();
    bytes.resize(42, 0);
    let stream = (44100u64 << 44) | ((bits - 1) << 36) | frames;
    bytes[18..26].copy_from_slice(&stream.to_be_bytes());
    bytes.extend([0xff, 0xf8, 0, 0]);
    bytes
}

#[test]
fn accepts_native_pcm16_flac_and_rejects_unsupported_streaminfo() {
    assert!(lint_sound("audio/welcome.flac", Some(&flac(16, 1024)), "").ok);
    for data in [
        flac(24, 1024),
        flac(16, 0),
        flac(16, MAX_DECODED_FRAMES + 1),
        b"OggSnotnativeflac".to_vec(),
        flac(16, 1024)[..42].to_vec(),
    ] {
        let result = lint_sound("audio/welcome.flac", Some(&data), "");
        assert!(!result.ok, "{:?}", result.diagnostics);
    }
}

const MAX_DECODED_FRAMES: u64 = 64 * 1024 * 1024 / 2;

#[test]
fn accepts_pcm16_mono_stereo_and_filename_type() {
    for (channels, rate) in [(1, 8000), (1, 48000), (2, 44100), (2, 192000)] {
        let result = lint_sound("audio/welcome.wav", Some(&wave(channels, rate, 16, 1)), "");
        assert!(result.ok, "{:?}", result.diagnostics);
    }
}

#[test]
fn accepts_optional_sound_display_metadata() {
    let result = lint_sound(
        "audio/welcome.wav",
        Some(&wave(1, 48000, 16, 1)),
        "    name: Welcome\n    description: ADA welcome line\n    category: Voice\n    subCategory: ADA\n",
    );
    assert!(result.ok, "{:?}", result.diagnostics);
}

#[test]
fn rejects_blank_sound_display_metadata() {
    let result = lint_sound(
        "audio/welcome.wav",
        Some(&wave(1, 48000, 16, 1)),
        "    name: ''\n",
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "schema.minLength"),
        "{:?}",
        result.diagnostics
    );

    let result = lint_sound(
        "audio/welcome.wav",
        Some(&wave(1, 48000, 16, 1)),
        "    category: '   '\n",
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "schema.pattern"),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn accepts_extensible_pcm_and_rejects_corrupt_chunks() {
    let mut extensible = wave(2, 48000, 16, 0xfffe);
    let mut extension = Vec::new();
    extension.extend(22u16.to_le_bytes());
    extension.extend(16u16.to_le_bytes());
    extension.extend(3u32.to_le_bytes());
    extension.extend([
        1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
    ]);
    extensible.splice(36..36, extension);
    extensible[16..20].copy_from_slice(&40u32.to_le_bytes());
    let size = extensible.len() as u32 - 8;
    extensible[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(lint_sound("audio/welcome.wav", Some(&extensible), "").ok);

    let mut bad_guid = extensible.clone();
    bad_guid[44] = 3;
    let mut bad_mask = extensible;
    bad_mask[40..44].copy_from_slice(&4u32.to_le_bytes());
    let mut truncated_chunk = wave(1, 48000, 16, 1);
    truncated_chunk[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut bad_riff_size = wave(1, 48000, 16, 1);
    bad_riff_size[4] = 0;
    let mut incomplete_frame = wave(2, 48000, 16, 1);
    incomplete_frame.truncate(incomplete_frame.len() - 2);
    let size = incomplete_frame.len() as u32;
    incomplete_frame[4..8].copy_from_slice(&(size - 8).to_le_bytes());
    incomplete_frame[40..44].copy_from_slice(&(size - 44).to_le_bytes());
    for bytes in [
        bad_guid,
        bad_mask,
        truncated_chunk,
        bad_riff_size,
        incomplete_frame,
    ] {
        let result = lint_sound("audio/welcome.wav", Some(&bytes), "");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "semantic.sound-format"),
            "{:?}",
            result.diagnostics
        );
    }
}

#[test]
fn rejects_invalid_formats_missing_files_paths_and_unknown_fields() {
    for data in [
        wave(1, 48000, 32, 3),
        wave(3, 48000, 16, 1),
        wave(1, 48000, 24, 1),
        wave(1, 7999, 16, 1),
        b"not a wave".to_vec(),
    ] {
        let result = lint_sound("audio/welcome.wav", Some(&data), "");
        assert!(!result.ok);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code == "semantic.sound-format"),
            "{:?}",
            result.diagnostics
        );
    }
    for (file, code) in [
        ("audio/missing.wav", "semantic.sound-missing"),
        ("../outside.wav", "semantic.sound-escape"),
    ] {
        let result = lint_sound(file, None, "");
        assert!(
            result.diagnostics.iter().any(|d| d.code == code),
            "{:?}",
            result.diagnostics
        );
    }
    assert!(!lint_sound("audio/welcome.mp3", None, "").ok);
    assert!(
        !lint_sound(
            "audio/welcome.wav",
            Some(&wave(1, 48000, 16, 1)),
            "    loop: true\n"
        )
        .ok
    );
}

#[test]
fn detects_asset_namespace_collisions() {
    let result = lint_sound("audio/welcome.wav", Some(&wave(1, 48000, 16, 1)), "---\ntype: curve\ncurveType: float\ncurves:\n  - id: AdaWelcome\n    keys:\n      - time: 0\n        value: 1\n");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "semantic.duplicate-id"),
        "{:?}",
        result.diagnostics
    );
}
