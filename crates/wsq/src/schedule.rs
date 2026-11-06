//! Schedules: for one line length, the list of terms that make up every output sample of the
//! one-dimensional analysis or synthesis, in the order they are summed.
//!
//! Two families exist. The NBIS schedules record the exact operation order of the NIST
//! reference implementation (`get_lets` and `join_lets` of NBIS 5.0.0), so that a float32
//! evaluation reproduces its results bit for bit. The specification schedules follow the
//! formulas of the algorithm document; they merge the two samples that share a coefficient,
//! which halves the multiplications and is meant for fused multiply-add evaluation.

/// One term of an exact schedule: `x[idx] * coef`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Term {
    pub idx: u32,
    pub coef: f32,
}

/// Exact schedule: `out[o] = t0; out[o] += t1; ...` in float32, in this order.
#[derive(Debug, Clone)]
pub struct ExactSchedule {
    pub n: usize,
    pub start: Vec<u32>,
    pub terms: Vec<Term>,
}

/// One term of a folded schedule: `(x[a] + x[b]) * coef`, or `x[a] * coef` when `b == a`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pair {
    pub a: u32,
    pub b: u32,
    pub coef: f32,
}

#[derive(Debug, Clone)]
pub struct FoldedSchedule {
    pub n: usize,
    pub start: Vec<u32>,
    pub pairs: Vec<Pair>,
}

#[derive(Clone, Copy)]
enum Op {
    Assign(i64, f32),
    Add(i64, f32),
}

/// Collects the operations applied to each output position, in program order.
struct Recorder {
    n: usize,
    /// (output position, input index, coefficient, is an assignment), in program order
    events: Vec<(u32, i64, f32, bool)>,
    bad: bool,
}

