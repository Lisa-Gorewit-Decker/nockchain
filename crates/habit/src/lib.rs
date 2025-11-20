use bytes::Bytes;

/// Sequential little-endian bit reader over an immutable byte buffer
pub struct BitReader {
    bytes: Bytes,
    bit_len: usize,
    bit_pos: usize,
}

impl BitReader {
    #[inline]
    pub fn new(bytes: Bytes) -> Self {
        let bit_len = bytes.len() * 8;
        Self {
            bytes,
            bit_len,
            bit_pos: 0,
        }
    }

    #[inline]
    pub fn position(&self) -> usize {
        self.bit_pos
    }

    #[inline]
    pub fn bits_remaining(&self) -> usize {
        self.bit_len.saturating_sub(self.bit_pos)
    }

    #[inline]
    pub fn read_bit(&mut self) -> Option<bool> {
        if self.bit_pos >= self.bit_len {
            return None;
        }
        let byte = self.bytes[self.bit_pos >> 3];
        let bit = ((byte >> (self.bit_pos & 7)) & 1) == 1;
        self.bit_pos += 1;
        Some(bit)
    }

    #[inline]
    pub fn read_bits_to_usize(&mut self, count: usize) -> Option<usize> {
        self.read_bits_to_u64(count).map(|v| v as usize)
    }

    #[inline]
    pub fn read_bits_to_bytes(&mut self, dest: &mut [u8], bits: usize) -> Option<()> {
        if bits == 0 {
            return Some(());
        }
        let bytes_needed = (bits + 7) >> 3;
        if dest.len() < bytes_needed || self.bits_remaining() < bits {
            return None;
        }
        let full_bytes = bits / 8;
        for dest_byte in dest.iter_mut().take(full_bytes) {
            *dest_byte = self.read_bits_to_u64(8)? as u8;
        }
        let rem = bits & 7;
        if rem > 0 {
            dest[full_bytes] = self.read_bits_to_u64(rem)? as u8;
        }
        Some(())
    }

    #[inline]
    pub fn read_unary(&mut self) -> Option<usize> {
        let mut zeros = 0usize;
        let mut pos = self.bit_pos;
        while pos < self.bit_len {
            let byte_idx = pos >> 3;
            let bit_offset = pos & 7;
            let available = (self.bit_len - pos).min(8 - bit_offset);
            if available == 0 {
                break;
            }
            let mut byte = self.bytes[byte_idx] >> bit_offset;
            if available < 8 - bit_offset {
                let mask = if available == 8 {
                    0xFF
                } else {
                    ((1u16 << available) - 1) as u8
                };
                byte &= mask;
            }
            if byte != 0 {
                let tz = byte.trailing_zeros() as usize;
                zeros += tz;
                pos += tz + 1;
                self.bit_pos = pos;
                return Some(zeros);
            } else {
                zeros += available;
                pos += available;
            }
        }
        None
    }

    #[inline]
    fn read_bits_to_u64(&mut self, count: usize) -> Option<u64> {
        if count == 0 {
            return Some(0);
        }
        if count > 64 || self.bits_remaining() < count {
            return None;
        }
        let mut value = 0u64;
        let mut bits_read = 0usize;
        let mut pos = self.bit_pos;
        while bits_read < count {
            let byte_idx = pos >> 3;
            let bit_offset = pos & 7;
            let take = (count - bits_read).min(8 - bit_offset);
            let mask = if take == 8 {
                0xFF
            } else {
                ((1u16 << take) - 1) as u8
            };
            let chunk = ((self.bytes[byte_idx] >> bit_offset) & mask) as u64;
            value |= chunk << bits_read;
            pos += take;
            bits_read += take;
        }
        self.bit_pos = pos;
        Some(value)
    }
}

