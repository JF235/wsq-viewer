//! The decoder (Algorithm 11 of the algorithm document).
//!
//! Decoding runs in four stages: the entropy decoder writes the quantizer indices, in stream
//! order, into a linear buffer; a vector pass dequantizes them into the subband rectangles of
//! the coefficient array; the inverse transform runs in place; a last vector pass converts to
//! pixels.

use crate::bits::BitReader;
use crate::error::{Error, Result};
use crate::huffman::{classify, WsqTable, KIND_RUN, KIND_VALUE};
use crate::nistcom;
use crate::simd::Backend;
use crate::stream::{self, Block, QuantTable, Stream, Tables};
use crate::transform::{self, Filters, Plan, ScheduleCache, Workspace};
use crate::tree::Tree;
use crate::{nbis, Clock, Image, Mode, Profile};

/// Decoder settings.
#[derive(Debug, Clone)]
pub struct DecodeOptions {
    /// `Exact` reproduces the NBIS reference decoder bit for bit; `Fast` allows a different
    /// operation order (results within one grey level).
    pub mode: Mode,
    /// Instruction set for the vector stages; `None` picks the best available.
    pub backend: Option<Backend>,
    /// Images with more pixels than this are refused before anything is allocated.
    pub max_pixels: u64,
    /// Threads used inside one decode (1: none).
    pub threads: usize,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        DecodeOptions { mode: Mode::Exact, backend: None, max_pixels: 1 << 28, threads: 1 }
    }
}

/// Decode a WSQ stream with default options.
pub fn decode(data: &[u8]) -> Result<Image> {
    decode_with(data, &DecodeOptions::default())
}

/// Decode a WSQ stream.
pub fn decode_with(data: &[u8], opts: &DecodeOptions) -> Result<Image> {
    Decoder::new(opts.clone()).decode(data)
}

/// A decoder that keeps tables between streams, as the abbreviated formats require, and reuses
/// its buffers and transform schedules between images.
pub struct Decoder {
    opts: DecodeOptions,
    tables: Tables,
    ws: Workspace,
    cache: ScheduleCache,
    coef: Vec<f32>,
    lin: Vec<f32>,
}

/// A transmitted subband: its place in the coefficient array, its size and its quantizer, and
/// where its indices start in the linear buffer.
#[derive(Clone, Copy)]
struct Band {
    k: usize,
    offset: usize,
    w: usize,
    h: usize,
    q: f32,
    z: f32,
    start: usize,
}

/// Decode the symbols of `blocks` into `lin[from..to]` (indices as floats). `lin` must be zero
/// on that range and have one slot past `to`.
fn decode_linear(blocks: &[Block], lin: &mut [f32], from: usize, to: usize) -> Result<()> {
    let mut k = from;
    let out = lin.as_mut_ptr();
    for block in blocks {
        let table = WsqTable::new(&block.huffman)?;
        let mut start = 0;
        for &end in &block.segments {
            let mut r = BitReader::new(&block.data[start..end]);
            start = end;
            while k < to {
                let e = table.entry(&mut r);
                let len = e & 31;
                let (kind, payload) = if len != 0 {
                    r.consume(len);
                    ((e >> 5) & 3, (e as i32) >> 8)
                } else {
                    // a long code, or bits that are no code (the padding at the end)
                    let Some(sym) = table.slow.decode(&mut r) else {
                        if r.remaining() < 16 {
                            break;
                        }
                        return Err(Error::BadHuffmanCode);
                    };
                    classify(sym)
                };
                if r.overrun() {
                    break; // an incomplete code: the padding
                }
                let (run, value) = if kind == KIND_VALUE || kind == KIND_RUN {
                    let is_run = (kind == KIND_RUN) as i32;
                    ((payload * is_run) as usize, payload * (1 - is_run))
                } else {
                    let v = match payload {
                        101 => (0, r.read(8) as i32),
                        102 => (0, -(r.read(8) as i32)),
                        103 => (0, r.read(16) as i32),
                        104 => (0, -(r.read(16) as i32)),
                        105 => (r.read(8) as usize, 0),
                        106 => (r.read(16) as usize, 0),
                        s => return Err(Error::BadSymbol(s as u8)),
                    };
                    if r.overrun() {
                        return Err(Error::Truncated);
                    }
                    v
                };
                // one path for runs and values: skip `run` zeros, then store the value (a run
                // stores a zero where the next value will go)
                k += run;
                let nz = (value != 0) as usize;
                if k + nz > to {
                    return Err(Error::CoefficientCount { expected: to - from, found: k + nz - from });
                }
                if k < to {
                    // SAFETY: k < to <= lin.len()
                    unsafe { *out.add(k) = value as f32 };
                }
                k += nz;
            }
        }
    }
    if k != to {
        return Err(Error::CoefficientCount { expected: to - from, found: k - from });
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct LinPtr(*mut f32, usize);
unsafe impl Send for LinPtr {}
unsafe impl Sync for LinPtr {}

impl LinPtr {
    /// Rebuild the slice (an accessor, so that closures capture the Send wrapper).
    unsafe fn slice<'a>(self) -> &'a mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.0, self.1) }
    }
}

