//! Encoder number two (Algorithm 12 of the algorithm document).

use crate::bits::BitWriter;
use crate::error::{Error, Result};
use crate::huffman::{encode_table, optimal_table, HuffTable};
use crate::nbis::{self, ENCODER_HI, ENCODER_LO};
use crate::nistcom;
use crate::simd::Backend;
use crate::stream::{Scaled, COM, DHT, DQT, DTT, EOI, SOB, SOF, SOI};
use crate::transform::{self, Filters, Plan, ScheduleCache, Workspace};
use crate::tree::{downsampling, Tree};
use crate::{Clock, Mode, Profile};

/// Encoder settings.
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// Target bit rate in bits per pixel; 0.75 is the usual choice for 500 ppi fingerprints.
    pub bitrate: f32,
    /// Resolution written in the NISTCOM comment.
    pub ppi: Option<u32>,
    /// `Exact` writes the same bytes as the NBIS encoder; `Fast` computes in a different order
    /// and in double precision where that is cheaper.
    pub mode: Mode,
    /// Write the NISTCOM comment NBIS writes (dimensions, resolution, bit rate).
    pub nistcom: bool,
    /// An extra comment segment.
    pub comment: Option<Vec<u8>>,
    pub backend: Option<Backend>,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions { bitrate: 0.75, ppi: None, mode: Mode::Exact, nistcom: true, comment: None, backend: None }
    }
}

/// Symbols of one block: (symbol, extra bits, number of extra bits).
type Tokens = Vec<(u8, u16, u8)>;

