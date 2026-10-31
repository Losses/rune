use std::sync::OnceLock;

fn params() -> crc_fast::CrcParams {
    static PARAMS: OnceLock<crc_fast::CrcParams> = OnceLock::new();
    *PARAMS.get_or_init(|| {
        // Historical Rune CRC: NOT standard Ogg CRC or CRC32C.
        crc_fast::CrcParams::new("RUNE", 32, 0x1bf52, 0, false, 0, 0xce7402ce)
    })
}

/// Streaming CRC with fixed parameters. Keep the evolving state in the digest,
/// never in a new parameter set (the software backend caches tables by params).
pub struct MediaCrc(crc_fast::Digest);

impl Default for MediaCrc {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaCrc {
    pub fn new() -> Self {
        Self(crc_fast::Digest::new_with_params(params()))
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finalize(&self) -> u32 {
        self.0.finalize() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(bytes: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &byte in bytes {
            crc ^= (byte as u32) << 24;
            for _ in 0..8 {
                crc = (crc << 1) ^ ((crc >> 31) * 0x1bf52);
            }
        }
        crc
    }

    #[test]
    fn legacy_crc_matches_across_offsets_and_stream_boundaries() {
        let mut state = 0x12345678u32;
        let bytes: Vec<_> = (0..8192)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        for offset in 0..16 {
            for len in (0..80).chain([127, 128, 129, 255, 256, 257, 1023, 1024, 4096]) {
                let data = &bytes[offset..offset + len];
                let expected = reference(data);
                for chunk in [1, 7, 16, 63, 256, 409600] {
                    let mut crc = MediaCrc::new();
                    for block in data.chunks(chunk) {
                        crc.update(block);
                    }
                    crc.update(&[]);
                    assert_eq!(
                        crc.finalize(),
                        expected,
                        "offset={offset} len={len} chunk={chunk}"
                    );
                }
            }
        }
        let mut crc = MediaCrc::new();
        crc.update(b"123456789");
        assert_eq!(crc.finalize(), 0xce7402ce);
    }
}
