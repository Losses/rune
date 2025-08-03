use std::error::Error;
use std::fmt;
use std::net::Ipv4Addr;
use std::str::from_utf8;

use log::error;

/// Represents errors that can occur during IPv4 address encoding.
#[derive(Debug)]
pub enum EncodeError {
    /// Indicates that the provided string is not a valid IPv4 address.
    InvalidIPv4,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Invalid IPv4 address")
    }
}

impl Error for EncodeError {}

/// Encodes an IPv4 address string into a 7-character base-36 encoded string.
///
/// This function takes an IPv4 address in standard dotted decimal notation (e.g., "192.168.1.1")
/// and converts it into a unique 7-character string using base-36 encoding.
///
/// # Arguments
///
/// * `ip` - A string slice representing the IPv4 address in dotted decimal notation.
///
/// # Returns
///
/// * `Result<String, EncodeError>` - On success, returns `Ok` containing the 7-character base-36 encoded string.
///   On failure, returns `Err` containing an `EncodeError`.
///
/// # Errors
///
/// * `EncodeError::InvalidIPv4`: If the input string `ip` is not a valid IPv4 address.
pub fn encode_ipv4(ip: &str) -> Result<String, EncodeError> {
    // Parse the input string into an Ipv4Addr.
    // If parsing fails (e.g., invalid format), return an `InvalidIPv4` error.
    let ipv4: Ipv4Addr = ip.parse().map_err(|_| EncodeError::InvalidIPv4)?;
    // Convert the Ipv4Addr to its 32-bit integer representation (network byte order).
    let num = ipv4.to_bits();
    // Initialize a vector to store the characters of the encoded string.
    // We know the encoded string will be 7 characters long, so pre-allocate capacity for efficiency.
    let mut chars = Vec::with_capacity(7);
    // `n` will be used to perform base-36 conversion, starting with the 32-bit IP number.
    let mut n = num;

    // Perform base-36 encoding 7 times to generate 7 characters.
    for _ in 0..7 {
        // Calculate the remainder when divided by 36. This gives us the next base-36 digit.
        let rem = (n % 36) as u8;
        // Convert the remainder to its character representation.
        // 0-9 are represented as '0'-'9', and 10-35 are represented as 'a'-'z'.
        chars.push(match rem {
            0..=9 => b'0' + rem,  // For remainders 0-9, use digits '0' through '9'.
            _ => b'a' + rem - 10, // For remainders 10-35, use lowercase letters 'a' through 'z'.
        });
        // Integer division by 36 to move to the next digit in base-36.
        n /= 36;
    }

    // The characters are generated in reverse order, so reverse the vector to get the correct order.
    chars.reverse();
    // Convert the vector of bytes to a String. This is safe because we only push ASCII characters.
    Ok(String::from_utf8(chars).unwrap())
}

/// Represents errors that can occur during base-36 encoded IPv4 address decoding.
#[derive(Debug)]
pub enum DecodeError {
    /// Indicates that the input string is not exactly 7 characters long.
    InvalidLength,
    /// Indicates that the input string contains an invalid character (not a base-36 character).
    InvalidCharacter,
    /// Indicates that the decoded value would exceed the 32-bit range for an IPv4 address.
    Overflow,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidLength => write!(f, "Encoded string must be 7 characters"),
            Self::InvalidCharacter => write!(f, "Invalid character in encoded string"),
            Self::Overflow => write!(f, "Encoded value exceeds 32 bits"),
        }
    }
}

impl Error for DecodeError {}

