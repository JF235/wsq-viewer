//! Bit-level reading and writing of entropy-coded data (MSB first, Annex C.3).

/// Reads bits from an entropy-coded segment whose stuffed zero bytes were removed.
///
/// Past the end of the data the reader supplies 1-bits, like the padding an encoder writes,
/// and counts how many real bits were consumed so that callers can tell when a code ran past
/// the end.
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
    consumed: usize,
    limit: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let mut r = BitReader { data, pos: 0, buf: 0, count: 0, consumed: 0, limit: data.len() * 8 };
        r.refill();
        r
    }

    #[inline(always)]
    fn refill(&mut self) {
        if self.count > 56 {
            return;
        }
        if self.pos + 8 <= self.data.len() {
            // Fast path: one unaligned big-endian load. Bits below the accounted bytes are the
            // true next bits, so OR-ing them in again on the next refill changes nothing.
            let word = u64::from_be_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
            self.buf |= word >> self.count;
            let take = (63 - self.count) >> 3;
            self.pos += take as usize;
            self.count += take * 8;
        } else {
            while self.count <= 56 {
                let byte = if self.pos < self.data.len() { self.data[self.pos] } else { 0xFF };
                self.pos += 1;
                self.buf |= (byte as u64) << (56 - self.count);
                self.count += 8;
            }
        }
    }

    /// The next `n` bits (1..=32) without consuming them.
    #[inline(always)]
    pub fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.refill();
        }
        (self.buf >> (64 - n)) as u32
    }

    #[inline(always)]
    pub fn consume(&mut self, n: u32) {
        self.buf <<= n;
        self.count -= n;
        self.consumed += n as usize;
    }

    #[inline(always)]
    pub fn read(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.consume(n);
        v
    }

    /// True once more bits were consumed than the segment holds.
    #[inline(always)]
    pub fn overrun(&self) -> bool {
        self.consumed > self.limit
    }

    /// Real bits left in the segment.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.consumed)
    }
}

/// Writes bits MSB first, stuffing a zero byte after every 0xFF.
pub struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    pub fn with_capacity(cap: usize) -> Self {
        BitWriter { out: Vec::with_capacity(cap), acc: 0, n: 0 }
    }

    /// Append the low `nbits` bits of `value` (nbits <= 32).
    #[inline(always)]
    pub fn put(&mut self, value: u32, nbits: u32) {
        if nbits == 0 {
            return;
        }
        let v = (value as u64) & ((1u64 << nbits) - 1);
        self.acc = (self.acc << nbits) | v;
        self.n += nbits;
        if self.n >= 32 {
            self.n -= 32;
            let word = (self.acc >> self.n) as u32;
            if word.to_be_bytes().contains(&0xFF) {
                for b in word.to_be_bytes() {
                    self.out.push(b);
                    if b == 0xFF {
                        self.out.push(0x00);
                    }
                }
            } else {
                self.out.extend_from_slice(&word.to_be_bytes());
            }
            self.acc &= (1u64 << self.n) - 1;
        }
    }

    fn flush_bytes(&mut self) {
        while self.n >= 8 {
            self.n -= 8;
            let byte = (self.acc >> self.n) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00);
            }
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    /// Pad the last byte with 1-bits and return the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        self.flush_bytes();
        if self.n > 0 {
            let pad = 8 - self.n;
            self.acc = (self.acc << pad) | ((1u64 << pad) - 1);
            self.n += pad;
            self.flush_bytes();
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_stuffing() {
        let mut w = BitWriter::with_capacity(16);
        let fields = [(0b1, 1), (0xFF, 8), (0x3, 2), (0xABCD, 16), (0, 3), (0x7F, 7)];
        for &(v, n) in &fields {
            w.put(v, n);
        }
        let bytes = w.finish();
        // unstuff
        let mut plain = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            plain.push(bytes[i]);
            if bytes[i] == 0xFF {
                assert_eq!(bytes[i + 1], 0);
                i += 1;
            }
            i += 1;
        }
        let mut r = BitReader::new(&plain);
        for &(v, n) in &fields {
            assert_eq!(r.read(n), v);
        }
        assert!(!r.overrun());
        // the padding reads as ones
        let rem = r.remaining() as u32;
        if rem > 0 {
            assert_eq!(r.read(rem), (1 << rem) - 1);
        }
    }

    #[test]
    fn reader_matches_slow_path_on_long_data() {
        let data: Vec<u8> = (0..1000u32).map(|i| ((i * 2654435761u32) >> 13) as u8).collect();
        let mut r = BitReader::new(&data);
        let mut bitpos = 0usize;
        let widths = [1u32, 3, 7, 13, 16, 2, 9, 5, 11];
        let mut k = 0;
        while bitpos + 16 <= data.len() * 8 {
            let n = widths[k % widths.len()];
            k += 1;
            let mut expect = 0u32;
            for b in 0..n as usize {
                let p = bitpos + b;
                expect = (expect << 1) | ((data[p / 8] >> (7 - p % 8)) & 1) as u32;
            }
            assert_eq!(r.read(n), expect, "at bit {bitpos}");
            bitpos += n as usize;
        }
    }
}
