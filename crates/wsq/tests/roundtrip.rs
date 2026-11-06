//! Encoding and decoding synthetic images: sizes, contents, modes, backends and threads.

mod common;

use common::{gradient, noise, psnr, ridges};
use wsq::{Backend, DecodeOptions, Decoder, EncodeOptions, Encoder, Mode};

const SIZES: [(u32, u32); 12] = [(1, 1), (2, 3), (7, 5), (16, 16), (31, 47), (49, 81), (81, 81), (128, 96), (333, 257), (400, 400), (589, 605), (800, 750)];

fn encode(px: &[u8], w: u32, h: u32, rate: f32, mode: Mode, backend: Backend) -> Vec<u8> {
    let opts = EncodeOptions { bitrate: rate, ppi: Some(500), mode, backend: Some(backend), ..Default::default() };
    Encoder::new().encode(px, w, h, &opts).unwrap_or_else(|e| panic!("{w}x{h}: {e}"))
}

fn decode(data: &[u8], mode: Mode, backend: Backend, threads: usize) -> wsq::Image {
    let opts = DecodeOptions { mode, backend: Some(backend), threads, ..Default::default() };
    Decoder::new(opts).decode(data).unwrap()
}

#[test]
fn every_size_round_trips() {
    for &(w, h) in &SIZES {
        for (label, px) in [("ridges", ridges(w, h, 1)), ("gradient", gradient(w, h)), ("noise", noise(w, h, 2))] {
            for mode in [Mode::Exact, Mode::Fast] {
                let data = encode(&px, w, h, 0.75, mode, Backend::best());
                let img = decode(&data, mode, Backend::best(), 1);
                assert_eq!((img.width, img.height, img.ppi), (w, h, Some(500)), "{label} {w}x{h}");
                assert_eq!(img.pixels.len(), px.len());
            }
        }
    }
}

#[test]
fn quality_grows_with_the_rate() {
    let (w, h) = (500, 500);
    let px = ridges(w, h, 3);
    let mut last = 0.0;
    for rate in [0.3, 0.75, 1.5, 2.25, 4.0] {
        let data = encode(&px, w, h, rate, Mode::Exact, Backend::best());
        let p = psnr(&decode(&data, Mode::Exact, Backend::best(), 1).pixels, &px);
        assert!(p > last, "PSNR {p:.2} at {rate} not above {last:.2}");
        last = p;
    }
    assert!(last > 30.0, "PSNR {last:.2} at 4 bpp");
}

#[test]
fn constant_images_decode_to_themselves() {
    for v in [0u8, 1, 128, 254, 255] {
        let px = vec![v; 100 * 90];
        let data = encode(&px, 100, 90, 0.75, Mode::Exact, Backend::best());
        let img = decode(&data, Mode::Exact, Backend::best(), 1);
        assert!(img.pixels.iter().all(|&p| p.abs_diff(v) <= 1), "constant {v}");
    }
}

#[test]
fn backends_and_threads_agree_bit_for_bit_in_exact_mode() {
    for &(w, h) in &[(333, 257), (800, 750), (1024, 768)] {
        let px = ridges(w, h, 7);
        let reference = encode(&px, w, h, 0.75, Mode::Exact, Backend::Scalar);
        let decoded = decode(&reference, Mode::Exact, Backend::Scalar, 1).pixels;
        for b in Backend::available() {
            assert_eq!(encode(&px, w, h, 0.75, Mode::Exact, b), reference, "encoder on {}", b.name());
            for t in [1, 2, 5] {
                assert_eq!(decode(&reference, Mode::Exact, b, t).pixels, decoded, "decoder on {} with {t} threads", b.name());
            }
            let fast = decode(&reference, Mode::Fast, b, 1).pixels;
            assert!(fast.iter().zip(&decoded).all(|(a, e)| a.abs_diff(*e) <= 1), "fast decoder on {}", b.name());
        }
    }
}

#[test]
fn info_reports_the_header() {
    let px = ridges(300, 200, 9);
    let data = encode(&px, 300, 200, 0.75, Mode::Exact, Backend::best());
    let i = wsq::read_info(&data).unwrap();
    assert_eq!((i.width, i.height, i.ppi, i.encoder, i.filters, i.blocks), (300, 200, Some(500), 2, (9, 7), 3));
    assert_eq!(i.bitrate, Some(0.75));
}

#[test]
fn bitrate_must_be_positive() {
    let px = vec![0u8; 64];
    for r in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let opts = EncodeOptions { bitrate: r, ..Default::default() };
        assert!(wsq::encode(&px, 8, 8, &opts).is_err());
    }
    assert!(wsq::encode(&px, 9, 8, &EncodeOptions::default()).is_err(), "wrong buffer length");
}
