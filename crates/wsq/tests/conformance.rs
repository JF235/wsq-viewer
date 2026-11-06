//! Conformance with NBIS 5.0.0 on the NIST WSQ reference data set v2.0.
//!
//! The data set is public (https://nigos.nist.gov/wsq/reference_images_v2.0_pgm.tar) but is not
//! part of the repository; set WSQ_NIST_DIR to its folder. The expected outputs are hashes of
//! what NBIS produced, in tests/data.

mod common;

use common::{fnv, nist_dir, read_pgm, skip};
use wsq::compliance::{decoder_measures, encoder_measures};
use wsq::{Backend, DecodeOptions, Decoder, EncodeOptions, Encoder, Mode};

fn lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).map(|l| l.split_whitespace().collect())
}

#[test]
fn exact_decoder_reproduces_nbis_on_every_backend() {
    let Some(dir) = nist_dir() else { return skip("NIST decoder test") };
    let expected = include_str!("data/nbis_decode.txt");
    let mut checked = 0;
    for f in lines(expected) {
        let (name, hash) = (f[0], u64::from_str_radix(f[1], 16).unwrap());
        let data = std::fs::read(dir.join("decode").join(format!("{name}.wsq"))).unwrap();
        for backend in Backend::available() {
            for threads in [1, 4] {
                let opts = DecodeOptions { mode: Mode::Exact, backend: Some(backend), threads, ..Default::default() };
                let img = Decoder::new(opts).decode(&data).unwrap();
                assert_eq!(fnv(&img.pixels), hash, "{name} on {} with {threads} threads", backend.name());
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 86);
}

#[test]
fn fast_decoder_meets_annex_aa3() {
    let Some(dir) = nist_dir() else { return skip("NIST decoder test") };
    for f in lines(include_str!("data/nbis_decode.txt")) {
        let data = std::fs::read(dir.join("decode").join(format!("{}.wsq", f[0]))).unwrap();
        let exact = Decoder::new(DecodeOptions::default()).decode(&data).unwrap();
        for backend in Backend::available() {
            let opts = DecodeOptions { mode: Mode::Fast, backend: Some(backend), ..Default::default() };
            let fast = Decoder::new(opts).decode(&data).unwrap();
            let m = decoder_measures(&fast.pixels, &exact.pixels);
            assert!(m.passes(), "{} on {}: {m:?}", f[0], backend.name());
        }
    }
}

#[test]
fn quantizer_indices_match_the_nist_reference() {
    let Some(dir) = nist_dir() else { return skip("NIST index test") };
    for name in ["cmp00001", "cmp00002"] {
        let data = std::fs::read(dir.join(format!("decode/75/{name}.wsq"))).unwrap();
        let idx = Decoder::new(DecodeOptions::default()).decode_indices(&data).unwrap();
        for (k, got) in idx.iter().enumerate().take(60) {
            let text = std::fs::read_to_string(dir.join(format!("decode/75/txt/{name}.{k:02}"))).unwrap();
            let want: Vec<i32> = text.lines().flat_map(|l| l.split_once(':').unwrap().1.split_whitespace().map(|v| v.parse::<i32>().unwrap())).collect();
            assert_eq!(got, &want, "{name} subband {k}");
        }
    }
}

#[test]
fn exact_encoder_writes_the_bytes_of_nbis_on_every_backend() {
    let Some(dir) = nist_dir() else { return skip("NIST encoder test") };
    let mut checked = 0;
    for f in lines(include_str!("data/nbis_encode.txt")) {
        let (name, rate, size, hash) = (f[0], f[1].parse::<f32>().unwrap(), f[2].parse::<usize>().unwrap(), u64::from_str_radix(f[3], 16).unwrap());
        let (w, h, px) = read_pgm(&dir.join(format!("{name}.pgm")));
        for backend in Backend::available() {
            let opts = EncodeOptions { bitrate: rate, ppi: Some(500), mode: Mode::Exact, backend: Some(backend), ..Default::default() };
            let out = Encoder::new().encode(&px, w, h, &opts).unwrap();
            assert_eq!((out.len(), fnv(&out)), (size, hash), "{name} at {rate} on {}", backend.name());
        }
        checked += 1;
    }
    assert_eq!(checked, 80);
}

#[test]
fn fast_encoder_meets_annex_aa2() {
    let Some(dir) = nist_dir() else { return skip("NIST encoder test") };
    for f in lines(include_str!("data/nbis_encode.txt")) {
        let (name, rate) = (f[0], f[1].parse::<f32>().unwrap());
        let (w, h, px) = read_pgm(&dir.join(format!("{name}.pgm")));
        let exact = wsq::encode(&px, w, h, &EncodeOptions { bitrate: rate, ppi: Some(500), ..Default::default() }).unwrap();
        let fast = wsq::encode(&px, w, h, &EncodeOptions { bitrate: rate, ppi: Some(500), mode: Mode::Fast, ..Default::default() }).unwrap();
        let m = encoder_measures(&fast, &exact).unwrap();
        assert!(m.passes(), "{name} at {rate}: {m:?}");
    }
}
