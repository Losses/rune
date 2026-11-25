// Preserve Rune's historical polynomial, byte order and initial/final state.
// This is not the standard Ogg CRC-32 (nor the CRC-32C instruction on x86).
const POLYNOMIAL: u32 = 0x1bf52;

/// T[n][byte] is the contribution of a byte followed by n zero bytes.
/// Eight tables break the per-byte dependency chain without changing the CRC.
const fn generate_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0; 256]; 8];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = (byte as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc << 1) ^ ((crc >> 31) * POLYNOMIAL);
            bit += 1;
        }
        tables[0][byte] = crc;
        byte += 1;
    }
    let mut slice = 1;
    while slice < 8 {
        byte = 0;
        while byte < 256 {
            let crc = tables[slice - 1][byte];
            tables[slice][byte] = (crc << 8) ^ tables[0][(crc >> 24) as usize];
            byte += 1;
        }
        slice += 1;
    }
    tables
}

const TABLES: [[u32; 256]; 8] = generate_tables();

/// Computes Rune's legacy media checksum over `array[from..to]`.
///
/// Pass a previous result as `initial` to continue hashing another chunk.
/// There is no initial or final inversion. The result is also used for cover
/// art and independent-file identities, so it must remain bit-for-bit stable.
/// Like the original implementation, an empty or reversed range returns
/// `initial`; a nonempty range outside the array panics.
pub fn media_crc32(array: &[u8], initial: u32, from: usize, to: usize) -> u32 {
    if from >= to {
        return initial;
    }
    let mut result = initial;
    let (blocks, rest) = array[from..to].as_chunks::<8>();
    for block in blocks {
        let head = result ^ u32::from_be_bytes([block[0], block[1], block[2], block[3]]);
        result = TABLES[7][(head >> 24) as usize]
            ^ TABLES[6][((head >> 16) & 0xff) as usize]
            ^ TABLES[5][((head >> 8) & 0xff) as usize]
            ^ TABLES[4][(head & 0xff) as usize]
            ^ TABLES[3][block[4] as usize]
            ^ TABLES[2][block[5] as usize]
            ^ TABLES[1][block[6] as usize]
            ^ TABLES[0][block[7] as usize];
    }
    for &byte in rest {
        result = (result << 8) ^ TABLES[0][(byte as u32 ^ (result >> 24)) as usize];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::media_crc32;

    // Independent bit-at-a-time reference; do not reuse the optimized tables.
    fn reference(bytes: &[u8], mut crc: u32) -> u32 {
        for &byte in bytes {
            crc ^= u32::from(byte) << 24;
            for _ in 0..8 {
                crc = (crc << 1) ^ if crc & 0x8000_0000 != 0 { 0x1bf52 } else { 0 };
            }
        }
        crc
    }

    fn data(length: usize) -> Vec<u8> {
        let mut state = 0x1234_5678u32;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect()
    }

    #[test]
    fn legacy_golden_vectors() {
        assert_eq!(media_crc32(b"123456789", 0, 0, 9), 0xce7402ce);
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(media_crc32(&bytes, 0, 0, bytes.len()), 0x51561ddc);
        assert_eq!(media_crc32(&[], 0xdeadbeef, 0, 0), 0xdeadbeef);
        assert_eq!(media_crc32(&[], 123, 99, 1), 123);
    }

    #[test]
    fn matches_reference_for_offsets_tails_and_initial_states() {
        let bytes = data(1040);
        for offset in 0..16 {
            for length in 0..=1024 {
                for initial in [0, 1, u32::MAX, 0x1234_5678] {
                    assert_eq!(
                        media_crc32(&bytes, initial, offset, offset + length),
                        reference(&bytes[offset..offset + length], initial),
                        "offset={offset} length={length} initial={initial}"
                    );
                }
            }
        }
    }

    #[test]
    fn incremental_hash_matches_whole_file_across_read_boundaries() {
        // Includes the actual 400 KiB file-read boundary and all block tails.
        let bytes = data(400 * 1024 + 37);
        let expected = reference(&bytes, 0x1357_9bdf);
        assert_eq!(media_crc32(&bytes, 0x1357_9bdf, 0, bytes.len()), expected);
        for chunk_size in [1, 3, 7, 8, 9, 31, 512, 4096, 400 * 1024] {
            let actual = bytes.chunks(chunk_size).fold(0x1357_9bdf, |crc, chunk| {
                media_crc32(chunk, crc, 0, chunk.len())
            });
            assert_eq!(actual, expected, "chunk_size={chunk_size}");
        }
    }

    #[test]
    #[should_panic]
    fn nonempty_out_of_bounds_range_still_panics() {
        media_crc32(&[1, 2, 3], 0, 0, 4);
    }
}
