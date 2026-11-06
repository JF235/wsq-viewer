//! Huffman tables (Annex C) and the JPEG Annex K procedure that builds them.

use crate::bits::BitReader;
use crate::error::{Error, Result};

/// A table as transmitted: code counts per length and the symbols by code length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuffTable {
    pub bits: [u8; 16],
    pub values: Vec<u8>,
}

impl HuffTable {
    /// Code length and code of every entry of `values`, in order (Figures C.1 and C.2).
    pub fn canonical_codes(&self) -> Result<Vec<(u8, u16)>> {
        let mut out = Vec::with_capacity(self.values.len());
        let mut code: u32 = 0;
        for len in 1..=16u32 {
            let n = self.bits[len as usize - 1] as u32;
            if code + n > (1 << len) {
                return Err(Error::BadHuffmanTable);
            }
            for _ in 0..n {
                out.push((len as u8, code as u16));
                code += 1;
            }
            code <<= 1;
        }
        if out.len() != self.values.len() {
            return Err(Error::BadHuffmanTable);
        }
        Ok(out)
    }
}

const LUT_BITS: u32 = 10;

/// Decoding form of a table: a direct lookup for codes up to LUT_BITS long and the
/// MAXCODE/VALPTR arrays of Figure F.16 of JPEG for longer ones.
pub struct DecodeTable {
    /// Entry for every LUT_BITS-bit prefix: (length << 8) | symbol, or 0 for a longer code.
    lut: Vec<u16>,
    maxcode: [i32; 17],
    mincode: [i32; 17],
    valptr: [u16; 17],
    values: Vec<u8>,
}

impl DecodeTable {
    pub fn new(table: &HuffTable) -> Result<Self> {
        let codes = table.canonical_codes()?;
        let mut lut = vec![0u16; 1 << LUT_BITS];
        let mut maxcode = [-1i32; 17];
        let mut mincode = [0i32; 17];
        let mut valptr = [0u16; 17];
        let mut i = 0usize;
        for len in 1..=16usize {
            let n = table.bits[len - 1] as usize;
            if n == 0 {
                continue;
            }
            valptr[len] = i as u16;
            mincode[len] = codes[i].1 as i32;
            maxcode[len] = codes[i + n - 1].1 as i32;
            i += n;
        }
        for (k, &(len, code)) in codes.iter().enumerate() {
            let len = len as u32;
            if len <= LUT_BITS {
                let shift = LUT_BITS - len;
                let base = (code as usize) << shift;
                let entry = ((len as u16) << 8) | table.values[k] as u16;
                for e in &mut lut[base..base + (1 << shift)] {
                    *e = entry;
                }
            }
        }
        Ok(DecodeTable { lut, maxcode, mincode, valptr, values: table.values.clone() })
    }

    /// Decode one symbol. Returns None if the bits are not a code of this table.
    #[inline(always)]
    pub fn decode(&self, r: &mut BitReader) -> Option<u8> {
        let e = self.lut[r.peek(LUT_BITS) as usize];
        if e != 0 {
            r.consume((e >> 8) as u32);
            return Some(e as u8);
        }
        self.decode_long(r)
    }

    #[cold]
    fn decode_long(&self, r: &mut BitReader) -> Option<u8> {
        let bits = r.peek(16) as i32;
        for len in (LUT_BITS as usize + 1)..=16 {
            let code = bits >> (16 - len);
            if self.maxcode[len] >= 0 && code <= self.maxcode[len] {
                r.consume(len as u32);
                let idx = self.valptr[len] as i32 + code - self.mincode[len];
                return self.values.get(idx as usize).copied();
            }
        }
        None
    }
}

/// What a WSQ symbol means, packed for the decoding loop.
pub const KIND_VALUE: u32 = 0;
pub const KIND_RUN: u32 = 1;
pub const KIND_OTHER: u32 = 2;

/// Decoding table specialized for the WSQ alphabet: every LUT entry gives the code length
/// (bits 0-4, 0 for codes longer than the LUT), the kind (bits 5-6) and, for values and runs,
/// the signed payload (bits 8-31); escapes and long codes go through `slow`.
pub struct WsqTable {
    lut: Box<[u32; 1 << LUT_BITS]>,
    pub slow: DecodeTable,
}

/// Kind and payload of a symbol.
#[inline(always)]
pub fn classify(sym: u8) -> (u32, i32) {
    match sym {
        1..=100 => (KIND_RUN, sym as i32),
        107..=179 | 181..=254 => (KIND_VALUE, sym as i32 - 180),
        _ => (KIND_OTHER, sym as i32),
    }
}

impl WsqTable {
    pub fn new(table: &HuffTable) -> Result<Self> {
        let slow = DecodeTable::new(table)?;
        let mut lut = Box::new([0u32; 1 << LUT_BITS]);
        for (i, e) in slow.lut.iter().enumerate() {
            if *e != 0 {
                let len = (*e >> 8) as u32;
                let (kind, payload) = classify(*e as u8);
                lut[i] = len | (kind << 5) | ((payload as u32) << 8);
            }
        }
        Ok(WsqTable { lut, slow })
    }

    /// The packed entry for the next bits (length 0 means: use `slow`).
    #[inline(always)]
    pub fn entry(&self, r: &mut BitReader) -> u32 {
        // SAFETY: peek(LUT_BITS) < 1 << LUT_BITS
        unsafe { *self.lut.get_unchecked(r.peek(LUT_BITS) as usize) }
    }
}

