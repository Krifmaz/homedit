//! Favicon hashing compatible with the widely used Shodan convention.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// MurmurHash3 (x86, 32-bit) of `data` with `seed`.
///
/// Written from the public algorithm description.
pub fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h = seed;
    let mut chunks = data.chunks_exact(4);
    for chunk in &mut chunks {
        let mut k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        k = k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h ^= k;
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        let mut k = 0u32;
        for (i, b) in tail.iter().enumerate() {
            k |= u32::from(*b) << (8 * i);
        }
        k = k.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h ^= k;
    }
    // Lengths above 4 GiB never reach here: favicons are capped far lower.
    h ^= data.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// Hashes favicon bytes the way public favicon databases do: base64 with a
/// newline after every 76 characters and at the end (MIME style), then
/// signed 32-bit MurmurHash3 with seed 0.
///
/// The same icon therefore gets the same number here as in those databases,
/// so known hashes can be added to the catalogue as
/// [`Signal::FaviconHash`](crate::catalogue::Signal::FaviconHash).
pub fn favicon_hash(bytes: &[u8]) -> i32 {
    let encoded = STANDARD.encode(bytes);
    let mut mime = String::with_capacity(encoded.len() + encoded.len() / 76 + 1);
    for line in encoded.as_bytes().chunks(76) {
        // Base64 output is ASCII, so this never fails.
        mime.push_str(std::str::from_utf8(line).unwrap_or_default());
        mime.push('\n');
    }
    murmur3_32(mime.as_bytes(), 0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur_vectors() {
        assert_eq!(murmur3_32(b"", 0), 0);
        assert_eq!(murmur3_32(b"", 1), 0x514e_28b7);
        assert_eq!(murmur3_32(b"hello", 0), 0x248b_fa47);
        assert_eq!(
            murmur3_32(b"The quick brown fox jumps over the lazy dog", 0),
            0x2e4f_f723
        );
    }

    #[test]
    fn favicon_hash_wraps_lines() {
        // 100 bytes encode to 136 base64 characters: two lines.
        let a = favicon_hash(&[7u8; 100]);
        let mut manual = STANDARD.encode([7u8; 100]);
        manual.insert(76, '\n');
        manual.push('\n');
        assert_eq!(a, murmur3_32(manual.as_bytes(), 0) as i32);
    }
}
