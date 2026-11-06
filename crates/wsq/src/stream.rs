//! The compressed data format (Annex B): markers, table segments, frame and blocks.

use crate::error::{Error, Result};
use crate::huffman::HuffTable;

pub const SOI: u16 = 0xFFA0;
pub const EOI: u16 = 0xFFA1;
pub const SOF: u16 = 0xFFA2;
pub const SOB: u16 = 0xFFA3;
pub const DTT: u16 = 0xFFA4;
pub const DQT: u16 = 0xFFA5;
pub const DHT: u16 = 0xFFA6;
pub const DRI: u16 = 0xFFA7;
pub const COM: u16 = 0xFFA8;
pub const RST0: u16 = 0xFFB0;
pub const RST7: u16 = 0xFFB7;

/// A decimal scaled integer: `value / 10^exp` (Section 6.2 of the algorithm document).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scaled {
    pub exp: u8,
    pub value: u32,
}

impl Scaled {
    /// The value as NBIS computes it: convert to float, then divide by 10 once per exponent
    /// step, rounding to float after each division.
    pub fn nbis_f32(self) -> f32 {
        let mut x = self.value as f32;
        for _ in 0..self.exp {
            x = (x as f64 / 10.0) as f32;
        }
        x
    }

    /// The value correctly rounded.
    pub fn to_f64(self) -> f64 {
        self.value as f64 / 10f64.powi(self.exp as i32)
    }

    /// NBIS's encoding of a nonnegative float: the largest exponent for which the rounded value
    /// stays below `limit` (65535 for 16-bit fields).
    pub fn nbis_from_f32(v: f32) -> Scaled {
        if v == 0.0 {
            return Scaled { exp: 0, value: 0 };
        }
        let mut x = v;
        let mut exp: i32 = 0;
        while x < 65535.0 {
            exp += 1;
            x *= 10.0;
        }
        exp -= 1;
        let r = x as f64 / 10.0;
        Scaled { exp: exp as u8, value: (r + 0.5) as u32 }
    }

    /// Same, for the 32-bit filter coefficients (limit 2^32 - 1, arithmetic as in NBIS).
    pub fn nbis_from_f32_u32(v: f32) -> Scaled {
        if v == 0.0 {
            return Scaled { exp: 0, value: 0 };
        }
        let mut x = v;
        let mut exp: i32 = 0;
        while (x as f64) < 4294967295.0 {
            exp += 1;
            x = (x as f64 * 10.0) as f32;
        }
        exp -= 1;
        let r = x as f64 / 10.0;
        Scaled { exp: exp as u8, value: (r + 0.5) as u32 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub black: u8,
    pub white: u8,
    pub height: u16,
    pub width: u16,
    pub m: Scaled,
    pub r: Scaled,
    pub encoder: u8,
    pub software: u16,
}

/// One transmitted filter tap: sign and magnitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tap {
    pub negative: bool,
    pub magnitude: Scaled,
}

impl Tap {
    pub fn nbis_f32(self) -> f32 {
        let m = self.magnitude.nbis_f32();
        if self.negative { -m } else { m }
    }
    pub fn to_f64(self) -> f64 {
        let m = self.magnitude.to_f64();
        if self.negative { -m } else { m }
    }
}

/// Transform table: lengths of the analysis filters and their transmitted right halves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformTable {
    pub l0: u8,
    pub l1: u8,
    pub h0: Vec<Tap>,
    pub h1: Vec<Tap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantTable {
    pub c: Scaled,
    pub q: [Scaled; 64],
    pub z: [Scaled; 64],
}

/// The entropy-coded data of a block, with stuffing removed. `segments` holds the end offsets
/// of the restart intervals (one entry when there are no restart markers).
#[derive(Debug, Clone)]
pub struct Block {
    pub huffman: HuffTable,
    pub data: Vec<u8>,
    pub segments: Vec<usize>,
}

/// Tables installed in a decoder; they persist across streams (abbreviated formats).
#[derive(Debug, Clone, Default)]
pub struct Tables {
    pub transform: Option<TransformTable>,
    pub quant: Option<QuantTable>,
    pub huffman: [Option<HuffTable>; 8],
    pub restart_interval: u16,
}

#[derive(Debug, Clone)]
pub struct Stream {
    pub frame: Option<Frame>,
    pub blocks: Vec<Block>,
    pub comments: Vec<Vec<u8>>,
}

struct Cursor<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.d.get(self.p).ok_or(Error::Truncated)?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(((self.u8()? as u16) << 8) | self.u8()? as u16)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(((self.u16()? as u32) << 16) | self.u16()? as u32)
    }
    fn scaled16(&mut self) -> Result<Scaled> {
        let exp = self.u8()?;
        Ok(Scaled { exp, value: self.u16()? as u32 })
    }
    /// A marker, skipping any 0xFF fill bytes before it.
    fn marker(&mut self) -> Result<u16> {
        let start = self.p;
        if self.u8()? != 0xFF {
            return Err(Error::ExpectedMarker { offset: start });
        }
        let mut b = self.u8()?;
        while b == 0xFF {
            b = self.u8()?;
        }
        if b == 0 {
            return Err(Error::ExpectedMarker { offset: start });
        }
        Ok(0xFF00 | b as u16)
    }
}

