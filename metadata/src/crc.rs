/// Generates a CRC lookup table element for a given index.
///
/// # Arguments
///
/// * `idx` - The index for which to generate the CRC lookup table element.
///
/// # Returns
///
/// A 32-bit unsigned integer representing the CRC lookup table element.
const fn get_element_from_table(idx: u32) -> u32 {
    let mut r: u32 = idx << 24;
    let mut i = 0;
    while i < 8 {
        r = (r << 1) ^ ((r >> 31) * 0x1bf52);
        i += 1;
    }
    r
}

/// Generates a CRC lookup table.
///
/// # Returns
///
/// An array of 256 32-bit unsigned integers representing the CRC lookup table.
const fn generate_table() -> [u32; 256] {
    let mut lup_arr = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        lup_arr[i] = get_element_from_table(i as u32);
        i += 1;
    }
    lup_arr
}

/// A constant array representing the precomputed CRC lookup table.
const CRC_LOOKUP_ARRAY: [u32; 256] = generate_table();

/// Computes the CRC-32 checksum for a slice of bytes using the Vorbis CRC-32 algorithm.
///
/// # Arguments
///
/// * `array` - A slice of bytes for which to compute the checksum.
/// * `initial` - The initial CRC value.
/// * `from` - The starting index of the slice to compute the checksum.
/// * `to` - The ending index of the slice to compute the checksum (exclusive).
///
/// # Returns
///
/// A 32-bit unsigned integer representing the CRC-32 checksum.
pub fn media_crc32(array: &[u8], initial: u32, from: usize, to: usize) -> u32 {
    let mut result: u32 = initial;
    for i in from..to {
        result =
            (result << 8) ^ CRC_LOOKUP_ARRAY[((array[i] as u32) ^ (result >> 24)) as usize & 0xff];
    }
    result
}