impl Recorder {
    fn new(n: usize) -> Self {
        Recorder { n, events: Vec::with_capacity(n * 10), bad: false }
    }
    #[inline]
    fn op(&mut self, pos: i64, op: Op) {
        if pos < 0 || pos as usize >= self.n {
            self.bad = true;
            return;
        }
        match op {
            Op::Assign(i, c) => self.events.push((pos as u32, i, c, true)),
            Op::Add(i, c) => self.events.push((pos as u32, i, c, false)),
        }
    }
    fn finish(self, n: usize) -> Option<ExactSchedule> {
        if self.bad {
            return None;
        }
        // stable bucketing by output position
        let mut count = vec![0u32; n + 1];
        for e in &self.events {
            count[e.0 as usize + 1] += 1;
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let mut fill = count.clone();
        let mut sorted = vec![(0i64, 0f32, false); self.events.len()];
        for e in &self.events {
            let slot = &mut fill[e.0 as usize];
            sorted[*slot as usize] = (e.1, e.2, e.3);
            *slot += 1;
        }
        let mut start = Vec::with_capacity(n + 1);
        let mut terms = Vec::with_capacity(self.events.len());
        for o in 0..n {
            let ops = &sorted[count[o] as usize..count[o + 1] as usize];
            // an assignment discards what was written before it
            let from = ops.iter().rposition(|e| e.2).unwrap_or(0);
            start.push(terms.len() as u32);
            let first = terms.len();
            for &(i, c, _) in &ops[from..] {
                if i == -1 {
                    continue; // the zero initialization of NBIS
                }
                if i < 0 || i as usize >= n {
                    return None; // NBIS would read outside the line
                }
                if c == 0.0 && terms.len() > first {
                    continue; // adding a zero product changes nothing
                }
                terms.push(Term { idx: i as u32, coef: c });
            }
        }
        start.push(terms.len() as u32);
        Some(ExactSchedule { n, start, terms })
    }
}

/// NBIS `get_lets`: analysis of one line of length `n` with the analysis lowpass `lo` and
/// highpass `hi` as NBIS stores them. Output: lowpass then highpass, or the reverse if `inv`.
/// Returns None when NBIS would read outside the line (very short lines).
pub fn nbis_analysis(n: usize, lo: &[f32], hi: &[f32], inv: bool) -> Option<ExactSchedule> {
    if n < 2 {
        return None;
    }
    let (lsz, hsz) = (lo.len() as i64, hi.len() as i64);
    let hi: Vec<f32> = if lsz % 2 == 0 { hi.iter().map(|&v| -v).collect() } else { hi.to_vec() };
    let n_i = n as i64;
    let da_ev = n_i % 2 == 1;
    let fi_ev = lsz % 2 == 1;
    let (mut loc, mut hoc, mut olle, mut ohle, olre, ohre);
    if fi_ev {
        loc = (lsz - 1) / 2;
        hoc = (hsz - 1) / 2 - 1;
        olle = false;
        ohle = false;
        olre = false;
        ohre = false;
    } else {
        loc = lsz / 2 - 2;
        hoc = hsz / 2 - 2;
        olle = true;
        ohle = true;
        olre = true;
        ohre = true;
        if loc == -1 {
            loc = 0;
            olle = false;
        }
        if hoc == -1 {
            hoc = 0;
            ohle = false;
        }
    }
    let (llen, hlen) = if da_ev { ((n_i + 1) / 2, (n_i + 1) / 2 - 1) } else { (n_i / 2, n_i / 2) };
    let (pstr, nstr) = (1i64, -1i64);
    let (p0, p1) = (0i64, n_i - 1);
    let mut rec = Recorder::new(n);
    let (mut lopass, mut hipass) = if inv { (hlen, 0) } else { (0, llen) };
    let (mut lspx, mut lspxstr, mut lle2, lre2) = (loc, nstr, olle, olre);
    let (mut hspx, mut hspxstr, mut hle2, hre2) = (hoc, nstr, ohle, ohre);

    let filt = |rec: &mut Recorder, out: i64, start: i64, mut str_: i64, mut le: bool, mut re: bool, f: &[f32]| {
        let mut px = start;
        rec.op(out, Op::Assign(px, f[0]));
        for &c in &f[1..] {
            if px == p0 {
                if le {
                    str_ = 0;
                    le = false;
                } else {
                    str_ = pstr;
                }
            }
            if px == p1 {
                if re {
                    str_ = 0;
                    re = false;
                } else {
                    str_ = nstr;
                }
            }
            px += str_;
            rec.op(out, Op::Add(px, c));
        }
    };
    for _ in 0..hlen {
        filt(&mut rec, lopass, lspx, lspxstr, lle2, lre2, lo);
        lopass += 1;
        filt(&mut rec, hipass, hspx, hspxstr, hle2, hre2, &hi);
        hipass += 1;
        for _ in 0..2 {
            if lspx == p0 {
                if lle2 {
                    lspxstr = 0;
                    lle2 = false;
                } else {
                    lspxstr = pstr;
                }
            }
            lspx += lspxstr;
            if hspx == p0 {
                if hle2 {
                    hspxstr = 0;
                    hle2 = false;
                } else {
                    hspxstr = pstr;
                }
            }
            hspx += hspxstr;
        }
    }
    if da_ev {
        filt(&mut rec, lopass, lspx, lspxstr, lle2, lre2, lo);
    }
    rec.finish(n)
}

/// NBIS `join_lets`: synthesis of one line of length `n` from the lowpass half and the highpass
/// half (highpass first if `inv`), with the synthesis filters `lo` and `hi` as NBIS builds them
/// from the transform table. Returns None when NBIS would read outside the line.
pub fn nbis_synthesis(n: usize, lo: &[f32], hi: &[f32], inv: bool) -> Option<ExactSchedule> {
    if n < 2 {
        return None;
    }
    let (lsz, hsz) = (lo.len() as i64, hi.len() as i64);
    let n_i = n as i64;
    let da_ev = n_i % 2 == 1;
    let fi_ev = lsz % 2 == 1;
    let (llen, hlen) = if da_ev { ((n_i + 1) / 2, (n_i + 1) / 2 - 1) } else { (n_i / 2, n_i / 2) };
    // (asym, ssfac, ofhre, loc, hoc, lotap, hotap, olle, olre, ohle, ohre)
    let (asym, ssfac, ofhre, mut loc, mut hoc, lotap, hotap, mut olle, olre, mut ohle, ohre) = if fi_ev {
        let (a, b, c, d) = if da_ev { (false, false, true, true) } else { (false, true, true, false) };
        (false, 1.0f32, 0i32, (lsz - 1) / 4, (hsz + 1) / 4 - 1, ((lsz - 1) / 2) % 2 == 1, ((hsz + 1) / 2) % 2 == 1, a, b, c, d)
    } else {
        let (a, b, c, d) = if da_ev { (true, false, true, true) } else { (true, true, true, true) };
        (true, -1.0f32, 2i32, lsz / 4 - 1, hsz / 4 - 1, (lsz / 2) % 2 == 1, (hsz / 2) % 2 == 1, a, b, c, d)
    };
    if !fi_ev {
        if loc == -1 {
            loc = 0;
            olle = false;
        }
        if hoc == -1 {
            hoc = 0;
            ohle = false;
        }
    }
    let hi: Vec<f32> = if fi_ev { hi.to_vec() } else { hi.iter().map(|&v| -v).collect() };
    let (pstr, nstr) = (1i64, -1i64);
    let (lbase, hbase) = if inv { (hlen, 0) } else { (0, llen) };
    let (lp0, lp1) = (0i64, llen - 1);
    let (hp0, hp1) = (0i64, hlen - 1);
    let mut rec = Recorder::new(n);
    let mut limg = 0i64;
    let mut himg = 0i64;
    rec.op(0, Op::Assign(-1, 0.0)); // *himg = 0.0 (marker term, removed below)
    rec.op(1, Op::Assign(-1, 0.0));
    let (mut lspx, mut lspxstr, mut lstap, mut lle2, lre2) = (loc, nstr, lotap as i64, olle, olre);
    let (mut hspx, mut hspxstr, mut hstap, mut hle2, hre2) = (hoc, nstr, hotap as i64, ohle, ohre);
    let mut osfac = ssfac;
    let mut fhre: i32;

    // low taps of one output, starting at tap `tap`
    let low = |rec: &mut Recorder, out: i64, tap: i64, spx: i64, mut str_: i64, mut le: bool, mut re: bool| {
        let mut px = spx;
        rec.op(out, Op::Assign(lbase + px, lo[tap as usize]));
        let mut i = tap + 2;
        while i < lsz {
            if px == lp0 {
                if le {
                    str_ = 0;
                    le = false;
                } else {
                    str_ = pstr;
                }
            }
            if px == lp1 {
                if re {
                    str_ = 0;
                    re = false;
                } else {
                    str_ = nstr;
                }
            }
            px += str_;
            rec.op(out, Op::Add(lbase + px, lo[i as usize]));
            i += 2;
        }
    };
    // high taps of one output
    #[allow(clippy::too_many_arguments)]
    let high = |rec: &mut Recorder, out: i64, tap: i64, spx: i64, mut str_: i64, mut le: bool, mut re: bool, mut sfac: f32, mut fh: i32| {
        let mut px = spx;
        let mut i = tap;
        while i < hsz {
            if px == hp0 {
                if le {
                    str_ = 0;
                    le = false;
                } else {
                    str_ = pstr;
                    sfac = 1.0;
                }
            }
            if px == hp1 {
                if re {
                    str_ = 0;
                    re = false;
                    if asym && da_ev {
                        re = true;
                        fh -= 1;
                        sfac = fh as f32;
                        if sfac == 0.0 {
                            re = false;
                        }
                    }
                } else {
                    str_ = nstr;
                    if asym {
                        sfac = -1.0;
                    }
                }
            }
            rec.op(out, Op::Add(hbase + px, hi[i as usize] * sfac));
            px += str_;
            i += 2;
        }
    };

    for _ in 0..hlen {
        let mut tap = lstap;
        while tap >= 0 {
            low(&mut rec, limg, tap, lspx, lspxstr, lle2, lre2);
            limg += 1;
            tap -= 1;
        }
        if lspx == lp0 {
            if lle2 {
                lspxstr = 0;
                lle2 = false;
            } else {
                lspxstr = pstr;
            }
        }
        lspx += lspxstr;
        lstap = 1;
        let mut tap = hstap;
        while tap >= 0 {
            fhre = ofhre;
            high(&mut rec, himg, tap, hspx, hspxstr, hle2, hre2, osfac, fhre);
            himg += 1;
            tap -= 1;
        }
        if hspx == hp0 {
            if hle2 {
                hspxstr = 0;
                hle2 = false;
            } else {
                hspxstr = pstr;
                osfac = 1.0;
            }
        }
        hspx += hspxstr;
        hstap = 1;
    }
    lstap = match (da_ev, lotap) {
        (true, true) => 1,
        (true, false) => 0,
        (false, true) => 2,
        (false, false) => 1,
    };
    let mut tap = 1;
    while tap >= lstap {
        low(&mut rec, limg, tap, lspx, lspxstr, lle2, lre2);
        limg += 1;
        tap -= 1;
    }
    fhre = ofhre;
    if da_ev {
        hstap = if hotap { 1 } else { 0 };
        if hsz == 2 {
            hspx -= hspxstr;
            fhre = 1;
        }
    } else {
        hstap = if hotap { 2 } else { 1 };
    }
    let mut tap = 1;
    while tap >= hstap {
        if hsz != 2 {
            fhre = ofhre;
        }
        high(&mut rec, himg, tap, hspx, hspxstr, hle2, hre2, osfac, fhre);
        himg += 1;
        tap -= 1;
    }
    rec.finish(n)
}

// ---------------------------------------------------------------------------------------------
// Specification schedules
// ---------------------------------------------------------------------------------------------

/// An FIR filter `h(j)` for `j = lo .. lo + taps.len() - 1`.
#[derive(Debug, Clone)]
pub struct Fir {
    pub lo: i64,
    pub taps: Vec<f64>,
}

impl Fir {
    fn iter(&self) -> impl Iterator<Item = (i64, f64)> + '_ {
        self.taps.iter().enumerate().map(move |(i, &t)| (self.lo + i as i64, t))
    }
}

