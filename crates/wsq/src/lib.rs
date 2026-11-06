//! WSQ (Wavelet Scalar Quantization) fingerprint image codec: FBI specification IAFIS-IC-0110,
//! version 3.1, encoder number two.
//!
//! In [`Mode::Exact`] the decoder reproduces the NIST reference implementation (NBIS 5.0.0)
//! pixel for pixel and the encoder writes the same bytes; [`Mode::Fast`] allows a different
//! arithmetic order. The transform runs on the widest instruction set available
//! ([`Backend`]); every backend gives identical results in exact mode.
//!
//! ```no_run
//! let data = std::fs::read("finger.wsq").unwrap();
//! let image = wsq::decode(&data).unwrap();
//! let again = wsq::encode(&image.pixels, image.width, image.height, &wsq::EncodeOptions::default()).unwrap();
//! ```

mod bits;
pub mod compliance;
mod decode;
mod encode;
mod error;
mod huffman;
mod kernels;
pub mod nbis;
mod nistcom;
pub mod schedule;
pub mod simd;
mod stream;
#[doc(hidden)]
pub mod transform;
pub mod tree;

pub use decode::{decode, decode_with, time_symbols, Coefficients, DecodeOptions, Decoder};
pub use encode::{encode, encode_profiled, EncodeOptions, Encoder};
pub use error::{Error, Result};
pub use simd::Backend;

/// Time spent in each stage of a decode or encode, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub stages: Vec<(&'static str, f64)>,
}

pub(crate) struct Clock {
    /// Present only when profiling: wasm32-unknown-unknown has no clock.
    last: Option<std::time::Instant>,
    profile: Option<Profile>,
}

impl Clock {
    pub(crate) fn new(enabled: bool) -> Clock {
        Clock { last: enabled.then(std::time::Instant::now), profile: enabled.then(Profile::default) }
    }
    #[inline]
    pub(crate) fn lap(&mut self, stage: &'static str) {
        if let (Some(p), Some(last)) = (&mut self.profile, &mut self.last) {
            let now = std::time::Instant::now();
            p.stages.push((stage, (now - *last).as_secs_f64() * 1e3));
            *last = now;
        }
    }
    pub(crate) fn finish(self) -> Profile {
        self.profile.unwrap_or_default()
    }
}

/// Arithmetic contract of the codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Same results as NBIS 5.0.0, bit for bit.
    Exact,
    /// Any operation order; results stay within the compliance tolerances.
    Fast,
}

/// An 8-bit grey image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Row-major pixels, `width * height` bytes.
    pub pixels: Vec<u8>,
    /// Resolution from the NISTCOM comment, if any.
    pub ppi: Option<u32>,
}

/// What a stream says about itself, without decoding the image.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub ppi: Option<u32>,
    /// Encoder number from the frame header (2 for encoder number two).
    pub encoder: u8,
    pub software: u16,
    /// Lengths of the analysis filters.
    pub filters: (u8, u8),
    pub blocks: usize,
    pub comments: Vec<Vec<u8>>,
    /// Bit rate from the NISTCOM comment, if any.
    pub bitrate: Option<f32>,
}

/// Read the header, tables and comments of a stream.
pub fn read_info(data: &[u8]) -> Result<Info> {
    let mut tables = stream::Tables::default();
    let s = stream::parse(data, &mut tables)?;
    let f = s.frame.as_ref().ok_or(Error::BadSegment("no frame header"))?;
    let t = tables.transform.as_ref();
    Ok(Info {
        width: f.width as u32,
        height: f.height as u32,
        ppi: nistcom::ppi(&s.comments),
        encoder: f.encoder,
        software: f.software,
        filters: t.map(|t| (t.l0, t.l1)).unwrap_or((0, 0)),
        blocks: s.blocks.len(),
        bitrate: s.comments.iter().find_map(|c| nistcom::lookup(c, "WSQ_BITRATE")?.parse().ok()),
        comments: s.comments,
    })
}
