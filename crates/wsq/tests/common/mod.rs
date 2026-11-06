//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::PathBuf;

/// FNV-1a, 64 bits.
pub fn fnv(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The NIST WSQ reference data set (PGM version), from WSQ_NIST_DIR; None if unset.
/// A variable that is set but points elsewhere is an error, not a silent skip.
pub fn nist_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("WSQ_NIST_DIR")?);
    assert!(
        d.join("decode").is_dir(),
        "WSQ_NIST_DIR={} has no decode/ folder; point it at reference_images_v2.0_pgm",
        d.display()
    );
    Some(d)
}

pub fn skip(why: &str) {
    eprintln!("skipped: {why} (set WSQ_NIST_DIR to the NIST reference_images_v2.0_pgm folder)");
}

pub fn read_pgm(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let d = std::fs::read(path).unwrap();
    assert!(d.starts_with(b"P5"));
    let mut pos = 2;
    let mut t = Vec::new();
    while t.len() < 3 {
        while d[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if d[pos] == b'#' {
            while d[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let s = pos;
        while !d[pos].is_ascii_whitespace() {
            pos += 1;
        }
        t.push(std::str::from_utf8(&d[s..pos]).unwrap().parse::<u32>().unwrap());
    }
    pos += 1;
    let n = (t[0] * t[1]) as usize;
    (t[0], t[1], d[pos..pos + n].to_vec())
}

/// Deterministic generator.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A synthetic image: ridge-like pattern with slowly turning orientation, a background and
/// some noise, so that the coder sees fingerprint-like statistics without real fingerprints.
pub fn ridges(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut rng = Lcg(seed);
    let (cx, cy) = (w as f64 * 0.45, h as f64 * 0.55);
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let theta = dy.atan2(dx);
            let phase = r / 4.5 + 2.0 * (theta * 2.0).sin();
            let envelope = (-(r / (0.45 * w.max(h) as f64)).powi(4)).exp();
            let v = 200.0 - 120.0 * envelope * (0.5 + 0.5 * phase.cos()) + (rng.below(17) as f64 - 8.0);
            out.push(v.clamp(0.0, 255.0) as u8);
        }
    }
    out
}

pub fn gradient(w: u32, h: u32) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| ((x * 255 / w.max(1) + y * 64 / h.max(1)) % 256) as u8)).collect()
}

pub fn noise(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut rng = Lcg(seed);
    (0..w * h).map(|_| rng.below(256) as u8).collect()
}

pub fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse: f64 = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum::<f64>() / a.len() as f64;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}