/// Parse a stream, updating `tables` with every table it defines.
pub fn parse(data: &[u8], tables: &mut Tables) -> Result<Stream> {
    let mut c = Cursor { d: data, p: 0 };
    let first = c.marker()?;
    if first != SOI {
        return Err(Error::UnexpectedMarker { offset: 0, marker: first });
    }
    let mut stream = Stream { frame: None, blocks: Vec::new(), comments: Vec::new() };
    loop {
        let at = c.p;
        let m = c.marker()?;
        match m {
            EOI => break,
            SOF | SOB | DTT | DQT | DHT | DRI | COM => {}
            _ => return Err(Error::UnexpectedMarker { offset: at, marker: m }),
        }
        let seg_start = c.p;
        let len = c.u16()? as usize;
        if len < 2 {
            return Err(Error::BadSegment("length below 2"));
        }
        let seg_end = seg_start + len;
        if seg_end > data.len() {
            return Err(Error::Truncated);
        }
        match m {
            SOF => {
                if stream.frame.is_some() {
                    return Err(Error::UnexpectedMarker { offset: at, marker: m });
                }
                let f = Frame {
                    black: c.u8()?,
                    white: c.u8()?,
                    height: c.u16()?,
                    width: c.u16()?,
                    m: c.scaled16()?,
                    r: c.scaled16()?,
                    encoder: c.u8()?,
                    software: c.u16()?,
                };
                stream.frame = Some(f);
            }
            DTT => {
                let l0 = c.u8()?;
                let l1 = c.u8()?;
                if !(1..=32).contains(&l0) || !(1..=32).contains(&l1) || (l0 % 2) != (l1 % 2) {
                    return Err(Error::BadSegment("filter lengths"));
                }
                let mut halves = [Vec::new(), Vec::new()];
                for (h, l) in halves.iter_mut().zip([l0, l1]) {
                    let n = l.div_ceil(2) as usize;
                    for _ in 0..n {
                        let sign = c.u8()?;
                        let exp = c.u8()?;
                        let value = c.u32()?;
                        h.push(Tap { negative: sign != 0, magnitude: Scaled { exp, value } });
                    }
                }
                let [h0, h1] = halves;
                tables.transform = Some(TransformTable { l0, l1, h0, h1 });
            }
            DQT => {
                let cc = c.scaled16()?;
                let mut q = [Scaled::default(); 64];
                let mut z = [Scaled::default(); 64];
                for k in 0..64 {
                    q[k] = c.scaled16()?;
                    z[k] = c.scaled16()?;
                }
                tables.quant = Some(QuantTable { c: cc, q, z });
            }
            DHT => {
                while c.p < seg_end {
                    let th = c.u8()?;
                    if th > 7 {
                        return Err(Error::BadSegment("Huffman table number above 7"));
                    }
                    let mut bits = [0u8; 16];
                    for b in &mut bits {
                        *b = c.u8()?;
                    }
                    let n: usize = bits.iter().map(|&b| b as usize).sum();
                    if n > 256 {
                        return Err(Error::BadHuffmanTable);
                    }
                    let mut values = Vec::with_capacity(n);
                    for _ in 0..n {
                        values.push(c.u8()?);
                    }
                    tables.huffman[th as usize] = Some(HuffTable { bits, values });
                }
            }
            DRI => {
                tables.restart_interval = c.u16()?;
            }
            COM => {
                stream.comments.push(data[c.p..seg_end].to_vec());
            }
            SOB => {
                if stream.frame.is_none() {
                    return Err(Error::UnexpectedMarker { offset: at, marker: m });
                }
                let td = c.u8()?;
                let huffman = tables
                    .huffman
                    .get(td as usize)
                    .and_then(|t| t.clone())
                    .ok_or(Error::MissingTable("Huffman table"))?;
                if c.p != seg_end {
                    return Err(Error::BadSegment("block header length"));
                }
                let (block_data, segments, end) = read_entropy_coded(data, seg_end)?;
                stream.blocks.push(Block { huffman, data: block_data, segments });
                c.p = end;
                continue;
            }
            _ => unreachable!(),
        }
        if c.p > seg_end {
            return Err(Error::BadSegment("segment shorter than its content"));
        }
        c.p = seg_end;
    }
    Ok(stream)
}