/// A filter bank in the form of the specification (Section 3.2 of the algorithm document).
#[derive(Debug, Clone)]
pub struct Bank {
    pub whole_sample: bool,
    pub h0: Fir,
    pub h1: Fir,
    pub f0: Fir,
    pub f1: Fir,
}

impl Bank {
    /// From the transmitted right halves.
    pub fn from_halves(l0: usize, h0_half: &[f64], h1_half: &[f64]) -> Bank {
        let ws = l0 % 2 == 1;
        let (h0, h1) = if ws {
            let r0 = h0_half.len() as i64 - 1;
            let r1 = h1_half.len() as i64 - 1;
            (
                Fir { lo: -r0, taps: (-r0..=r0).map(|j| h0_half[j.unsigned_abs() as usize]).collect() },
                Fir { lo: -1 - r1, taps: (-1 - r1..=-1 + r1).map(|j| h1_half[(j + 1).unsigned_abs() as usize]).collect() },
            )
        } else {
            let r0 = h0_half.len() as i64;
            let r1 = h1_half.len() as i64;
            (
                Fir { lo: -r0, taps: (-r0..r0).map(|j| if j >= 0 { h0_half[j as usize] } else { h0_half[(-1 - j) as usize] }).collect() },
                Fir { lo: -r1, taps: (-r1..r1).map(|j| if j >= 0 { h1_half[j as usize] } else { -h1_half[(-1 - j) as usize] }).collect() },
            )
        };
        let f0 = Fir { lo: h1.lo + 1, taps: h1.iter().map(|(j, t)| if (j + 1).rem_euclid(2) == 0 { t } else { -t }).collect() };
        let f1 = Fir { lo: h0.lo + 1, taps: h0.iter().map(|(j, t)| if j.rem_euclid(2) == 0 { t } else { -t }).collect() };
        Bank { whole_sample: ws, h0, h1, f0, f1 }
    }