/// Code and length of every symbol, for encoding.
pub fn encode_table(table: &HuffTable) -> Result<[(u16, u8); 256]> {
    let mut out = [(0u16, 0u8); 256];
    for (k, (len, code)) in table.canonical_codes()?.into_iter().enumerate() {
        out[table.values[k] as usize] = (code, len);
    }
    Ok(out)
}

/// Optimal table for the symbol counts `freq` (JPEG Annex K.2 to K.4).
///
/// A dummy symbol with count 1 takes the longest code, which is all ones, and is dropped at the
/// end. Ties between equal counts go to the larger symbol value first, as in NBIS, which makes
/// the tables identical to those of the NIST encoder.
pub fn optimal_table(freq: &[u32; 256]) -> HuffTable {
    const N: usize = 257;
    if freq.iter().all(|&c| c == 0) {
        // a block without symbols: an empty (but valid) table
        return HuffTable { bits: [0; 16], values: Vec::new() };
    }
    let mut f = [0u64; N];
    for (i, &c) in freq.iter().enumerate() {
        f[i] = c as u64;
    }
    f[256] = 1;
    let mut codesize = [0u32; N];
    let mut others = [-1i32; N];
    loop {
        // smallest and second smallest nonzero counts; ties to the larger index
        let (mut v1, mut v2) = (usize::MAX, usize::MAX);
        for i in 0..N {
            if f[i] == 0 {
                continue;
            }
            if v1 == usize::MAX || f[i] < f[v1] || (f[i] == f[v1] && i > v1) {
                v2 = v1;
                v1 = i;
            } else if v2 == usize::MAX || f[i] < f[v2] || (f[i] == f[v2] && i > v2) {
                v2 = i;
            }
        }
        if v2 == usize::MAX {
            break;
        }
        f[v1] += f[v2];
        f[v2] = 0;
        let mut v = v1;
        codesize[v] += 1;
        while others[v] != -1 {
            v = others[v] as usize;
            codesize[v] += 1;
        }
        others[v] = v2 as i32;
        let mut v = v2;
        codesize[v] += 1;
        while others[v] != -1 {
            v = others[v] as usize;
            codesize[v] += 1;
        }
    }
    let mut b = [0i32; 64];
    for &s in &codesize {
        if s > 0 {
            b[s as usize] += 1;
        }
    }
    // Figure K.3: limit code lengths to 16 bits
    let mut i = 63usize;
    while i > 16 {
        while b[i] > 0 {
            let mut j = i - 2;
            while b[j] == 0 {
                j -= 1;
            }
            b[i] -= 2;
            b[i - 1] += 1;
            b[j + 1] += 2;
            b[j] -= 1;
        }
        i -= 1;
    }
    while b[i] == 0 {
        i -= 1;
    }
    b[i] -= 1; // drop the dummy symbol's code
    let mut bits = [0u8; 16];
    for l in 1..=16 {
        bits[l - 1] = b[l] as u8;
    }
    let mut values = Vec::new();
    for size in 1..64u32 {
        for (sym, &cs) in codesize.iter().enumerate().take(256) {
            if cs == size {
                values.push(sym as u8);
            }
        }
    }
    HuffTable { bits, values }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bits::BitWriter;

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *seed >> 33
    }

    #[test]
    fn example_from_the_document() {
        let t = HuffTable { bits: [0, 2, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], values: vec![1, 181, 179, 2, 182, 3] };
        let codes = t.canonical_codes().unwrap();
        assert_eq!(codes, vec![(2, 0b00), (2, 0b01), (3, 0b100), (3, 0b101), (3, 0b110), (4, 0b1110)]);
    }

    #[test]
    fn optimal_tables_roundtrip_and_avoid_all_ones() {
        let mut seed = 7;
        for trial in 0..200 {
            let mut freq = [0u32; 256];
            let nsym = 2 + (lcg(&mut seed) % 200) as usize;
            for _ in 0..nsym {
                let s = (lcg(&mut seed) % 255) as usize + 1;
                // skewed counts to force long codes and length limiting
                freq[s] += 1 + (lcg(&mut seed) % (1 + (1u64 << (lcg(&mut seed) % 20)))) as u32;
            }
            let t = optimal_table(&freq);
            let codes = t.canonical_codes().unwrap();
            for &(len, code) in &codes {
                assert!(len <= 16);
                assert_ne!(code as u32, (1u32 << len) - 1, "all-ones code in trial {trial}");
            }
            let enc = encode_table(&t).unwrap();
            let dec = DecodeTable::new(&t).unwrap();
            let syms: Vec<u8> = (0..500)
                .map(|_| loop {
                    let s = (lcg(&mut seed) % 256) as usize;
                    if freq[s] > 0 {
                        break s as u8;
                    }
                })
                .collect();
            let mut w = BitWriter::with_capacity(1024);
            for &s in &syms {
                let (c, l) = enc[s as usize];
                w.put(c as u32, l as u32);
            }
            let bytes = w.finish();
            let plain: Vec<u8> = {
                let mut v = Vec::new();
                let mut i = 0;
                while i < bytes.len() {
                    v.push(bytes[i]);
                    i += if bytes[i] == 0xFF { 2 } else { 1 };
                }
                v
            };
            let mut r = BitReader::new(&plain);
            for &s in &syms {
                assert_eq!(dec.decode(&mut r), Some(s));
            }
            assert!(!r.overrun());
        }
    }
}
