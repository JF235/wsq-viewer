//! The compliance measures of Annex AA, for comparing an implementation with a reference.

use crate::error::{Error, Result};
use crate::stream::{self, Tables};
use crate::{DecodeOptions, Decoder};

/// Encoder measures (Annex AA.2) of a test stream against a reference stream of the same image.
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderMeasures {
    /// Size difference without comments, in percent of the reference size.
    pub size_diff_percent: f64,
    /// Largest relative difference of a bin width or zero-bin width, in percent.
    pub bin_width_diff_percent: f64,
    /// Share of quantizer indices equal to the reference, in percent.
    pub equal_indices_percent: f64,
    pub max_index_diff: i64,
}

impl EncoderMeasures {
    pub fn passes(&self) -> bool {
        self.size_diff_percent.abs() <= 0.4
            && self.bin_width_diff_percent <= 0.051
            && self.equal_indices_percent >= 99.99
            && self.max_index_diff <= 1
    }
}

/// Decoder measures (Annex AA.3) of a reconstruction against a reference reconstruction.
#[derive(Debug, Clone, PartialEq)]
pub struct DecoderMeasures {
    pub equal_pixels_percent: f64,
    pub max_pixel_diff: u8,
}

impl DecoderMeasures {
    pub fn passes(&self) -> bool {
        self.equal_pixels_percent >= 99.9 && self.max_pixel_diff <= 1
    }
}

fn size_without_comments(data: &[u8]) -> Result<usize> {
    let mut t = Tables::default();
    let s = stream::parse(data, &mut t)?;
    Ok(data.len() - s.comments.iter().map(|c| c.len() + 4).sum::<usize>())
}

pub fn encoder_measures(test: &[u8], reference: &[u8]) -> Result<EncoderMeasures> {
    let (st, sr) = (size_without_comments(test)?, size_without_comments(reference)?);
    let mut tt = Tables::default();
    let mut tr = Tables::default();
    stream::parse(test, &mut tt)?;
    stream::parse(reference, &mut tr)?;
    let (qt, qr) = (tt.quant.ok_or(Error::MissingTable("quantization table"))?, tr.quant.ok_or(Error::MissingTable("quantization table"))?);
    let mut bin = 0f64;
    for k in 0..64 {
        for (a, b) in [(qt.q[k], qr.q[k]), (qt.z[k], qr.z[k])] {
            let (a, b) = (a.to_f64(), b.to_f64());
            if b != 0.0 {
                bin = bin.max(((a - b) / b).abs() * 100.0);
            } else if a != 0.0 {
                bin = f64::INFINITY;
            }
        }
    }
    let it = Decoder::new(DecodeOptions::default()).decode_indices(test)?;
    let ir = Decoder::new(DecodeOptions::default()).decode_indices(reference)?;
    let (mut n, mut eq, mut maxd) = (0usize, 0usize, 0i64);
    for (a, b) in it.iter().zip(&ir) {
        if a.len() != b.len() {
            // a subband present in one stream only: compare against zeros
            let m = a.len().max(b.len());
            for i in 0..m {
                let (x, y) = (*a.get(i).unwrap_or(&0) as i64, *b.get(i).unwrap_or(&0) as i64);
                n += 1;
                eq += (x == y) as usize;
                maxd = maxd.max((x - y).abs());
            }
            continue;
        }
        for (&x, &y) in a.iter().zip(b) {
            n += 1;
            eq += (x == y) as usize;
            maxd = maxd.max((x as i64 - y as i64).abs());
        }
    }
    Ok(EncoderMeasures {
        size_diff_percent: (st as f64 - sr as f64) / sr as f64 * 100.0,
        bin_width_diff_percent: bin,
        equal_indices_percent: if n == 0 { 100.0 } else { eq as f64 / n as f64 * 100.0 },
        max_index_diff: maxd,
    })
}

pub fn decoder_measures(test: &[u8], reference: &[u8]) -> DecoderMeasures {
    assert_eq!(test.len(), reference.len());
    let mut eq = 0usize;
    let mut maxd = 0u8;
    for (&a, &b) in test.iter().zip(reference) {
        eq += (a == b) as usize;
        maxd = maxd.max(a.abs_diff(b));
    }
    DecoderMeasures { equal_pixels_percent: eq as f64 / test.len().max(1) as f64 * 100.0, max_pixel_diff: maxd }
}