    fn ext(&self, n: i64, n0: i64) -> i64 {
        if n0 == 1 {
            return 0;
        }
        if self.whole_sample {
            let p = 2 * n0 - 2;
            let u = n.rem_euclid(p);
            if u < n0 { u } else { p - u }
        } else {
            let p = 2 * n0;
            let u = n.rem_euclid(p);
            if u < n0 { u } else { p - 1 - u }
        }
    }

    /// Index into the stored subband (and sign) of sample k of the extended subband `i`.
    fn sub_ext(&self, i: usize, k: i64, n0: i64) -> Option<(i64, f64)> {
        if self.whole_sample {
            if i == 0 {
                if n0 == 1 {
                    return Some((0, 1.0));
                }
                let p = n0 - 1;
                let u = k.rem_euclid(p);
                Some((u.min(p - u), 1.0))
            } else {
                if n0 < 2 {
                    return None;
                }
                let p = n0 - 1;
                let u = k.rem_euclid(p);
                Some((u.min(p - 1 - u), 1.0))
            }
        } else {
            let u = k.rem_euclid(n0);
            let v = n0 - 1 - u;
            if i == 0 {
                Some((u.min(v), 1.0))
            } else if u < v {
                Some((u, 1.0))
            } else if u > v {
                Some((v, -1.0))
            } else {
                None
            }
        }
    }
}

