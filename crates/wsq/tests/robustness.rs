//! Malformed input: the decoder must return errors, never panic or loop.

mod common;

use common::{ridges, Lcg};
use wsq::{DecodeOptions, Decoder, EncodeOptions, Error};

fn sample() -> Vec<u8> {
    let px = ridges(96, 80, 11);
    wsq::encode(&px, 96, 80, &EncodeOptions { ppi: Some(500), ..Default::default() }).unwrap()
}

/// Marker segments of a stream: (marker, start, end) with end past the segment and its
/// entropy-coded data.
fn segments(d: &[u8]) -> Vec<(u16, usize, usize)> {
    let mut out = vec![(0xFFA0, 0, 2)];
    let mut p = 2;
    while p < d.len() {
        let m = u16::from_be_bytes([d[p], d[p + 1]]);
        if m == 0xFFA1 {
            out.push((m, p, p + 2));
            break;
        }
        let len = u16::from_be_bytes([d[p + 2], d[p + 3]]) as usize;
        let mut end = p + 2 + len;
        if m == 0xFFA3 {
            while !(d[end] == 0xFF && d[end + 1] != 0) {
                end += 1;
            }
        }
        out.push((m, p, end));
        p = end;
    }
    out
}

#[test]
fn every_truncation_is_an_error() {
    let data = sample();
    let full = wsq::decode(&data).unwrap();
    for n in 0..data.len() {
        let r = std::panic::catch_unwind(|| wsq::decode(&data[..n]));
        assert!(r.is_ok(), "panic at length {n}");
        assert!(r.unwrap().is_err(), "length {n} decoded");
    }
    assert_eq!(wsq::decode(&data).unwrap(), full);
}

#[test]
fn random_mutations_never_panic() {
    let data = sample();
    let mut rng = Lcg(5);
    let mut errors = 0;
    for _ in 0..3000 {
        let mut d = data.clone();
        for _ in 0..1 + rng.below(4) {
            let i = rng.below(d.len() as u64) as usize;
            d[i] = rng.below(256) as u8;
        }
        let r = std::panic::catch_unwind(|| wsq::decode(&d));
        assert!(r.is_ok(), "panic on a mutation");
        errors += r.unwrap().is_err() as usize;
    }
    assert!(errors > 0);
}

#[test]
fn oversized_and_empty_frames_are_refused() {
    let data = sample();
    let sof = segments(&data).into_iter().find(|s| s.0 == 0xFFA2).unwrap().1;
    let mut big = data.clone();
    big[sof + 6..sof + 10].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(matches!(wsq::decode(&big), Err(Error::TooLarge { .. })));
    let small = DecodeOptions { max_pixels: 1000, ..Default::default() };
    assert!(matches!(wsq::decode_with(&data, &small), Err(Error::TooLarge { .. })));
    let mut empty = data.clone();
    empty[sof + 6..sof + 8].copy_from_slice(&[0, 0]);
    assert!(matches!(wsq::decode(&empty), Err(Error::BadDimensions)));
}

#[test]
fn tables_can_come_in_a_separate_stream() {
    let data = sample();
    let full = wsq::decode(&data).unwrap();
    let segs = segments(&data);
    let (mut tables, mut image) = (vec![0xFF, 0xA0], vec![0xFF, 0xA0]);
    for &(m, a, b) in &segs[1..segs.len() - 1] {
        let target = if matches!(m, 0xFFA4..=0xFFA6) { &mut tables } else { &mut image };
        target.extend_from_slice(&data[a..b]);
    }
    tables.extend_from_slice(&[0xFF, 0xA1]);
    image.extend_from_slice(&[0xFF, 0xA1]);
    assert!(wsq::decode(&image).is_err(), "the image stream alone has no tables");
    let mut dec = Decoder::new(DecodeOptions::default());
    dec.install_tables(&tables).unwrap();
    assert_eq!(dec.decode(&image).unwrap(), full);
}

#[test]
fn comments_and_fill_bytes_anywhere() {
    let data = sample();
    let full = wsq::decode(&data).unwrap();
    let mut out = Vec::new();
    for (i, &(_, a, b)) in segments(&data).iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // fill bytes before every marker
            out.extend_from_slice(&[0xFF, 0xA8, 0x00, 0x07, b'h', b'e', b'l', b'l', b'o']);
            out.push(0xFF);
            out.extend_from_slice(&data[a + 1..b]);
        } else {
            out.extend_from_slice(&data[a..b]);
        }
    }
    assert_eq!(wsq::decode(&out).unwrap().pixels, full.pixels);
    assert!(wsq::read_info(&out).unwrap().comments.iter().any(|c| c == b"hello"));
}

#[test]
fn invalid_tables_are_errors() {
    let data = sample();
    let dht = segments(&data).into_iter().find(|s| s.0 == 0xFFA6).unwrap().1;
    let mut bad = data.clone();
    bad[dht + 5] = 200; // more codes of length 1 than exist
    assert!(wsq::decode(&bad).is_err());
}
