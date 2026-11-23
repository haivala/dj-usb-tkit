//! MPEG-1 Layer III frame headers: the one reader for the seek index builder
//! and the MP3 encoder's output.

/// One MPEG-1 Layer III frame at `offset` in a file.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mp3Frame {
    pub(crate) offset: usize,
    /// Bytes, header and padding included: `144 * bitrate / sample rate`,
    /// plus the padding byte.
    pub(crate) length: usize,
    pub(crate) padding: usize,
    pub(crate) bitrate_kbps: u16,
    pub(crate) sample_rate: u32,
}

/// The MPEG-1 Layer III frame whose header starts at `offset`. `None` for
/// no frame sync, another MPEG version or layer, or a free-format or
/// reserved bitrate or sample rate.
pub(crate) fn parse_mpeg1_layer3_frame(bytes: &[u8], offset: usize) -> Option<Mp3Frame> {
    let header = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
    if header & 0xffe0_0000 != 0xffe0_0000
        || (header >> 19) & 0x3 != 0x3
        || (header >> 17) & 0x3 != 0x1
    {
        return None;
    }
    const BITRATES: [u16; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 32_000, 0];
    let bitrate_kbps = BITRATES[((header >> 12) & 0xf) as usize];
    let sample_rate = SAMPLE_RATES[((header >> 10) & 0x3) as usize];
    if bitrate_kbps == 0 || sample_rate == 0 {
        return None;
    }
    let padding = ((header >> 9) & 1) as usize;
    let length = 144_000 * usize::from(bitrate_kbps) / sample_rate as usize + padding;
    Some(Mp3Frame {
        offset,
        length,
        padding,
        bitrate_kbps,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_len(header: [u8; 4]) -> Option<usize> {
        parse_mpeg1_layer3_frame(&header, 0).map(|frame| frame.length)
    }

    #[test]
    fn frame_length_follows_the_header() {
        // 320 kbit/s, 44.1 kHz, no padding / padding; 48 kHz.
        assert_eq!(frame_len([0xFF, 0xFB, 0xE0, 0x44]), Some(1044));
        assert_eq!(frame_len([0xFF, 0xFB, 0xE2, 0x44]), Some(1045));
        assert_eq!(frame_len([0xFF, 0xFB, 0xE4, 0x44]), Some(960));
        // MPEG-2, or no sync.
        assert_eq!(frame_len([0xFF, 0xF3, 0xE0, 0x44]), None);
        assert_eq!(frame_len([0x00, 0xFB, 0xE0, 0x44]), None);
    }
}