fn fold(terms: Vec<(i64, f64)>) -> Vec<Pair> {
    // merge equal indices, then pair up indices that share a coefficient
    let mut merged: Vec<(i64, f64)> = Vec::new();
    for (i, c) in terms {
        if let Some(e) = merged.iter_mut().find(|e| e.0 == i) {
            e.1 += c;
        } else {
            merged.push((i, c));
        }
    }
    merged.retain(|e| e.1 != 0.0);
    let mut out = Vec::new();
    let mut used = vec![false; merged.len()];
    for a in 0..merged.len() {
        if used[a] {
            continue;
        }
        used[a] = true;
        let ca = merged[a].1;
        let partner = (a + 1..merged.len()).find(|&b| !used[b] && merged[b].1 == ca);
        match partner {
            Some(b) => {
                used[b] = true;
                out.push(Pair { a: merged[a].0 as u32, b: merged[b].0 as u32, coef: ca as f32 });
            }
            // a lone term is stored as (x + x) * c / 2, which is exact, so that every term has
            // the same shape
            None => out.push(Pair { a: merged[a].0 as u32, b: merged[a].0 as u32, coef: (ca / 2.0) as f32 }),
        }
    }
    out
}

fn folded(n: usize, per_output: Vec<Vec<(i64, f64)>>) -> FoldedSchedule {
    let mut start = Vec::with_capacity(n + 1);
    let mut pairs = Vec::new();
    for t in per_output {
        start.push(pairs.len() as u32);
        pairs.extend(fold(t));
    }
    start.push(pairs.len() as u32);
    FoldedSchedule { n, start, pairs }
}

fn exact_from_terms(n: usize, per_output: Vec<Vec<(i64, f64)>>) -> ExactSchedule {
    let mut start = Vec::with_capacity(n + 1);
    let mut terms = Vec::new();
    for t in per_output {
        start.push(terms.len() as u32);
        let mut merged: Vec<(i64, f64)> = Vec::new();
        for (i, c) in t {
            if let Some(e) = merged.iter_mut().find(|e| e.0 == i) {
                e.1 += c;
            } else {
                merged.push((i, c));
            }
        }
        terms.extend(merged.into_iter().filter(|e| e.1 != 0.0).map(|(i, c)| Term { idx: i as u32, coef: c as f32 }));
    }
    start.push(terms.len() as u32);
    ExactSchedule { n, start, terms }
}

fn analysis_terms(n: usize, bank: &Bank, inv: bool) -> Vec<Vec<(i64, f64)>> {
    let n0 = n as i64;
    let (rho0, rho1) = (n.div_ceil(2), n / 2);
    let mut out = vec![Vec::new(); n];
    let (base0, base1) = if inv { (rho1, 0) } else { (0, rho0) };
    for k in 0..rho0 {
        out[base0 + k] = bank.h0.iter().map(|(j, c)| (bank.ext(2 * k as i64 - j, n0), c)).collect();
    }
    for k in 0..rho1 {
        out[base1 + k] = bank.h1.iter().map(|(j, c)| (bank.ext(2 * k as i64 - j, n0), c)).collect();
    }
    out
}

