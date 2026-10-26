//! Same bounded PCM16 WAV/FLAC contract as KDFSoundLoader.cpp in the runtime mod.
use std::{fs::File, io::Read, path::Path};

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn validate_file(path: &Path) -> Result<(), String> {
    let flac = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("flac"));
    let file = File::open(path).map_err(|error| format!("could not read audio: {error}"))?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    if !(if flac { 42 } else { 44 }..=MAX_FILE_BYTES).contains(&size) {
        return Err("audio size must be at least its header size and at most 64 MiB".into());
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("audio exceeds 64 MiB".into());
    }
    if flac {
        validate_flac(&bytes)
    } else {
        validate_wave(&bytes)
    }
}

fn validate_flac(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 42
        || &bytes[..4] != b"fLaC"
        || bytes[4] & 127 != 0
        || bytes[5..8] != [0, 0, 34]
    {
        return Err("sound must be native FLAC with a valid STREAMINFO block, not Ogg FLAC".into());
    }
    let stream = u64::from_be_bytes(bytes[18..26].try_into().unwrap());
    let rate = stream >> 44;
    let channels = ((stream >> 41) & 7) + 1;
    let bits = ((stream >> 36) & 31) + 1;
    let frames = stream & ((1 << 36) - 1);
    if bits != 16
        || ![1, 2].contains(&channels)
        || !(8000..=192000).contains(&rate)
        || frames == 0
        || frames > MAX_FILE_BYTES / (channels * 2)
    {
        return Err("FLAC must be PCM16 mono/stereo, 8000-192000 Hz, with a known nonzero frame count and at most 64 MiB decoded PCM".into());
    }
    let mut offset = 4;
    loop {
        if bytes.len() - offset < 4 {
            return Err("truncated FLAC metadata header".into());
        }
        let last = bytes[offset] & 128 != 0;
        let size = (usize::from(bytes[offset + 1]) << 16)
            | (usize::from(bytes[offset + 2]) << 8)
            | usize::from(bytes[offset + 3]);
        offset = offset
            .checked_add(4 + size)
            .ok_or("FLAC metadata overflow")?;
        if offset > bytes.len() {
            return Err("truncated FLAC metadata block".into());
        }
        if last {
            break;
        }
    }
    if bytes.len() - offset < 2 || bytes[offset] != 0xff || bytes[offset + 1] & 0xfe != 0xf8 {
        return Err("FLAC needs an audio frame after its metadata".into());
    }
    // Frame decoding and PCM checksum verification happen in the game importer.
    Ok(())
}

fn validate_wave(bytes: &[u8]) -> Result<(), String> {
    let read16 = |offset| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
    let read32 = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("sound must be a little-endian RIFF/WAVE file".into());
    }
    if u64::from(read32(4)) + 8 != bytes.len() as u64 {
        return Err("WAV RIFF size does not match file size".into());
    }
    let mut offset = 12;
    let mut channels = None;
    let mut data_size = None;
    while offset < bytes.len() {
        if bytes.len() - offset < 8 {
            return Err("truncated WAV chunk header".into());
        }
        let size = read32(offset + 4) as usize;
        let body = offset + 8;
        let next = body
            .checked_add(size)
            .and_then(|end| end.checked_add(size & 1))
            .ok_or("WAV chunk overflow")?;
        if next > bytes.len() {
            return Err("truncated WAV chunk data".into());
        }
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                if channels.is_some() || size < 16 {
                    return Err("invalid or duplicate WAV fmt chunk".into());
                }
                let count = read16(body + 2);
                let rate = read32(body + 4);
                let tag = read16(body);
                if tag == 0xfffe {
                    const PCM_GUID: [u8; 16] = [
                        1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
                    ];
                    if size < 40
                        || read16(body + 16) != 22
                        || read16(body + 18) != 16
                        || bytes[body + 24..body + 40] != PCM_GUID
                        || ![0, if count == 1 { 4 } else { 3 }].contains(&read32(body + 20))
                    {
                        return Err("WAV extensible format must be PCM16 mono or stereo".into());
                    }
                } else if tag != 1 {
                    return Err(
                        "WAV must contain uncompressed PCM16, not float or compressed audio".into(),
                    );
                }
                if ![1, 2].contains(&count)
                    || !(8000..=192000).contains(&rate)
                    || read16(body + 14) != 16
                    || read16(body + 12) != count * 2
                    || read32(body + 8) != rate * u32::from(count) * 2
                {
                    return Err("WAV must be PCM16 mono/stereo, 8000-192000 Hz, with consistent rate and frame size".into());
                }
                channels = Some(count);
            }
            b"data" => {
                if data_size.is_some() || size == 0 {
                    return Err("empty or duplicate WAV data chunk".into());
                }
                data_size = Some(size);
            }
            _ => {}
        }
        offset = next;
    }
    match (channels, data_size) {
        (Some(channels), Some(size)) if size % (usize::from(channels) * 2) == 0 => Ok(()),
        _ => Err("WAV needs fmt and data chunks containing complete PCM frames".into()),
    }
}