/// Copy the entropy-coded data starting at `start`, removing stuffed zeros, up to the first
/// marker other than a restart marker. Returns the data, the end offsets of the restart
/// segments, and the position of that marker.
fn read_entropy_coded(data: &[u8], start: usize) -> Result<(Vec<u8>, Vec<usize>, usize)> {
    let mut out = Vec::with_capacity(data.len().saturating_sub(start));
    let mut segments = Vec::new();
    let mut p = start;
    loop {
        // copy the run of ordinary bytes
        let rest = &data[p..];
        let n = rest.iter().position(|&b| b == 0xFF).unwrap_or(rest.len());
        out.extend_from_slice(&rest[..n]);
        p += n;
        if p >= data.len() {
            return Err(Error::Truncated);
        }
        // at 0xFF
        let mut q = p + 1;
        while q < data.len() && data[q] == 0xFF {
            q += 1;
        }
        let next = *data.get(q).ok_or(Error::Truncated)?;
        if next == 0x00 && q == p + 1 {
            out.push(0xFF);
            p += 2;
            continue;
        }
        let marker = 0xFF00 | next as u16;
        if next == 0x00 {
            // fill bytes followed by a stuffed zero: the last 0xFF is data
            out.push(0xFF);
            p = q + 1;
            continue;
        }
        if (RST0..=RST7).contains(&marker) {
            segments.push(out.len());
            p = q + 1;
            continue;
        }
        segments.push(out.len());
        return Ok((out, segments, p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_roundtrip_nbis() {
        for &v in &[0.44f32, 1.0, 12.345, 0.0123, 255.0, 3.25e-3, 1.2345678] {
            let s = Scaled::nbis_from_f32(v);
            assert!(s.value < 65536);
            // four to five significant digits: below 65535 the relative step is at most 1e-4 / 2
            assert!(((s.to_f64() - v as f64) / v as f64).abs() < 5.1e-5, "{v} -> {s:?}");
        }
        assert_eq!(Scaled::nbis_from_f32(0.44), Scaled { exp: 5, value: 44000 });
    }

    #[test]
    fn entropy_coded_unstuffing_and_restarts() {
        let d = [0x12, 0xFF, 0x00, 0x34, 0xFF, 0xB0, 0x56, 0xFF, 0xFF, 0xA1];
        let (out, seg, end) = read_entropy_coded(&d, 0).unwrap();
        assert_eq!(out, vec![0x12, 0xFF, 0x34, 0x56]);
        assert_eq!(seg, vec![3, 4]);
        assert_eq!(end, 7);
    }
}
