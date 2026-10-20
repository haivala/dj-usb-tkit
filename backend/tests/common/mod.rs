//! Helpers shared by the functional test crates.

use std::fs;
use std::path::Path;

/// Copies an audio fixture to `target`, adding the target's file name as an
/// extra metadata block so every copy is a different file. The library keeps
/// one track per file content, so tests that use one fixture for several
/// tracks need distinct bytes. Audio data and existing metadata are kept.
pub fn copy_unique_audio(source: &Path, target: &Path) {
    let bytes = fs::read(source).expect("read audio fixture");
    let marker = target
        .file_name()
        .expect("target file name")
        .to_string_lossy()
        .into_owned()
        .into_bytes();
    let unique = match &bytes[..4] {
        b"RIFF" => with_trailing_chunk(&bytes, *b"JUNK", &marker, u32::to_le_bytes),
        b"FORM" => with_trailing_chunk(&bytes, *b"ANNO", &marker, u32::to_be_bytes),
        b"fLaC" => with_flac_application_block(&bytes, &marker),
        [b'I', b'D', b'3', _] => with_id3v23_priv_frame(&bytes, &marker),
        _ => panic!("unsupported audio fixture: {}", source.display()),
    };
    fs::write(target, unique).expect("write audio fixture copy");
}

/// RIFF (little-endian sizes) and AIFF FORM (big-endian) share the layout:
/// append a chunk and grow the container size at offset 4.
fn with_trailing_chunk(
    bytes: &[u8],
    id: [u8; 4],
    data: &[u8],
    encode: fn(u32) -> [u8; 4],
) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out.extend_from_slice(&id);
    out.extend_from_slice(&encode(data.len() as u32));
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    let container = encode((out.len() - 8) as u32);
    out[4..8].copy_from_slice(&container);
    out
}

/// Inserts an APPLICATION block right after STREAMINFO (which must stay first).
fn with_flac_application_block(bytes: &[u8], data: &[u8]) -> Vec<u8> {
    let streaminfo_header = 4;
    let streaminfo_len = u32::from_be_bytes([0, bytes[5], bytes[6], bytes[7]]) as usize;
    let streaminfo_end = streaminfo_header + 4 + streaminfo_len;
    let streaminfo_was_last = bytes[4] & 0x80 != 0;

    let mut block = vec![if streaminfo_was_last { 0x82 } else { 0x02 }];
    let len = (4 + data.len()) as u32;
    block.extend_from_slice(&len.to_be_bytes()[1..]);
    block.extend_from_slice(b"test");
    block.extend_from_slice(data);

    let mut out = bytes[..streaminfo_end].to_vec();
    out[4] &= 0x7f;
    out.extend_from_slice(&block);
    out.extend_from_slice(&bytes[streaminfo_end..]);
    out
}

/// Inserts a PRIV frame at the start of an ID3v2.3 tag and grows the tag size.
fn with_id3v23_priv_frame(bytes: &[u8], data: &[u8]) -> Vec<u8> {
    assert_eq!(bytes[3], 3, "fixture tag must be ID3v2.3");
    assert_eq!(bytes[5], 0, "fixture tag must have no header flags");
    let syncsafe = |b: &[u8]| {
        b.iter()
            .fold(0u32, |size, &byte| (size << 7) | u32::from(byte & 0x7f))
    };
    let tag_size = syncsafe(&bytes[6..10]);

    let mut payload = b"dj-usb-tkit-test\0".to_vec();
    payload.extend_from_slice(data);
    let mut frame = b"PRIV".to_vec();
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&payload);

    let new_size = tag_size + frame.len() as u32;
    let mut out = bytes[..6].to_vec();
    out.extend((0..4).rev().map(|i| ((new_size >> (7 * i)) & 0x7f) as u8));
    out.extend_from_slice(&frame);
    out.extend_from_slice(&bytes[10..]);
    out
}
