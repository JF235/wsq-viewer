//! Arithmetic that reproduces NBIS 5.0.0 bit for bit: the same float and double operations, in
//! the same order, as the C code. Each function names the NBIS routine it mirrors.

use crate::schedule::Bank;
use crate::stream::TransformTable;
use crate::tree::{downsampling, Tree};

/// Encoder analysis lowpass filter (`lofilt` in NBIS `globals.c`), as float. The literals are
/// those of NBIS, digit for digit.
#[allow(clippy::excessive_precision)]
pub const ENCODER_LO: [f32; 9] = [
    0.03782845550699546,
    -0.02384946501938000,
    -0.11062440441842342,
    0.37740285561265380,
    0.85269867900940344,
    0.37740285561265380,
    -0.11062440441842342,
    -0.02384946501938000,
    0.03782845550699546,
];

/// Encoder analysis highpass filter (`hifilt` in NBIS `globals.c`).
#[allow(clippy::excessive_precision)]
pub const ENCODER_HI: [f32; 7] = [
    0.06453888262893845,
    -0.04068941760955844,
    -0.41809227322221221,
    0.78848561640566439,
    -0.41809227322221221,
    -0.04068941760955844,
    0.06453888262893845,
];

/// The standard bank as the specification gives it (right halves), in double precision.
#[allow(clippy::excessive_precision)]
pub fn standard_bank() -> Bank {
    let h0: Vec<f64> = ENCODER_LO[4..].iter().map(|&v| v as f64).collect();
    let h1: Vec<f64> = ENCODER_HI[3..].iter().map(|&v| v as f64).collect();
    // use the full-precision values rather than the float literals
    let h0x = [0.852698679009403, 0.377402855612654, -0.110624404418423, -0.023849465019380, 0.037828455506995];
    let h1x = [0.788485616405664, -0.418092273222212, -0.040689417609558, 0.064538882628938];
    debug_assert!(h0.iter().zip(h0x).all(|(a, b)| (a - b).abs() < 1e-7));
    debug_assert!(h1.iter().zip(h1x).all(|(a, b)| (a - b).abs() < 1e-7));
    Bank::from_halves(9, &h0x, &h1x)
}

fn int_sign(power: usize) -> f32 {
    if power.is_multiple_of(2) { 1.0 } else { -1.0 }
}

/// Synthesis filters as NBIS builds them from a transform table (`getc_transform_table`):
/// returns (lo, hi) where `hi` has L0 taps (from h0) and `lo` has L1 taps (from h1).
pub fn decoder_filters(t: &TransformTable) -> (Vec<f32>, Vec<f32>) {
    let hisz = t.l0 as usize;
    let losz = t.l1 as usize;
    let mut hi = vec![0f32; hisz];
    let mut lo = vec![0f32; losz];
    let a_lo: Vec<f32> = t.h0.iter().map(|tap| tap.nbis_f32()).collect();
    let a_hi: Vec<f32> = t.h1.iter().map(|tap| tap.nbis_f32()).collect();
    if hisz % 2 == 1 {
        let a = hisz.div_ceil(2) - 1;
        for (cnt, &v) in a_lo.iter().enumerate().take(a + 1) {
            hi[cnt + a] = int_sign(cnt) * v;
            if cnt > 0 {
                hi[a - cnt] = hi[cnt + a];
            }
        }
    } else {
        let a = hisz / 2 - 1;
        for (cnt, &v) in a_lo.iter().enumerate().take(a + 1) {
            hi[cnt + a + 1] = int_sign(cnt) * v;
            hi[a - cnt] = -hi[cnt + a + 1];
        }
    }
    if losz % 2 == 1 {
        let a = losz.div_ceil(2) - 1;
        for (cnt, &v) in a_hi.iter().enumerate().take(a + 1) {
            lo[cnt + a] = int_sign(cnt) * v;
            if cnt > 0 {
                lo[a - cnt] = lo[cnt + a];
            }
        }
    } else {
        let a = losz / 2 - 1;
        for (cnt, &v) in a_hi.iter().enumerate().take(a + 1) {
            lo[cnt + a + 1] = int_sign(cnt + 1) * v;
            lo[a - cnt] = lo[cnt + a + 1];
        }
    }
    (lo, hi)
}

