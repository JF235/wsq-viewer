//! `wsq bench`: timing of the codec on a set of files.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use wsq::{Backend, DecodeOptions, Decoder, EncodeOptions, Encoder, Mode};

use crate::pgm;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn name(p: &str) -> &str {
    Path::new(p).file_name().and_then(|n| n.to_str()).unwrap_or(p)
}

/// Per file: name, width, height, best and median total time, then the median of each stage.
pub fn decode(files: &[String], mode: Mode, backend: Option<Backend>, reps: usize, threads: usize) -> Result<(), String> {
    let opts = DecodeOptions { mode, backend, threads, ..Default::default() };
    let mut dec = Decoder::new(opts);
    for f in files {
        let data = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
        let (img, _) = dec.decode_profiled(&data).map_err(|e| format!("{f}: {e}"))?; // warm-up
        let mut totals = Vec::with_capacity(reps);
        let mut stages: Vec<Vec<f64>> = Vec::new();
        let mut labels = Vec::new();
        for _ in 0..reps {
            let t0 = Instant::now();
            let (_, prof) = dec.decode_profiled(&data).map_err(|e| e.to_string())?;
            totals.push(t0.elapsed().as_secs_f64() * 1e3);
            if stages.is_empty() {
                stages = vec![Vec::new(); prof.stages.len()];
                labels = prof.stages.iter().map(|s| s.0).collect();
            }
            for (i, s) in prof.stages.iter().enumerate() {
                stages[i].push(s.1);
            }
        }
        let best = totals.iter().cloned().fold(f64::INFINITY, f64::min);
        let med = median(&mut totals);
        let st: Vec<String> = labels.iter().zip(stages.iter_mut()).map(|(l, v)| format!("{l}={:.4}", median(v))).collect();
        println!("{} {} {} {:.4} {:.4} {}", name(f), img.width, img.height, best, med, st.join(" "));
    }
    Ok(())
}

pub fn encode(files: &[String], mode: Mode, backend: Option<Backend>, reps: usize, rate: f32) -> Result<(), String> {
    let opts = EncodeOptions { bitrate: rate, ppi: Some(500), mode, backend, ..Default::default() };
    for f in files {
        let (w, h, px) = pgm::read(Path::new(f)).map_err(|e| format!("{f}: {e}"))?;
        let mut enc = Encoder::new();
        enc.encode(&px, w, h, &opts).map_err(|e| e.to_string())?; // warm-up
        let mut totals = Vec::with_capacity(reps);
        let mut stages: Vec<Vec<f64>> = Vec::new();
        let mut labels = Vec::new();
        for _ in 0..reps {
            let t0 = Instant::now();
            let (_, prof) = enc.encode_profiled(&px, w, h, &opts).map_err(|e| e.to_string())?;
            totals.push(t0.elapsed().as_secs_f64() * 1e3);
            if stages.is_empty() {
                stages = vec![Vec::new(); prof.stages.len()];
                labels = prof.stages.iter().map(|s| s.0).collect();
            }
            for (i, s) in prof.stages.iter().enumerate() {
                stages[i].push(s.1);
            }
        }
        let best = totals.iter().cloned().fold(f64::INFINITY, f64::min);
        let med = median(&mut totals);
        let st: Vec<String> = labels.iter().zip(stages.iter_mut()).map(|(l, v)| format!("{l}={:.4}", median(v))).collect();
        println!("{} {} {} {:.4} {:.4} {}", name(f), w, h, best, med, st.join(" "));
    }
    Ok(())
}

/// Decode every file `rounds` times on `threads` threads; print images per second and
/// megapixels per second. With `coefficients`, stop before the inverse transform.
pub fn batch(files: &[String], mode: Mode, backend: Option<Backend>, threads: usize, rounds: usize) -> Result<(), String> {
    batch_inner(files, mode, backend, threads, rounds, false)
}

pub fn batch_coefficients(files: &[String], mode: Mode, backend: Option<Backend>, threads: usize, rounds: usize) -> Result<(), String> {
    batch_inner(files, mode, backend, threads, rounds, true)
}

