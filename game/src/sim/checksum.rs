//! FNV-1a: a simple 64-bit hash, written out by hand so it's identical on
//! every platform (native and WASM) and every Rust version. Rust's built-in
//! `DefaultHasher` makes no such promise.
//!
//! Used to fingerprint the whole game state: if two machines' checksums for
//! the same frame differ, their games have desynced.

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

pub struct Fnv1a {
    hash: u64,
}

impl Fnv1a {
    pub fn new() -> Fnv1a {
        Fnv1a { hash: OFFSET_BASIS }
    }

    /// For each byte: xor it in, then multiply by the prime. Multiplying
    /// spreads every byte's influence across all 64 bits.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= b as u64;
            self.hash = self.hash.wrapping_mul(PRIME);
        }
    }

    // Numbers are fed in little-endian byte order on purpose: `to_le_bytes`
    // gives the same bytes on every CPU, so the hash does too.
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }
    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_i8(&mut self, v: i8) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_i16(&mut self, v: i16) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(v as u8);
    }

    pub fn finish(&self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(bytes: &[u8]) -> u64 {
        let mut h = Fnv1a::new();
        h.write_bytes(bytes);
        h.finish()
    }

    // Published FNV-1a 64-bit test vectors, so we know this is the real
    // algorithm and not a near miss.
    #[test]
    fn known_test_vectors() {
        assert_eq!(hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn numbers_are_little_endian() {
        let mut h = Fnv1a::new();
        h.write_u32(0x0403_0201);
        assert_eq!(h.finish(), hash(&[1, 2, 3, 4]));
    }
}