/// Decodes a 7-character base-36 encoded string back into an IPv4 address string.
///
/// This function takes a 7-character base-36 encoded string and converts it back
/// into a standard IPv4 address string in dotted decimal notation (e.g., "192.168.1.1").
///
/// # Arguments
///
/// * `s` - A string slice representing the 7-character base-36 encoded string.
///
/// # Returns
///
/// * `Result<String, DecodeError>` - On success, returns `Ok` containing the IPv4 address string.
///   On failure, returns `Err` containing a `DecodeError`.
///
/// # Errors
///
/// * `DecodeError::InvalidLength`: If the input string `s` is not exactly 7 characters long.
/// * `DecodeError::InvalidCharacter`: If the input string `s` contains characters that are not valid
///   base-36 characters (0-9, a-z, A-Z).
/// * `DecodeError::Overflow`: If the decoded value exceeds the 32-bit range, which should not happen
///   for a valid 7-character base-36 encoded IPv4 address, but is checked for safety.
pub fn decode_ipv4(s: &str) -> Result<String, DecodeError> {
    // Check if the input string length is exactly 7 characters.
    // If not, return an `InvalidLength` error.
    if s.len() != 7 {
        return Err(DecodeError::InvalidLength);
    }

    // Initialize a u32 variable to accumulate the decoded integer value.
    let mut num: u32 = 0;
    // Iterate through each character in the input string.
    for c in s.chars() {
        // Convert each character to its corresponding base-36 digit value.
        let d = match c {
            '0'..='9' => c as u8 - b'0',      // Digits '0'-'9' are values 0-9.
            'a'..='z' => c as u8 - b'a' + 10, // Lowercase 'a'-'z' are values 10-35.
            'A'..='Z' => c as u8 - b'A' + 10, // Uppercase 'A'-'Z' are also values 10-35 (case-insensitive).
            _ => return Err(DecodeError::InvalidCharacter), // If character is not in base-36 range, return `InvalidCharacter` error.
        };
        // Multiply the current accumulated value by 36 (base) and add the new digit value.
        // Use `checked_mul` and `checked_add` to detect potential overflow.
        num = num
            .checked_mul(36)
            .and_then(|n| n.checked_add(d as u32))
            .ok_or(DecodeError::Overflow)?; // If overflow occurs, return `Overflow` error.
    }

    // Convert the decoded 32-bit integer back into four 8-bit octets of an IPv4 address.
    let octets = [
        (num >> 24) as u8, // Get the first octet (most significant byte) by right-shifting 24 bits.
        (num >> 16) as u8, // Get the second octet by right-shifting 16 bits.
        (num >> 8) as u8,  // Get the third octet by right-shifting 8 bits.
        num as u8,         // Get the fourth octet (least significant byte).
    ];
    // Format the octets into a dotted decimal IPv4 address string.
    Ok(format!(
        "{}.{}.{}.{}",
        octets[0], octets[1], octets[2], octets[3]
    ))
}

/// Represents errors that can occur during URL encoding or decoding related to RuneScape server URLs.
#[derive(Debug)]
pub enum UrlError {
    /// Indicates that the URL does not start with the expected protocol "rnsrv://".
    InvalidProtocol,
    /// Indicates that the format of the URL after the protocol prefix is invalid.
    InvalidFormat,
    /// Wraps a `DecodeError` that occurred during IPv4 address decoding within the URL.
    DecodeError(DecodeError),
}

impl From<DecodeError> for UrlError {
    fn from(e: DecodeError) -> Self {
        Self::DecodeError(e)
    }
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidProtocol => write!(f, "Invalid URL protocol: expected 'rnsrv://'"),
            Self::InvalidFormat => write!(f, "Invalid URL format after protocol prefix"),
            Self::DecodeError(e) => write!(f, "IP address decode error: {e}"),
        }
    }
}

impl std::error::Error for UrlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DecodeError(e) => Some(e),
            _ => None,
        }
    }
}