fn batch_inner(files: &[String], mode: Mode, backend: Option<Backend>, threads: usize, rounds: usize, coefficients: bool) -> Result<(), String> {
    let data: Vec<Vec<u8>> = files.iter().map(|f| std::fs::read(f).map_err(|e| format!("{f}: {e}"))).collect::<Result<_, _>>()?;
    let pixels: usize = data
        .iter()
        .map(|d| wsq::read_info(d).map(|i| (i.width * i.height) as usize).unwrap_or(0))
        .sum();
    let jobs = data.len() * rounds;
    let next = AtomicUsize::new(0);
    let opts = DecodeOptions { mode, backend, ..Default::default() };
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut dec = Decoder::new(opts.clone());
                loop {
                    let j = next.fetch_add(1, Ordering::Relaxed);
                    if j >= jobs {
                        break;
                    }
                    if coefficients {
                        dec.decode_coefficients(&data[j % data.len()]).expect("decode");
                    } else {
                        dec.decode(&data[j % data.len()]).expect("decode");
                    }
                }
            });
        }
    });
    let dt = t0.elapsed().as_secs_f64();
    println!(
        "threads={threads} images={jobs} seconds={dt:.4} images_per_s={:.1} mpix_per_s={:.1}",
        jobs as f64 / dt,
        (pixels * rounds) as f64 / dt / 1e6
    );
    Ok(())
}

/// Time the column and row passes of the inverse transform for a `w` x `h` image.
pub fn passes(dims: &str, mode: Mode, backend: Option<Backend>, reps: usize) -> Result<(), String> {
    let (w, h) = dims.split_once('x').ok_or("dimensions are WxH")?;
    let (w, h): (usize, usize) = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
    let backend = backend.unwrap_or_else(Backend::best);
    let plan = wsq::transform::standard_synthesis_plan(w, h, mode == Mode::Exact);
    let mut img: Vec<f32> = (0..w * h).map(|i| ((i * 2654435761usize) % 1000) as f32 / 100.0 - 5.0).collect();
    let mut ws = wsq::transform::Workspace::default();
    for identity in [false, true] {
        let (mut c, mut r) = (Vec::new(), Vec::new());
        for _ in 0..reps {
            let (tc, tr) = wsq::transform::time_passes(&plan, &mut img, w, backend, &mut ws, identity);
            c.push(tc * 1e3);
            r.push(tr * 1e3);
        }
        println!("{} {}x{} cols={:.3} rows={:.3} total={:.3}", if identity { "copy" } else { "transform" }, w, h, median(&mut c), median(&mut r), median(&mut c) + median(&mut r));
    }
    // memory reference: one copy of the image
    let src = img.clone();
    let mut t = Vec::new();
    for _ in 0..reps {
        let t0 = Instant::now();
        img.copy_from_slice(&src);
        t.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    println!("memcpy {:.3} ms for {:.1} MB", median(&mut t), (w * h * 4) as f64 / 1e6);
    Ok(())
}

pub fn coverage(dims: &str, mode: Mode) -> Result<(), String> {
    let (w, h) = dims.split_once('x').ok_or("dimensions are WxH")?;
    let (w, h): (usize, usize) = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
    let plan = wsq::transform::standard_synthesis_plan(w, h, mode == Mode::Exact);
    for (rows, n, m0, m1) in wsq::transform::row_coverage(&plan) {
        println!("rows={rows} n={n} vector outputs {}..{} ({:.0}%)", 2 * m0, 2 * m1, 200.0 * (m1 - m0) as f64 / n as f64);
    }
    Ok(())
}

pub fn symbols(files: &[String], reps: usize) -> Result<(), String> {
    for f in files {
        let data = std::fs::read(f).map_err(|e| e.to_string())?;
        let mut t = Vec::new();
        let mut n = 0;
        for _ in 0..reps {
            let (k, s) = wsq::time_symbols(&data).map_err(|e| e.to_string())?;
            n = k;
            t.push(s * 1e3);
        }
        let m = median(&mut t);
        println!("{} symbols={} ms={:.4} ns_per_symbol={:.2}", name(f), n, m, m * 1e6 / n as f64);
    }
    Ok(())
}