/// The bank of a transform table, in double precision.
pub fn table_bank(t: &TransformTable) -> Bank {
    let h0: Vec<f64> = t.h0.iter().map(|tap| tap.to_f64()).collect();
    let h1: Vec<f64> = t.h1.iter().map(|tap| tap.to_f64()).collect();
    Bank::from_halves(t.l0 as usize, &h0, &h1)
}

/// `conv_img_2_flt_ret`, first part: image mean and scale.
/// A constant image (scale zero) gets scale 1 instead of NBIS's division by zero.
pub fn normalize_params(pixels: &[u8]) -> (f32, f32) {
    let sum: u64 = pixels.iter().map(|&p| p as u64).sum();
    let (low, high) = pixels.iter().fold((255u8, 0u8), |(l, h), &p| (l.min(p), h.max(p)));
    // NBIS sums in a 32-bit unsigned integer (and stops on overflow): the same value here
    let m_shift = (sum as u32) as f32 / pixels.len() as f32;
    let m_shift = if sum > u32::MAX as u64 { sum as f32 / pixels.len() as f32 } else { m_shift };
    let low_diff = m_shift - low as f32;
    let high_diff = high as f32 - m_shift;
    let mut r_scale = if low_diff >= high_diff { low_diff } else { high_diff };
    r_scale = (r_scale as f64 / 128.0) as f32;
    if r_scale == 0.0 {
        r_scale = 1.0;
    }
    (m_shift, r_scale)
}

/// `conv_img_2_flt_ret`: image mean and scale, then the normalized float image.
/// A constant image (scale zero) gets scale 1 instead of NBIS's division by zero.
pub fn normalize(pixels: &[u8], out: &mut [f32]) -> (f32, f32) {
    let mut sum: u32 = 0;
    let (mut low, mut high) = (255u8, 0u8);
    for &p in pixels {
        sum = sum.wrapping_add(p as u32);
        low = low.min(p);
        high = high.max(p);
    }
    let sum = if pixels.len() > (u32::MAX / 255) as usize {
        // NBIS stops with an overflow error here; use the exact sum
        pixels.iter().map(|&p| p as u64).sum::<u64>() as f32
    } else {
        sum as f32
    };
    let m_shift = sum / pixels.len() as f32;
    let low_diff = m_shift - low as f32;
    let high_diff = high as f32 - m_shift;
    let mut r_scale = if low_diff >= high_diff { low_diff } else { high_diff };
    r_scale = (r_scale as f64 / 128.0) as f32;
    if r_scale == 0.0 {
        r_scale = 1.0;
    }
    for (o, &p) in out.iter_mut().zip(pixels) {
        *o = (p as f32 - m_shift) / r_scale;
    }
    (m_shift, r_scale)
}

fn window_variance(coef: &[f32], width: usize, x: usize, y: usize, w: usize, h: usize) -> f32 {
    let mut sum_pix = 0f32;
    let mut ssq = 0f32;
    for row in 0..h {
        let line = &coef[(y + row) * width + x..(y + row) * width + x + w];
        for &v in line {
            sum_pix += v;
            ssq += v * v;
        }
    }
    let n = (w * h) as f32;
    let sum2 = (sum_pix * sum_pix) / n;
    ((ssq - sum2) as f64 / ((w * h) as f64 - 1.0)) as f32
}

/// `variance`: subband variances of encoder number two (version 3.1 window rule).
///
/// Where a window or subband holds fewer than two coefficients, NBIS divides by zero; here the
/// variance is taken over the whole subband, or set to zero for a single coefficient.
pub fn variances(coef: &[f32], width: usize, tree: &Tree) -> [f32; 60] {
    let mut var = [0f32; 60];
    let sub = |k: usize| -> f32 {
        let r = tree.subbands[k];
        let (w, h) = ((3 * r.w) / 4, (7 * r.h) / 16);
        if w * h >= 2 {
            window_variance(coef, width, r.x + r.w / 8, r.y + (9 * r.h) / 32, w, h)
        } else {
            full(coef, width, &r)
        }
    };
    fn full(coef: &[f32], width: usize, r: &crate::tree::Region) -> f32 {
        if r.w * r.h >= 2 { window_variance(coef, width, r.x, r.y, r.w, r.h) } else { 0.0 }
    }
    let mut vsum = 0f32;
    for (k, v) in var.iter_mut().enumerate().take(4) {
        *v = sub(k);
        vsum += *v;
    }
    if vsum < 20000.0 {
        for (k, v) in var.iter_mut().enumerate() {
            *v = full(coef, width, &tree.subbands[k]);
        }
    } else {
        for (k, v) in var.iter_mut().enumerate().skip(4) {
            *v = sub(k);
        }
    }
    var
}