// Fast bit writer that appends bits LSB-first into an underlying Vec<u8>
pub struct BitWriter {
    buf: Vec<u8>,   // final byte buffer (little-endian bit order per byte)
    acc: u8,        // in-progress byte accumulator
    nbits: u8,      // number of bits currently stored in `acc` (0-7)
    bit_len: usize, // total number of bits written so far
}
impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BitWriter {
    #[inline]
    pub fn new() -> Self {
        BitWriter {
            buf: Vec::with_capacity(1024),
            acc: 0,
            nbits: 0,
            bit_len: 0,
        }
    }

    #[inline]
    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    #[inline]
    pub fn write_bit(&mut self, bit: bool) {
        if bit {
            self.acc |= 1 << self.nbits;
        }
        self.nbits += 1;
        self.bit_len += 1;
        if self.nbits == 8 {
            self.flush_acc();
        }
    }

    #[inline]
    pub fn write_zeros(&mut self, count: usize) {
        // produce `count` zero bits quickly
        // Fill partial acc first
        let mut remaining = count;
        if self.nbits != 0 {
            let space = 8 - self.nbits;
            if remaining < space as usize {
                // // just bump counters – acc already contains zeros in high bits
                // self.nbits += remaining as u8;
                // self.bit_len += remaining;
                // return;
                // keep the valid low bits we already had, clear the bits we are about to add
                let mask = (1u16 << self.nbits) - 1; // e.g. nbits = 3  -> 0b00000111
                self.acc &= mask as u8; // zero out bits [self.nbits .. 7]

                // now bump the cursors exactly as before
                self.nbits += remaining as u8;
                self.bit_len += remaining;
                return;
            } else {
                // fill acc with zeros and flush
                // self.nbits = 8;
                // self.bit_len += space as usize;
                // remaining -= space as usize;
                // zero-fill high bits we are about to claim
                let mask = (1u16 << self.nbits) - 1; // keep the `nbits` low bits
                self.acc &= mask as u8; // clear [self.nbits .. 7]

                // now top-off the byte and flush
                self.nbits = 8;
                self.bit_len += space as usize;
                remaining -= space as usize;
                self.flush_acc();
            }
        }
        // Now we are byte-aligned
        let full_bytes = remaining / 8;
        if full_bytes > 0 {
            self.buf.extend(std::iter::repeat_n(0u8, full_bytes));
            self.bit_len += full_bytes * 8;
            remaining -= full_bytes * 8;
        }
        // Remaining < 8, leave in acc (which is zero)
        self.nbits = remaining as u8;
        self.acc = 0; // already zero
        self.bit_len += remaining;
    }

    #[inline]
    pub fn write_bits_from_value(&mut self, mut value: usize, count: usize) {
        for _ in 0..count {
            self.write_bit((value & 1) == 1);
            value >>= 1;
        }
    }

    #[inline]
    pub fn write_bits_from_le_bytes(&mut self, bytes: &[u8], total_bits: usize) {
        if total_bits == 0 {
            return;
        }

        let full_bytes = total_bits / 8;
        let rem_bits: usize = total_bits % 8;

        if self.nbits == 0 {
            // Aligned path: copy full bytes directly
            if full_bytes > 0 {
                self.buf.extend_from_slice(&bytes[..full_bytes]);
                self.bit_len += full_bytes * 8;
            }
        } else if full_bytes > 0 {
            // Unaligned path: merge each byte with current accumulator
            let shift = self.nbits;
            let mut carry = self.acc;
            for &byte in &bytes[..full_bytes] {
                let combined = carry | (byte << shift);
                self.buf.push(combined);
                self.bit_len += 8;
                carry = byte >> (8 - shift);
            }
            self.acc = carry;
            // note: nbits unchanged
        }

        // Handle remaining bits (<8) from the next byte
        if rem_bits > 0 {
            let src_byte = if full_bytes < bytes.len() {
                bytes[full_bytes]
            } else {
                0
            };
            for i in 0..rem_bits {
                self.write_bit(((src_byte >> i) & 1) == 1);
            }
        }
        // Update bit_len to reflect the total number of bits written so far
        // This didn't work.
        // self.bit_len = self.buf.len() * 8 + self.nbits as usize;
    }

    #[inline]
    pub fn flush_acc(&mut self) {
        if self.nbits == 0 {
            return;
        }
        self.buf.push(self.acc);
        self.acc = 0;
        self.nbits = 0;
    }

    pub fn into_bytes(mut self) -> Bytes {
        if self.nbits > 0 {
            // Flush final partial byte (upper bits remain 0)
            self.flush_acc();
        }
        Bytes::from(self.buf)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use quickcheck::TestResult;

    use super::{BitReader, BitWriter};

    fn read_bits_to_vec(mut reader: BitReader, bits: usize) -> Option<Vec<u8>> {
        if bits == 0 {
            return Some(Vec::new());
        }
        let mut out = vec![0u8; (bits + 7) >> 3];
        reader.read_bits_to_bytes(&mut out, bits)?;
        Some(out)
    }

    #[test]
    fn reads_unary_across_bytes() {
        let bytes = Bytes::from(vec![0b0000_0000, 0b0001_0000]);
        let mut reader = BitReader::new(bytes);
        let zeros = reader.read_unary().expect("unary should succeed");
        assert_eq!(zeros, 12);
        assert_eq!(reader.position(), 13);
    }

    #[test]
    fn reads_bits_to_u64_with_offset() {
        let bytes = Bytes::from(vec![0b1010_1100, 0b0101_1110]);
        let mut reader = BitReader::new(bytes);
        assert_eq!(reader.read_bits_to_u64(3), Some(0b100));
        assert_eq!(reader.read_bits_to_u64(5), Some(0b10101));
        assert_eq!(reader.read_bits_to_u64(8), Some(0b0101_1110));
        assert_eq!(reader.read_bits_to_u64(1), None);
    }

    #[test]
    fn writes_zeros_preserve_existing_bits() {
        let mut writer = BitWriter::new();
        writer.write_bits_from_value(0b101, 3);
        writer.write_zeros(5);
        writer.write_bits_from_value(0b11, 2);
        assert_eq!(writer.bit_len(), 10);
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(bytes);
        let mut out = [0u8; 2];
        reader.read_bits_to_bytes(&mut out, 10).expect("read bits");
        assert_eq!(out[0], 0b0000_0101);
        assert_eq!(out[1] & 0b11, 0b11);
    }

    #[test]
    fn write_bits_from_le_bytes_roundtrip() {
        let bytes = Bytes::from(vec![0xA5, 0x5A, 0xFF]);
        let mut writer = BitWriter::new();
        writer.write_bits_from_le_bytes(&bytes, 20);
        let written = writer.into_bytes();
        let mut reader = BitReader::new(written);
        let mut out = [0u8; 3];
        reader.read_bits_to_bytes(&mut out, 20).expect("read bits");
        assert_eq!(out[0], 0xA5);
        assert_eq!(out[1], 0x5A);
        assert_eq!(out[2] & 0x0F, 0x0F);
    }

    #[test]
    fn write_zeros_then_read_zero_bits() {
        let mut writer = BitWriter::new();
        writer.write_zeros(24);
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(bytes);
        let mut out = [0xFFu8; 3];
        reader.read_bits_to_bytes(&mut out, 24).expect("read bits");
        assert_eq!(out, [0u8; 3]);
    }

    #[test]
    fn read_bits_to_bytes_exact_remaining() {
        let bytes = Bytes::from(vec![0x01]);
        let mut reader = BitReader::new(bytes);
        let mut out = [0u8; 1];
        assert!(reader.read_bits_to_bytes(&mut out, 8).is_some());
        assert_eq!(out[0], 0x01);
        assert!(reader.read_bits_to_bytes(&mut out, 1).is_none());
    }

    #[test]
    fn write_zeros_byte_aligned() {
        let mut writer = BitWriter::new();
        writer.write_zeros(16);
        assert_eq!(writer.bit_len(), 16);
        let bytes = writer.into_bytes();
        assert_eq!(bytes.as_ref(), &[0u8, 0u8]);
    }

    #[test]
    fn read_bits_returns_none_when_too_long() {
        let bytes = Bytes::from(vec![0xFF]);
        let mut reader = BitReader::new(bytes);
        assert!(reader.read_bits_to_u64(65).is_none());
    }

    quickcheck::quickcheck! {
        fn prop_write_then_read_bits(payload: Vec<u8>, bit_count: usize) -> TestResult {
            let bits = bit_count % 129;
            if bits == 0 {
                return TestResult::passed();
            }
            let needed = (bits + 7) >> 3;
            let mut data = payload;
            data.resize(needed, 0u8);

            let mut writer = BitWriter::new();
            writer.write_bits_from_le_bytes(&data, bits);
            let written = writer.into_bytes();

            let reader = BitReader::new(written);
            let read_back = read_bits_to_vec(reader, bits);
            if let Some(mut bytes) = read_back {
                let mask_bits = bits & 7;
                if mask_bits != 0 {
                    let mask = (1u8 << mask_bits) - 1;
                    let last_idx = needed - 1;
                    bytes[last_idx] &= mask;
                    data[last_idx] &= mask;
                }
                return TestResult::from_bool(bytes == data);
            }

            TestResult::error("failed to read written bits")
        }

        fn prop_zero_bits_roundtrip(count: usize) -> TestResult {
            let bits = count % 257;
            let mut writer = BitWriter::new();
            writer.write_zeros(bits);
            let bytes = writer.into_bytes();
            let reader = BitReader::new(bytes);
            let read_back = read_bits_to_vec(reader, bits).unwrap_or_default();
        let expected = vec![0u8; (bits + 7) >> 3];
        TestResult::from_bool(read_back == expected)

        }
    }
}
