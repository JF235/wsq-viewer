//! `wsq`: encode, decode, inspect and benchmark WSQ files.

use std::path::Path;
use std::process::ExitCode;

use wsq::{Backend, DecodeOptions, EncodeOptions, Mode};

mod bench;
mod pgm;

fn usage() -> ExitCode {
    eprintln!(
        "usage:
  wsq decode <in.wsq> <out.pgm|out.raw> [--fast] [--backend NAME]
  wsq encode <in.pgm|in.raw WxH> <out.wsq> [--rate R] [--ppi N] [--fast] [--backend NAME]
  wsq info <in.wsq>
  wsq bench decode|encode|batch <files>... [--fast] [--backend NAME] [--reps N] [--rate R] [--threads T] [--rounds K]
  wsq backends"
    );
    ExitCode::from(2)
}

struct Flags {
    reps: usize,
    threads: usize,
    rounds: usize,
    fast: bool,
    backend: Option<Backend>,
    rate: f32,
    ppi: Option<u32>,
    rest: Vec<String>,
}

fn flags(args: &[String]) -> Result<Flags, String> {
    let mut f = Flags { reps: 20, threads: 1, rounds: 5, fast: false, backend: None, rate: 0.75, ppi: None, rest: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--fast" => f.fast = true,
            "--reps" => f.reps = it.next().ok_or("--reps needs a value")?.parse().map_err(|e| format!("{e}"))?,
            "--threads" => f.threads = it.next().ok_or("--threads needs a value")?.parse().map_err(|e| format!("{e}"))?,
            "--rounds" => f.rounds = it.next().ok_or("--rounds needs a value")?.parse().map_err(|e| format!("{e}"))?,
            "--backend" => {
                let n = it.next().ok_or("--backend needs a name")?;
                f.backend = Some(Backend::from_name(n).ok_or(format!("unknown backend {n}"))?);
            }
            "--rate" => f.rate = it.next().ok_or("--rate needs a value")?.parse().map_err(|e| format!("{e}"))?,
            "--ppi" => f.ppi = Some(it.next().ok_or("--ppi needs a value")?.parse().map_err(|e| format!("{e}"))?),
            _ => f.rest.push(a.clone()),
        }
    }
    Ok(f)
}

fn run(args: Vec<String>) -> Result<(), String> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let f = flags(&args[1.min(args.len())..])?;
    let mode = if f.fast { Mode::Fast } else { Mode::Exact };
    match (cmd, f.rest.as_slice()) {
        ("decode", [input, output]) => {
            let data = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
            let opts = DecodeOptions { mode, backend: f.backend, ..Default::default() };
            let img = wsq::decode_with(&data, &opts).map_err(|e| format!("{input}: {e}"))?;
            if output.ends_with(".raw") {
                std::fs::write(output, &img.pixels).map_err(|e| e.to_string())?;
            } else {
                pgm::write(Path::new(output), img.width, img.height, &img.pixels).map_err(|e| e.to_string())?;
            }
        }
        ("encode", [input, rest @ ..]) => {
            let (w, h, pixels, output) = if input.ends_with(".raw") {
                let [dims, output] = rest else { return Err("raw input needs WxH".into()) };
                let (w, h) = dims.split_once('x').ok_or("dimensions are WxH")?;
                let (w, h): (u32, u32) = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
                (w, h, std::fs::read(input).map_err(|e| e.to_string())?, output)
            } else {
                let [output] = rest else { return Err("encode needs an output".into()) };
                let (w, h, p) = pgm::read(Path::new(input)).map_err(|e| format!("{input}: {e}"))?;
                (w, h, p, output)
            };
            let opts = EncodeOptions { bitrate: f.rate, ppi: f.ppi, mode, backend: f.backend, ..Default::default() };
            let data = wsq::encode(&pixels, w, h, &opts).map_err(|e| e.to_string())?;
            std::fs::write(output, data).map_err(|e| e.to_string())?;
        }
        ("info", [input]) => {
            let data = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
            let i = wsq::read_info(&data).map_err(|e| e.to_string())?;
            println!("{}x{} ppi={:?} encoder={} software={} filters={:?} blocks={} bitrate={:?}", i.width, i.height, i.ppi, i.encoder, i.software, i.filters, i.blocks, i.bitrate);
            for c in &i.comments {
                println!("comment: {}", String::from_utf8_lossy(c).replace('\n', " | "));
            }
        }
        ("compare-encoded", [test, reference]) => {
            let (t, r) = (std::fs::read(test).map_err(|e| e.to_string())?, std::fs::read(reference).map_err(|e| e.to_string())?);
            let m = wsq::compliance::encoder_measures(&t, &r).map_err(|e| e.to_string())?;
            println!(
                "size {:+.4}% bins {:.5}% indices {:.5}% maxdiff {} {}",
                m.size_diff_percent, m.bin_width_diff_percent, m.equal_indices_percent, m.max_index_diff,
                if m.passes() { "PASS" } else { "FAIL" }
            );
        }
        ("bench", [what, files @ ..]) => match what.as_str() {
            "decode" => bench::decode(files, mode, f.backend, f.reps, f.threads)?,
            "encode" => bench::encode(files, mode, f.backend, f.reps, f.rate)?,
            "batch" => bench::batch(files, mode, f.backend, f.threads, f.rounds)?,
            "batch-coefficients" => bench::batch_coefficients(files, mode, f.backend, f.threads, f.rounds)?,
            "symbols" => bench::symbols(files, f.reps)?,
            "coverage" => bench::coverage(files.first().ok_or("coverage needs WxH")?, mode)?,
            "passes" => bench::passes(files.first().ok_or("passes needs WxH")?, mode, f.backend, f.reps)?,
            _ => return Err(String::new()),
        },
        ("dump-coefficients", [input, output]) => {
            // width, height (u32), m, r (f32), then width * height f32, little-endian
            let data = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
            let opts = DecodeOptions { mode, backend: f.backend, ..Default::default() };
            let c = wsq::Decoder::new(opts).decode_coefficients(&data).map_err(|e| e.to_string())?;
            let mut out = Vec::with_capacity(16 + 4 * c.coef.len());
            out.extend_from_slice(&(c.width as u32).to_le_bytes());
            out.extend_from_slice(&(c.height as u32).to_le_bytes());
            out.extend_from_slice(&c.m.to_le_bytes());
            out.extend_from_slice(&c.r.to_le_bytes());
            for v in &c.coef {
                out.extend_from_slice(&v.to_le_bytes());
            }
            std::fs::write(output, out).map_err(|e| e.to_string())?;
        }
        ("backends", []) => {
            for b in Backend::available() {
                println!("{} ({} lanes)", b.name(), b.lanes());
            }
            println!("best: {}", Backend::best().name());
        }
        _ => return Err(String::new()),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => usage(),
        Err(e) => {
            eprintln!("wsq: {e}");
            ExitCode::FAILURE
        }
    }
}