const A_WEIGHTS: [f32; 8] = [1.32, 1.08, 1.42, 1.08, 1.32, 1.42, 1.08, 1.08];

/// `quantize` (first part): bin widths Q_k and zero-bin widths Z_k for target rate `r`.
pub fn bin_widths(var: &[f32; 60], r: f32) -> ([f32; 64], [f32; 64]) {
    let a = |k: usize| if k >= 52 { A_WEIGHTS[k - 52] } else { 1.0f32 };
    let mut qbss = [0f32; 64];
    let mut qzbs = [0f32; 64];
    for k in 0..60 {
        if (var[k] as f64) < 1.01 {
            qbss[k] = 0.0;
        } else if k < 4 {
            qbss[k] = 1.0;
        } else {
            qbss[k] = (10.0f64 / (a(k) * (var[k] as f64).ln() as f32) as f64) as f32;
        }
    }
    let m = |k: usize| 1.0f32 / downsampling(k) as f32;
    let mut sigma = [0f32; 60];
    let mut k0 = Vec::new();
    for k in 0..60 {
        if (var[k] as f64) >= 1.01 {
            k0.push(k);
            sigma[k] = (var[k] as f64).sqrt() as f32;
        }
    }
    if k0.is_empty() {
        return (qbss, qzbs);
    }
    let mut kset = k0.clone();
    let q: f32 = loop {
        let mut s = 0f32;
        for &k in &kset {
            s += m(k);
        }
        let mut p = 1f32;
        for &k in &kset {
            p = (p as f64 * ((sigma[k] / qbss[k]) as f64).powf(m(k) as f64)) as f32;
        }
        let q = ((2f64.powf((r / s) as f64 - 1.0) / 2.5) / (p as f64).powf(1.0 / s as f64)) as f32;
        let np: Vec<usize> = kset.iter().copied().filter(|&k| ((qbss[k] / q) as f64) >= 5.0 * sigma[k] as f64).collect();
        if np.is_empty() {
            break q;
        }
        kset.retain(|k| !np.contains(k));
        if kset.is_empty() {
            break q;
        }
    };
    for k in 0..60 {
        qbss[k] = if k0.contains(&k) { qbss[k] / q } else { 0.0 };
        qzbs[k] = (1.2f64 * qbss[k] as f64) as f32;
    }
    (qbss, qzbs)
}

/// `quantize` (second part): the index of one coefficient.
#[inline(always)]
pub fn quantize(v: f32, q: f32, zbin: f32) -> i32 {
    if -zbin <= v && v <= zbin {
        0
    } else if v > 0.0 {
        (((v - zbin) / q) as f64 + 1.0) as i32
    } else {
        (((v + zbin) / q) as f64 - 1.0) as i32
    }
}

/// `unquantize`: the reconstruction of a nonzero index.
#[inline(always)]
pub fn dequantize(p: i32, q: f32, z: f32, c: f32) -> f32 {
    if p > 0 {
        ((q * (p as f32 - c)) as f64 + z as f64 / 2.0) as f32
    } else {
        ((q * (p as f32 + c)) as f64 - z as f64 / 2.0) as f32
    }
}

/// `conv_img_2_uchar`: one output pixel.
#[inline(always)]
pub fn to_pixel(v: f32, r_scale: f32, m_shift: f32) -> u8 {
    let t = v * r_scale + m_shift;
    let t = (t as f64 + 0.5) as f32;
    if t < 0.0 {
        0
    } else if t > 255.0 {
        255
    } else {
        t as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizer_matches_the_formula() {
        let (q, z) = (2.0f32, 2.4f32);
        let zb = z / 2.0;
        assert_eq!(quantize(1.2, q, zb), 0);
        assert_eq!(quantize(-1.2, q, zb), 0);
        assert_eq!(quantize(1.3, q, zb), 1);
        assert_eq!(quantize(3.2, q, zb), 2);
        assert_eq!(quantize(-3.3, q, zb), -2);
        assert!((dequantize(1, q, z, 0.44) - (1.2 + 0.56 * 2.0)).abs() < 1e-6);
    }
}