/// Entropy-decode all blocks into `lin`. With `parallel` and the three blocks of encoder number
/// two, the blocks are decoded on three threads.
fn decode_all(stream: &Stream, bands: &[Band], lin: &mut [f32], total: usize, parallel: bool) -> Result<()> {
    if parallel && stream.blocks.len() == 3 {
        let start_of = |first: usize| bands.iter().find(|b| b.k >= first).map(|b| b.start).unwrap_or(total);
        let cuts = [0, start_of(19), start_of(52), total];
        let ptr = LinPtr(lin.as_mut_ptr(), lin.len());
        let results: Vec<Result<()>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..3)
                .map(|b| {
                    let block = &stream.blocks[b];
                    let (from, to) = (cuts[b], cuts[b + 1]);
                    // SAFETY: the threads write disjoint ranges [from, to) (decode_linear never
                    // writes at `to`)
                    s.spawn(move || decode_linear(std::slice::from_ref(block), unsafe { ptr.slice() }, from, to))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        return results.into_iter().collect();
    }
    decode_linear(&stream.blocks, lin, 0, total)
}

/// The dequantized coefficient array of an image, before the inverse transform.
#[derive(Debug, Clone)]
pub struct Coefficients {
    pub width: usize,
    pub height: usize,
    /// Normalization shift and scale of the frame header.
    pub m: f32,
    pub r: f32,
    /// Row-major, `width * height`; subband k occupies its rectangle of the subband tree.
    pub coef: Vec<f32>,
}

struct Prepared {
    stream: Stream,
    width: usize,
    height: usize,
    tree: Tree,
}

impl Decoder {
    pub fn new(opts: DecodeOptions) -> Self {
        Decoder {
            opts,
            tables: Tables::default(),
            ws: Workspace::default(),
            cache: ScheduleCache::default(),
            coef: Vec::new(),
            lin: Vec::new(),
        }
    }

    fn prepare(&mut self, data: &[u8]) -> Result<Prepared> {
        let stream = stream::parse(data, &mut self.tables)?;
        let frame = stream.frame.as_ref().ok_or(Error::BadSegment("no frame header"))?;
        let (width, height) = (frame.width as usize, frame.height as usize);
        if width == 0 || height == 0 {
            return Err(Error::BadDimensions);
        }
        if (width * height) as u64 > self.opts.max_pixels {
            return Err(Error::TooLarge { width: width as u32, height: height as u32 });
        }
        Ok(Prepared { stream, width, height, tree: Tree::new(width, height) })
    }

    fn bands(&self, p: &Prepared, qs: &[f32; 64], zs: &[f32; 64]) -> (Vec<Band>, usize) {
        let mut start = 0;
        let mut bands = Vec::with_capacity(64);
        for k in 0..64 {
            let r = p.tree.subbands[k];
            if qs[k] != 0.0 && !r.is_empty() {
                bands.push(Band { k, offset: r.y * p.width + r.x, w: r.w, h: r.h, q: qs[k], z: zs[k], start });
                start += r.len();
            }
        }
        (bands, start)
    }

    /// Install the tables of a table-specification stream (abbreviated format, Annex B.4), for
    /// later streams that omit them.
    pub fn install_tables(&mut self, data: &[u8]) -> Result<()> {
        stream::parse(data, &mut self.tables).map(|_| ())
    }

    /// Decode a stream (installing the tables it defines).
    pub fn decode(&mut self, data: &[u8]) -> Result<Image> {
        self.decode_inner(data, &mut Clock::new(false))
    }

    /// Decode and report the time of each stage.
    pub fn decode_profiled(&mut self, data: &[u8]) -> Result<(Image, Profile)> {
        let mut clock = Clock::new(true);
        let img = self.decode_inner(data, &mut clock)?;
        Ok((img, clock.finish()))
    }

    fn decode_inner(&mut self, data: &[u8], clock: &mut Clock) -> Result<Image> {
        let p = self.prepare(data)?;
        clock.lap("parse");
        let quant = self.tables.quant.clone().ok_or(Error::MissingTable("quantization table"))?;
        let transform = self.tables.transform.clone().ok_or(Error::MissingTable("transform table"))?;
        let exact = self.opts.mode == Mode::Exact;
        let backend = self.opts.backend.unwrap_or_else(Backend::best);
        if !backend.is_available() {
            return Err(Error::BadParameter("backend not available on this machine"));
        }
        let frame = p.stream.frame.clone().unwrap();
        let (width, height) = (p.width, p.height);
        let threads = self.opts.threads.max(1);

        let (qs, zs, c) = quant_values(&quant, exact);
        let (bands, total) = self.bands(&p, &qs, &zs);

        // entropy decoding into the linear buffer
        let mut lin = std::mem::take(&mut self.lin);
        lin.clear();
        lin.resize(total + 1, 0.0);
        clock.lap("zero");
        let res = decode_all(&p.stream, &bands, &mut lin, total, threads > 1);
        if let Err(e) = res {
            self.lin = lin;
            return Err(e);
        }
        clock.lap("entropy");

        // dequantization into the coefficient array
        let mut coef = std::mem::take(&mut self.coef);
        coef.resize(width * height, 0.0);
        for (k, &q) in qs.iter().enumerate() {
            let r = p.tree.subbands[k];
            if q == 0.0 {
                for y in r.y..r.y + r.h {
                    coef[y * width + r.x..y * width + r.x + r.w].fill(0.0);
                }
            }
        }
        for b in &bands {
            for y in 0..b.h {
                let src = &lin[b.start + y * b.w..b.start + (y + 1) * b.w];
                let dst = &mut coef[b.offset + y * width..b.offset + y * width + b.w];
                transform::dequantize(src, dst, b.q, b.z, c, exact, backend);
            }
        }
        self.lin = lin;
        clock.lap("dequantize");

        // inverse transform
        let skip_diagonal = (60..64).all(|k| qs[k] == 0.0);
        let filters = if exact {
            let (lo, hi) = nbis::decoder_filters(&transform);
            Filters::Nbis { lo, hi, bank: nbis::table_bank(&transform) }
        } else {
            Filters::Folded { bank: nbis::table_bank(&transform) }
        };
        let plan = Plan::synthesis(&p.tree, &filters, skip_diagonal, &mut self.cache);
        clock.lap("plan");
        transform::inverse_threads(&plan, &mut coef, width, backend, &mut self.ws, threads);
        clock.lap("transform");

        let (m, r) = if exact { (frame.m.nbis_f32(), frame.r.nbis_f32()) } else { (frame.m.to_f64() as f32, frame.r.to_f64() as f32) };
        let mut pixels = vec![0u8; width * height];
        transform::pixels_threads(&coef, &mut pixels, r, m, exact, backend, threads);
        self.coef = coef;
        clock.lap("output");
        Ok(Image { width: width as u32, height: height as u32, pixels, ppi: nistcom::ppi(&p.stream.comments) })
    }

    /// Parse, entropy-decode and dequantize, without the inverse transform (for offloading the
    /// transform, for instance to a GPU).
    pub fn decode_coefficients(&mut self, data: &[u8]) -> Result<Coefficients> {
        let p = self.prepare(data)?;
        let quant = self.tables.quant.clone().ok_or(Error::MissingTable("quantization table"))?;
        let exact = self.opts.mode == Mode::Exact;
        let backend = self.opts.backend.unwrap_or_else(Backend::best);
        let frame = p.stream.frame.clone().unwrap();
        let (qs, zs, c) = quant_values(&quant, exact);
        let (bands, total) = self.bands(&p, &qs, &zs);
        let mut lin = std::mem::take(&mut self.lin);
        lin.clear();
        lin.resize(total + 1, 0.0);
        let res = decode_all(&p.stream, &bands, &mut lin, total, self.opts.threads > 1);
        if let Err(e) = res {
            self.lin = lin;
            return Err(e);
        }
        let mut coef = vec![0f32; p.width * p.height];
        for b in &bands {
            for y in 0..b.h {
                let src = &lin[b.start + y * b.w..b.start + (y + 1) * b.w];
                let dst = &mut coef[b.offset + y * p.width..b.offset + y * p.width + b.w];
                transform::dequantize(src, dst, b.q, b.z, c, exact, backend);
            }
        }
        self.lin = lin;
        let (m, r) = if exact { (frame.m.nbis_f32(), frame.r.nbis_f32()) } else { (frame.m.to_f64() as f32, frame.r.to_f64() as f32) };
        Ok(Coefficients { width: p.width, height: p.height, m, r, coef })
    }

    /// The quantizer indices of every subband (empty for subbands not transmitted).
    pub fn decode_indices(&mut self, data: &[u8]) -> Result<Vec<Vec<i32>>> {
        let p = self.prepare(data)?;
        let quant = self.tables.quant.clone().ok_or(Error::MissingTable("quantization table"))?;
        let (qs, zs, _) = quant_values(&quant, false);
        let (bands, total) = self.bands(&p, &qs, &zs);
        let mut lin = vec![0f32; total + 1];
        decode_all(&p.stream, &bands, &mut lin, total, false)?;
        let mut out = vec![Vec::new(); 64];
        for b in &bands {
            out[b.k] = lin[b.start..b.start + b.w * b.h].iter().map(|&v| v as i32).collect();
        }
        Ok(out)
    }
}

fn quant_values(q: &QuantTable, exact: bool) -> ([f32; 64], [f32; 64], f32) {
    let mut qs = [0f32; 64];
    let mut zs = [0f32; 64];
    for k in 0..64 {
        if exact {
            qs[k] = q.q[k].nbis_f32();
            zs[k] = q.z[k].nbis_f32();
        } else {
            qs[k] = q.q[k].to_f64() as f32;
            zs[k] = q.z[k].to_f64() as f32;
        }
    }
    let c = if exact { q.c.nbis_f32() } else { q.c.to_f64() as f32 };
    (qs, zs, c)
}

/// Timing aid: decode the symbols of every block and discard them. Returns (symbols, seconds).
#[doc(hidden)]
pub fn time_symbols(data: &[u8]) -> Result<(usize, f64)> {
    let mut tables = Tables::default();
    let stream = stream::parse(data, &mut tables)?;
    let t0 = std::time::Instant::now();
    let mut n = 0usize;
    let mut sink = 0i64;
    for block in &stream.blocks {
        let table = WsqTable::new(&block.huffman)?;
        let mut r = BitReader::new(&block.data);
        loop {
            let e = table.entry(&mut r);
            let len = e & 31;
            let (kind, payload) = if len != 0 {
                r.consume(len);
                ((e >> 5) & 3, (e as i32) >> 8)
            } else {
                match table.slow.decode(&mut r) {
                    Some(sym) => classify(sym),
                    None => break,
                }
            };
            if r.overrun() {
                break;
            }
            if kind == 2 {
                let extra = if matches!(payload, 103 | 104 | 106) { 16 } else { 8 };
                sink += r.read(extra) as i64;
            } else {
                sink += payload as i64;
            }
            n += 1;
        }
    }
    std::hint::black_box(sink);
    Ok((n, t0.elapsed().as_secs_f64()))
}
