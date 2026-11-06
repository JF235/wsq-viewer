//! Binary PGM (P5, 8-bit) reading and writing.

use std::io;
use std::path::Path;

pub fn read(path: &Path) -> io::Result<(u32, u32, Vec<u8>)> {
    let d = std::fs::read(path)?;
    parse(&d).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an 8-bit binary PGM"))
}

pub fn parse(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if !d.starts_with(b"P5") {
        return None;
    }
    let mut pos = 2;
    let mut tokens = Vec::new();
    while tokens.len() < 3 {
        while d.get(pos)?.is_ascii_whitespace() {
            pos += 1;
        }
        if d[pos] == b'#' {
            while d.get(pos)? != &b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while !d.get(pos)?.is_ascii_whitespace() {
            pos += 1;
        }
        tokens.push(std::str::from_utf8(&d[start..pos]).ok()?.parse::<u32>().ok()?);
    }
    let (w, h, maxval) = (tokens[0], tokens[1], tokens[2]);
    if maxval > 255 {
        return None;
    }
    pos += 1;
    let n = (w as usize).checked_mul(h as usize)?;
    Some((w, h, d.get(pos..pos + n)?.to_vec()))
}

pub fn write(path: &Path, w: u32, h: u32, pixels: &[u8]) -> io::Result<()> {
    let mut out = format!("P5\n{w} {h}\n255\n").into_bytes();
    out.extend_from_slice(pixels);
    std::fs::write(path, out)
}