fn synthesis_terms(n: usize, bank: &Bank, inv: bool) -> Vec<Vec<(i64, f64)>> {
    let n0 = n as i64;
    let (rho0, rho1) = (n.div_ceil(2), n / 2);
    let (base0, base1) = if inv { (rho1 as i64, 0) } else { (0, rho0 as i64) };
    let mut out = vec![Vec::new(); n];
    for (x, terms) in out.iter_mut().enumerate() {
        let x = x as i64;
        for (i, f, base, rho) in [(0usize, &bank.f0, base0, rho0), (1, &bank.f1, base1, rho1)] {
            if rho == 0 {
                continue;
            }
            for (t, c) in f.iter() {
                let d = x - t;
                if d.rem_euclid(2) != 0 {
                    continue;
                }
                if let Some((idx, s)) = bank.sub_ext(i, d.div_euclid(2), n0) {
                    terms.push((base + idx, c * s));
                }
            }
        }
    }
    out
}

/// Specification analysis as an exact (unfused, unfolded) schedule.
pub fn spec_analysis_exact(n: usize, bank: &Bank, inv: bool) -> ExactSchedule {
    exact_from_terms(n, analysis_terms(n, bank, inv))
}

/// Specification synthesis as an exact (unfused, unfolded) schedule.
pub fn spec_synthesis_exact(n: usize, bank: &Bank, inv: bool) -> ExactSchedule {
    exact_from_terms(n, synthesis_terms(n, bank, inv))
}

/// Specification analysis of a line of length `n` (Algorithm 1), output lowpass then highpass,
/// or the reverse if `inv`.
pub fn spec_analysis(n: usize, bank: &Bank, inv: bool) -> FoldedSchedule {
    folded(n, analysis_terms(n, bank, inv))
}

/// Specification synthesis of a line of length `n` (Algorithm 2) from the two halves of the
/// input (highpass first if `inv`).
pub fn spec_synthesis(n: usize, bank: &Bank, inv: bool) -> FoldedSchedule {
    folded(n, synthesis_terms(n, bank, inv))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H0: [f64; 5] = [0.85269867900940, 0.37740285561265, -0.11062440441842, -0.02384946501938, 0.037828455506995];
    const H1: [f64; 4] = [0.78848561640566, -0.41809227322221, -0.040689417609558, 0.064538882628938];

    fn run_folded(s: &FoldedSchedule, x: &[f64]) -> Vec<f64> {
        (0..s.n)
            .map(|o| {
                s.pairs[s.start[o] as usize..s.start[o + 1] as usize]
                    .iter()
                    .map(|p| (x[p.a as usize] + x[p.b as usize]) * p.coef as f64)
                    .sum()
            })
            .collect()
    }

    #[test]
    fn spec_schedules_reconstruct() {
        let bank = Bank::from_halves(9, &H0, &H1);
        for n in 1..60 {
            for inv in [false, true] {
                let x: Vec<f64> = (0..n).map(|i| ((i * 37 + 11) % 23) as f64 - 11.0).collect();
                let y = run_folded(&spec_analysis(n, &bank, inv), &x);
                let z = run_folded(&spec_synthesis(n, &bank, inv), &y);
                for i in 0..n {
                    assert!((z[i] - x[i]).abs() < 1e-5, "n={n} inv={inv} i={i}: {} vs {}", z[i], x[i]);
                }
            }
        }
    }

    #[test]
    fn haar_half_sample_bank_reconstructs() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let bank = Bank::from_halves(2, &[s], &[-s]);
        for n in 1..20 {
            let x: Vec<f64> = (0..n).map(|i| (i * i % 7) as f64).collect();
            let y = run_folded(&spec_analysis(n, &bank, false), &x);
            let z = run_folded(&spec_synthesis(n, &bank, false), &y);
            for i in 0..n {
                assert!((z[i] - x[i]).abs() < 1e-6, "n={n}");
            }
        }
    }
}