/// Turn a block of indices into symbols (Algorithm 6).
#[cfg(test)]
fn tokenize(indices: &[i32], out: &mut Tokens) {
    let mut run: u32 = 0;
    let flush = |n: u32, out: &mut Tokens| {
        if n <= 100 {
            out.push((n as u8, 0, 0));
        } else if n <= 0xFF {
            out.push((105, n as u16, 8));
        } else {
            out.push((106, n as u16, 16));
        }
    };
    for &v in indices {
        if v == 0 {
            run += 1;
            if run == 0xFFFF {
                flush(run, out);
                run = 0;
            }
            continue;
        }
        if run > 0 {
            flush(run, out);
            run = 0;
        }
        if (-73..=74).contains(&v) {
            out.push(((v + 180) as u8, 0, 0));
        } else if v > 0 {
            if v <= 0xFF { out.push((101, v as u16, 8)) } else { out.push((103, v as u16, 16)) }
        } else if -v <= 0xFF {
            out.push((102, (-v) as u16, 8));
        } else {
            out.push((104, (-v) as u16, 16));
        }
    }
    if run > 0 {
        flush(run, out);
    }
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_scaled16(out: &mut Vec<u8>, s: Scaled) {
    out.push(s.exp);
    put_u16(out, s.value as u16);
}

fn put_huffman(out: &mut Vec<u8>, id: u8, t: &HuffTable) {
    put_u16(out, DHT);
    put_u16(out, (3 + 16 + t.values.len()) as u16);
    out.push(id);
    out.extend_from_slice(&t.bits);
    out.extend_from_slice(&t.values);
}

fn put_block(out: &mut Vec<u8>, id: u8, tokens: &Tokens, t: &HuffTable) -> Result<()> {
    put_u16(out, SOB);
    put_u16(out, 3);
    out.push(id);
    let codes = encode_table(t)?;
    let mut w = BitWriter::with_capacity(tokens.len());
    for &(s, extra, n) in tokens {
        let (code, len) = codes[s as usize];
        // code (at most 16 bits) and extra bits (at most 16) in one write
        w.put(((code as u32) << n) | extra as u32, len as u32 + n as u32);
    }
    out.extend_from_slice(&w.finish());
    Ok(())
}

/// Sum and sum of squares of a window, in double precision with independent accumulators.
fn window_sums(coef: &[f32], width: usize, x: usize, y: usize, w: usize, h: usize) -> (f64, f64) {
    let (mut s, mut q) = ([0f64; 8], [0f64; 8]);
    for row in y..y + h {
        let line = &coef[row * width + x..row * width + x + w];
        let mut chunks = line.chunks_exact(8);
        for c in &mut chunks {
            for i in 0..8 {
                let v = c[i] as f64;
                s[i] += v;
                q[i] += v * v;
            }
        }
        for (i, &v) in chunks.remainder().iter().enumerate() {
            s[i] += v as f64;
            q[i] += v as f64 * v as f64;
        }
    }
    (s.iter().sum(), q.iter().sum())
}

/// Subband variances (version 3.1 window rule) in double precision.
fn variances_f64(coef: &[f32], width: usize, tree: &Tree) -> [f32; 60] {
    let var = |x: usize, y: usize, w: usize, h: usize| -> f32 {
        let n = (w * h) as f64;
        if n < 2.0 {
            return 0.0;
        }
        let (s, q) = window_sums(coef, width, x, y, w, h);
        ((q - s * s / n) / (n - 1.0)) as f32
    };
    let window = |k: usize| {
        let r = tree.subbands[k];
        let (w, h) = ((3 * r.w) / 4, (7 * r.h) / 16);
        if w * h >= 2 { var(r.x + r.w / 8, r.y + (9 * r.h) / 32, w, h) } else { var(r.x, r.y, r.w, r.h) }
    };
    let mut out = [0f32; 60];
    let sum4: f64 = (0..4).map(|k| window(k) as f64).sum();
    for (k, o) in out.iter_mut().enumerate() {
        let r = tree.subbands[k];
        *o = if sum4 > 20000.0 { window(k) } else { var(r.x, r.y, r.w, r.h) };
    }
    out
}

/// Bin widths of encoder number two computed in double precision.
fn bin_widths_f64(var: &[f32; 60], r: f64) -> ([f32; 64], [f32; 64]) {
    const A: [f64; 8] = [1.32, 1.08, 1.42, 1.08, 1.32, 1.42, 1.08, 1.08];
    let mut qp = [0f64; 60];
    let mut k0 = Vec::new();
    for k in 0..60 {
        let v = var[k] as f64;
        if v >= 1.01 {
            k0.push(k);
            qp[k] = if k < 4 { 1.0 } else { 10.0 / (if k >= 52 { A[k - 52] } else { 1.0 } * v.ln()) };
        }
    }
    let mut q = [0f32; 64];
    let mut z = [0f32; 64];
    if k0.is_empty() {
        return (q, z);
    }
    let sigma = |k: usize| (var[k] as f64).sqrt();
    let m = |k: usize| downsampling(k) as f64;
    let mut kset = k0.clone();
    let qq = loop {
        let s: f64 = kset.iter().map(|&k| 1.0 / m(k)).sum();
        let logp: f64 = kset.iter().map(|&k| (sigma(k) / qp[k]).ln() / m(k)).sum();
        let qq = 2f64.powf(r / s - 1.0) / 2.5 * (-logp / s).exp();
        let before = kset.len();
        kset.retain(|&k| qp[k] / qq < 5.0 * sigma(k));
        if kset.len() == before || kset.is_empty() {
            break qq;
        }
    };
    for &k in &k0 {
        q[k] = (qp[k] / qq) as f32;
        z[k] = (1.2 * qp[k] / qq) as f32;
    }
    (q, z)
}

/// Encode an 8-bit grey image.
pub fn encode(pixels: &[u8], width: u32, height: u32, opts: &EncodeOptions) -> Result<Vec<u8>> {
    Encoder::new().encode(pixels, width, height, opts)
}

/// Encode and report the time of each stage.
pub fn encode_profiled(pixels: &[u8], width: u32, height: u32, opts: &EncodeOptions) -> Result<(Vec<u8>, Profile)> {
    Encoder::new().encode_profiled(pixels, width, height, opts)
}

/// An encoder that reuses its buffers and transform schedules between images.
#[derive(Default)]
pub struct Encoder {
    cache: ScheduleCache,
    ws: Workspace,
    coef: Vec<f32>,
}

impl Encoder {
    pub fn new() -> Self {
        Encoder::default()
    }

    pub fn encode(&mut self, pixels: &[u8], width: u32, height: u32, opts: &EncodeOptions) -> Result<Vec<u8>> {
        encode_inner(pixels, width, height, opts, &mut Clock::new(false), &mut self.cache, &mut self.ws, &mut self.coef)
    }

    pub fn encode_profiled(&mut self, pixels: &[u8], width: u32, height: u32, opts: &EncodeOptions) -> Result<(Vec<u8>, Profile)> {
        let mut clock = Clock::new(true);
        let out = encode_inner(pixels, width, height, opts, &mut clock, &mut self.cache, &mut self.ws, &mut self.coef)?;
        Ok((out, clock.finish()))
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_inner(pixels: &[u8], width: u32, height: u32, opts: &EncodeOptions, clock: &mut Clock, cache: &mut ScheduleCache, ws: &mut Workspace, coef_buf: &mut Vec<f32>) -> Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || w > 0xFFFF || h > 0xFFFF || pixels.len() != w * h {
        return Err(Error::BadDimensions);
    }
    if !(opts.bitrate > 0.0 && opts.bitrate.is_finite()) {
        return Err(Error::BadParameter("bit rate must be positive"));
    }
    let backend = opts.backend.unwrap_or_else(Backend::best);
    if !backend.is_available() {
        return Err(Error::BadParameter("backend not available on this machine"));
    }
    let exact = opts.mode == Mode::Exact;
    let tree = Tree::new(w, h);

    // normalization
    let mut coef = std::mem::take(coef_buf);
    coef.resize(w * h, 0.0);
    let (m_shift, r_scale) = if exact {
        let (m, r) = nbis::normalize_params(pixels);
        transform::normalize(pixels, &mut coef, m, r, backend);
        (m, r)
    } else {
        let sum: u64 = pixels.iter().map(|&p| p as u64).sum();
        let mean = sum as f64 / pixels.len() as f64;
        let (lo, hi) = pixels.iter().fold((255u8, 0u8), |(l, h), &p| (l.min(p), h.max(p)));
        let mut r = (mean - lo as f64).max(hi as f64 - mean) / 128.0;
        if r == 0.0 {
            r = 1.0;
        }
        let (mf, inv) = (mean as f32, (1.0 / r) as f32);
        for (o, &p) in coef.iter_mut().zip(pixels) {
            *o = (p as f32 - mf) * inv;
        }
        (mean as f32, r as f32)
    };

    clock.lap("normalize");

    // transform
    let filters = if exact {
        Filters::Nbis { lo: ENCODER_LO.to_vec(), hi: ENCODER_HI.to_vec(), bank: nbis::standard_bank() }
    } else {
        Filters::Folded { bank: nbis::standard_bank() }
    };
    let plan = Plan::analysis(&tree, &filters, true, cache);
    clock.lap("plan");
    transform::forward(&plan, &mut coef, w, backend, ws);
    clock.lap("transform");

    // quantization
    let var = if exact { nbis::variances(&coef, w, &tree) } else { variances_f64(&coef, w, &tree) };
    clock.lap("variance");
    let (mut q, mut z) = if exact { nbis::bin_widths(&var, opts.bitrate) } else { bin_widths_f64(&var, opts.bitrate as f64) };
    let mut indices: [Vec<f32>; 3] = Default::default();
    for (b, range) in [0..19usize, 19..52, 52..60].into_iter().enumerate() {
        indices[b].reserve(range.clone().filter(|&k| q[k] != 0.0).map(|k| tree.subbands[k].len()).sum());
        for k in range {
            if q[k] == 0.0 {
                continue;
            }
            let r = tree.subbands[k];
            let base = indices[b].len();
            loop {
                indices[b].resize(base + r.len(), 0.0);
                let zbin = (z[k] as f64 / 2.0) as f32;
                let mut peak = 0f32;
                for y in 0..r.h {
                    let src = &coef[(r.y + y) * w + r.x..(r.y + y) * w + r.x + r.w];
                    peak = peak.max(transform::quantize(src, &mut indices[b][base + y * r.w..base + (y + 1) * r.w], q[k], zbin, backend));
                }
                // Indices must fit the 16-bit escapes, and bin widths the 16-bit table fields.
                if peak <= 65535.0 && q[k] < 65535.0 && z[k] < 65535.0 {
                    break;
                }
                indices[b].truncate(base);
                if peak == 0.0 {
                    q[k] = 0.0; // nothing survives: do not transmit the subband
                    z[k] = 0.0;
                    break;
                }
                let grow = (peak / 65520.0).max(1.0001);
                q[k] = (q[k] * grow).min(65000.0);
                z[k] = (z[k] * grow).min(65000.0);
            }
        }
    }

    clock.lap("quantize");

    // entropy coding
    let mut tokens: [Tokens; 3] = Default::default();
    for b in 0..3 {
        tokens[b].reserve(indices[b].len() / 4);
        transform::tokens(&indices[b], &mut tokens[b], backend);
    }
    clock.lap("tokenize");
    let mut freq = [[0u32; 256]; 2];
    for (b, t) in tokens.iter().enumerate() {
        for &(s, _, _) in t {
            freq[(b > 0) as usize][s as usize] += 1;
        }
    }
    let tables = [optimal_table(&freq[0]), optimal_table(&freq[1])];
    clock.lap("tables");

    // file
    let mut out = Vec::with_capacity(w * h / 8 + 2048);
    put_u16(&mut out, SOI);
    let mut comments = Vec::new();
    if opts.nistcom {
        comments.push(nistcom::nbis_comment(w, h, opts.ppi, opts.bitrate).into_bytes());
    }
    if let Some(c) = &opts.comment {
        comments.push(c.clone());
    }
    for c in &comments {
        if c.len() > 0xFFFF - 2 {
            return Err(Error::BadParameter("comment longer than 65533 bytes"));
        }
        put_u16(&mut out, COM);
        put_u16(&mut out, (c.len() + 2) as u16);
        out.extend_from_slice(c);
    }
    put_u16(&mut out, DTT);
    put_u16(&mut out, 58);
    out.push(9);
    out.push(7);
    for &v in ENCODER_LO[4..].iter().chain(ENCODER_HI[3..].iter()) {
        let s = Scaled::nbis_from_f32_u32(v.abs());
        out.push((v < 0.0) as u8);
        out.push(s.exp);
        out.extend_from_slice(&s.value.to_be_bytes());
    }
    put_u16(&mut out, DQT);
    put_u16(&mut out, 389);
    out.push(2);
    put_u16(&mut out, 44);
    for k in 0..64 {
        let (sq, sz) = if q[k] != 0.0 { (Scaled::nbis_from_f32(q[k]), Scaled::nbis_from_f32(z[k])) } else { Default::default() };
        put_scaled16(&mut out, sq);
        put_scaled16(&mut out, sz);
    }
    put_u16(&mut out, SOF);
    put_u16(&mut out, 17);
    out.push(0);
    out.push(255);
    put_u16(&mut out, h as u16);
    put_u16(&mut out, w as u16);
    put_scaled16(&mut out, Scaled::nbis_from_f32(m_shift));
    put_scaled16(&mut out, Scaled::nbis_from_f32(r_scale));
    out.push(2);
    put_u16(&mut out, 0);
    put_huffman(&mut out, 0, &tables[0]);
    put_block(&mut out, 0, &tokens[0], &tables[0])?;
    put_huffman(&mut out, 1, &tables[1]);
    put_block(&mut out, 1, &tokens[1], &tables[1])?;
    put_block(&mut out, 1, &tokens[2], &tables[1])?;
    put_u16(&mut out, EOI);
    *coef_buf = coef;
    clock.lap("entropy");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_follows_the_nist_rules() {
        let mut t = Tokens::new();
        let mut v = vec![0; 300];
        v.extend([1, -1, 74, 75, -73, -74, 300, -300, 0, 5]);
        tokenize(&v, &mut t);
        assert_eq!(
            t,
            vec![(106, 300, 16), (181, 0, 0), (179, 0, 0), (254, 0, 0), (101, 75, 8), (107, 0, 0), (102, 74, 8), (103, 300, 16), (104, 300, 16), (1, 0, 0), (185, 0, 0)]
        );
    }
}