/// Encodes a slice of IPv4 address strings into a RuneScape server URL format.
///
/// This function takes a slice of IPv4 address strings, encodes each IP address using `encode_ipv4`,
/// and concatenates them into a URL string with the "rnsrv://" protocol prefix.
///
/// # Arguments
///
/// * `ips` - A slice of string slices, where each string slice represents an IPv4 address in
///   dotted decimal notation.
///
/// # Returns
///
/// * `Result<String, EncodeError>` - On success, returns `Ok` containing the RuneScape server URL string.
///   On failure, returns `Err` containing an `EncodeError` if any of the IPv4 addresses fail to encode.
///
/// # Errors
///
/// * `EncodeError::InvalidIPv4`: If any of the IPv4 addresses in the input slice are invalid.
///   This error is propagated from the `encode_ipv4` function.
pub fn encode_rnsrv_url(ips: &[&str]) -> Result<String, EncodeError> {
    // Initialize a String buffer to build the URL efficiently.
    // Pre-allocate capacity to avoid reallocations, assuming 7 characters per IP and 8 for "rnsrv://".
    let mut buffer = String::with_capacity(7 * ips.len() + 8);
    // Push the "rnsrv://" protocol prefix to the buffer.
    buffer.push_str("rnsrv://");

    // Iterate through each IPv4 address string in the input slice.
    for ip in ips {
        // Encode each IPv4 address using `encode_ipv4` and append the result to the buffer.
        // Propagate any `EncodeError` from `encode_ipv4`.
        buffer.push_str(&encode_ipv4(ip)?);
    }

    // Return the built URL string.
    Ok(buffer)
}

/// Decodes a RuneScape server URL string back into a vector of IPv4 address strings.
///
/// This function takes a RuneScape server URL string, verifies the "rnsrv://" protocol prefix,
/// and decodes the encoded IPv4 addresses within the URL using `decode_ipv4`.
///
/// # Arguments
///
/// * `url` - A string slice representing the RuneScape server URL string.
///
/// # Returns
///
/// * `Result<Vec<String>, UrlError>` - On success, returns `Ok` containing a vector of IPv4 address strings.
///   On failure, returns `Err` containing a `UrlError`.
///
/// # Errors
///
/// * `UrlError::InvalidProtocol`: If the input URL does not start with "rnsrv://".
/// * `UrlError::InvalidFormat`: If the part of the URL after "rnsrv://" is not a valid sequence
///   of 7-character encoded IPv4 addresses (i.e., length is not a multiple of 7).
/// * `UrlError::DecodeError`: If any of the 7-character chunks within the URL fail to decode
///   into a valid IPv4 address. This wraps a `DecodeError` from the `decode_ipv4` function.
pub fn decode_rnsrv_url(url: &str) -> Result<Vec<String>, UrlError> {
    // Check if the URL starts with the "rnsrv://" protocol prefix.
    // If not, return an `InvalidProtocol` error.
    if !url.starts_with("rnsrv://") {
        error!("Unable to parse: {url}");
        return Err(UrlError::InvalidProtocol);
    }

    // Extract the encoded part of the URL, which is after the "rnsrv://" prefix.
    let encoded = &url[8..];
    // Check if the length of the encoded part is a multiple of 7.
    // If not, the format is invalid, return `InvalidFormat` error.
    if encoded.len() % 7 != 0 {
        return Err(UrlError::InvalidFormat);
    }

    // Initialize a vector to store the decoded IPv4 address strings.
    let mut ips = Vec::with_capacity(encoded.len() / 7);
    // Iterate over the encoded string in chunks of 7 characters.
    for chunk in encoded.as_bytes().chunks_exact(7) {
        // Convert each 7-character chunk to a string slice.
        // This should be safe as we are expecting base-36 ASCII characters.
        let s = from_utf8(chunk).map_err(|_| UrlError::InvalidFormat)?;
        // Decode each 7-character chunk using `decode_ipv4` and push the result to the `ips` vector.
        // Propagate any `DecodeError` from `decode_ipv4` by converting it to `UrlError::DecodeError`.
        ips.push(decode_ipv4(s)?);
    }

    // Return the vector of decoded IPv4 address strings.
    Ok(ips)
}
