//! Two-dimensional transform over the subband tree (Algorithm 4 of the algorithm document).
#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::sync::Arc;

use crate::dispatch;
use crate::kernels::{column_direct, column_pass, dequant_row, normalize_row, quantize_row, row_direct, row_pass, to_pixels, tokenize, RowPlan, Sched};
use crate::schedule::{self, Bank};
use crate::simd::{Backend, Vf32};
use crate::tree::{Region, Tree};

/// How the one-dimensional transforms are evaluated.
#[derive(Debug, Clone)]
pub enum Filters {
    /// In the operation order of NBIS. `lo` and `hi` are the filters exactly as NBIS holds them
    /// (synthesis filters built from the transform table for decoding, the encoder's analysis
    /// filters for encoding); `bank` serves lines too short for the NBIS order.
    Nbis { lo: Vec<f32>, hi: Vec<f32>, bank: Bank },
    /// In the order of the specification formulas, folded, with fused multiply-add.
    Folded { bank: Bank },
}

impl Filters {
    /// A fingerprint of the coefficients and evaluation order, for caching schedules.
    fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let bank = match self {
            Filters::Nbis { lo, hi, bank } => {
                0u8.hash(&mut h);
                lo.iter().chain(hi).for_each(|v| v.to_bits().hash(&mut h));
                bank
            }
            Filters::Folded { bank } => {
                1u8.hash(&mut h);
                bank
            }
        };
        bank.whole_sample.hash(&mut h);
        bank.h0.taps.iter().chain(&bank.h1.taps).for_each(|v| v.to_bits().hash(&mut h));
        h.finish()
    }
}

/// Schedules kept across transforms of images of the same sizes.
#[derive(Default)]
pub struct ScheduleCache {
    map: HashMap<(u64, Dir, usize, bool), Arc<Sched>>,
    rows: HashMap<(u64, Dir, usize, bool), Arc<RowPlan>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Dir {
    Analysis,
    Synthesis,
}

struct Step {
    r: Region,
    rows: Arc<Sched>,
    cols: Arc<Sched>,
    /// Row-direct form of `rows` and `cols` (synthesis plans only).
    row_plan: Option<Arc<RowPlan>>,
    col_plan: Option<Arc<RowPlan>>,
}

/// The schedules for every split of a tree.
pub struct Plan {
    steps: Vec<Step>,
}

fn make(n: usize, inv: bool, dir: Dir, f: &Filters) -> Sched {
    match (f, dir) {
        (Filters::Nbis { lo, hi, bank }, Dir::Synthesis) => schedule::nbis_synthesis(n, lo, hi, inv)
            .map(Sched::Exact)
            .unwrap_or_else(|| Sched::Exact(schedule::spec_synthesis_exact(n, bank, inv))),
        (Filters::Nbis { lo, hi, bank }, Dir::Analysis) => schedule::nbis_analysis(n, lo, hi, inv)
            .map(Sched::Exact)
            .unwrap_or_else(|| Sched::Exact(schedule::spec_analysis_exact(n, bank, inv))),
        (Filters::Folded { bank }, Dir::Synthesis) => Sched::Folded(schedule::spec_synthesis(n, bank, inv)),
        (Filters::Folded { bank }, Dir::Analysis) => Sched::Folded(schedule::spec_analysis(n, bank, inv)),
    }
}

impl Plan {
    fn new(tree: &Tree, f: &Filters, dir: Dir, skip_diagonal: bool, cache: &mut ScheduleCache) -> Plan {
        let key = f.key();
        let mut get = |n: usize, inv: bool| -> (Arc<Sched>, Option<Arc<RowPlan>>) {
            let s = cache.map.entry((key, dir, n, inv)).or_insert_with(|| Arc::new(make(n, inv, dir, f))).clone();
            let rp = (dir == Dir::Synthesis)
                .then(|| cache.rows.entry((key, dir, n, inv)).or_insert_with(|| Arc::new(RowPlan::new((*s).clone()))).clone());
            (s, rp)
        };
        let steps = tree
            .splits
            .iter()
            .enumerate()
            .filter(|&(i, r)| !(skip_diagonal && i == tree.diagonal_split) && r.w > 0 && r.h > 0)
            .map(|(_, &r)| {
                let (rows, row_plan) = get(r.w, r.inv_x);
                let (cols, col_plan) = get(r.h, r.inv_y);
                Step { r, rows, cols, row_plan, col_plan }
            })
            .collect();
        Plan { steps }
    }

    /// Plan of the inverse transform. `skip_diagonal` leaves out the split of the quadrant of
    /// subbands 60 to 63, which reconstructs to zero when none of them is transmitted.
    pub fn synthesis(tree: &Tree, f: &Filters, skip_diagonal: bool, cache: &mut ScheduleCache) -> Plan {
        Plan::new(tree, f, Dir::Synthesis, skip_diagonal, cache)
    }

    /// Plan of the forward transform.
    pub fn analysis(tree: &Tree, f: &Filters, skip_diagonal: bool, cache: &mut ScheduleCache) -> Plan {
        Plan::new(tree, f, Dir::Analysis, skip_diagonal, cache)
    }
}

/// Buffers reused across transforms.
#[derive(Default)]
pub struct Workspace {
    pub tmp: Vec<f32>,
    pub scratch: Vec<f32>,
}

/// Column pass of a synthesis step over columns `c0..c1` of the region. Narrow vectors gain
/// from evaluating even and odd outputs together; wide ones are faster with plain strips.
#[inline(always)]
unsafe fn synth_cols<V: Vf32>(st: &Step, src: *const f32, dst: *mut f32, width: usize, c0: usize, c1: usize) {
    unsafe {
        match &st.col_plan {
            Some(cp) if V::LANES <= 4 => column_direct::<V>(cp, src.add(c0), dst.add(c0), width, c1 - c0),
            _ => column_pass::<V>(&st.cols, src.add(c0), dst.add(c0), width, c1 - c0),
        }
    }
}

/// Row pass of a synthesis step over rows `r0..r1` of the region.
#[inline(always)]
unsafe fn synth_rows<V: Vf32>(st: &Step, src: *const f32, dst: *mut f32, width: usize, r0: usize, r1: usize, scratch: &mut Vec<f32>) {
    unsafe {
        match &st.row_plan {
            Some(rp) => row_direct::<V>(rp, src.add(r0 * width), dst.add(r0 * width), width, r1 - r0),
            None => row_pass::<V>(&st.rows, src.add(r0 * width), dst.add(r0 * width), width, r1 - r0, scratch),
        }
    }
}

#[inline(always)]
unsafe fn inverse_impl<V: Vf32>(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, scratch: &mut Vec<f32>) {
    unsafe {
        for st in plan.steps.iter().rev() {
            let off = st.r.y * width + st.r.x;
            synth_cols::<V>(st, img.add(off), tmp.add(off), width, 0, st.r.w);
            synth_rows::<V>(st, tmp.add(off), img.add(off), width, 0, st.r.h, scratch);
        }
    }
}

#[inline(always)]
unsafe fn forward_impl<V: Vf32>(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, scratch: &mut Vec<f32>) {
    unsafe {
        for st in plan.steps.iter() {
            let off = st.r.y * width + st.r.x;
            row_pass::<V>(&st.rows, img.add(off), tmp.add(off), width, st.r.h, scratch);
            column_pass::<V>(&st.cols, tmp.add(off), img.add(off), width, st.r.w);
        }
    }
}

/// Inverse transform in place on a `width`-wide coefficient array.
pub fn inverse(plan: &Plan, img: &mut [f32], width: usize, backend: Backend, ws: &mut Workspace) {
    assert!(backend.is_available());
    if ws.tmp.len() < img.len() {
        ws.tmp.resize(img.len(), 0.0);
    }
    let img = img.as_mut_ptr();
    let tmp = ws.tmp.as_mut_ptr();
    let scratch = &mut ws.scratch;
    dispatch!(backend, inverse_impl(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, scratch: &mut Vec<f32>) -> ())
}

/// Forward transform in place.
pub fn forward(plan: &Plan, img: &mut [f32], width: usize, backend: Backend, ws: &mut Workspace) {
    assert!(backend.is_available());
    if ws.tmp.len() < img.len() {
        ws.tmp.resize(img.len(), 0.0);
    }
    let img = img.as_mut_ptr();
    let tmp = ws.tmp.as_mut_ptr();
    let scratch = &mut ws.scratch;
    dispatch!(backend, forward_impl(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, scratch: &mut Vec<f32>) -> ())
}

/// Convert a coefficient array to pixels (see `kernels::to_pixels`).
pub fn pixels(src: &[f32], dst: &mut [u8], r: f32, m: f32, exact: bool, backend: Backend) {
    assert!(backend.is_available() && dst.len() == src.len());
    let n = src.len();
    let (src, dst) = (src.as_ptr(), dst.as_mut_ptr());
    dispatch!(backend, to_pixels(src: *const f32, dst: *mut u8, n: usize, r: f32, m: f32, exact: bool) -> ())
}

#[inline(always)]
unsafe fn cols_only<V: Vf32>(st: &Step, src: *const f32, dst: *mut f32, width: usize, c1: usize) {
    unsafe { synth_cols::<V>(st, src, dst, width, 0, c1) }
}

#[inline(always)]
unsafe fn rows_only<V: Vf32>(st: &Step, src: *const f32, dst: *mut f32, width: usize, r1: usize, scratch: &mut Vec<f32>) {
    unsafe { synth_rows::<V>(st, src, dst, width, 0, r1, scratch) }
}

/// Timing aid: run the passes of `plan` on `img` and return the seconds spent in column passes
/// and in row passes. With `identity`, every schedule is replaced by a copy, which leaves only
/// the cost of moving data (and, for rows, of the transposes).
#[doc(hidden)]
pub fn time_passes(plan: &Plan, img: &mut [f32], width: usize, backend: Backend, ws: &mut Workspace, identity: bool) -> (f64, f64) {
    use crate::schedule::{ExactSchedule, Term};
    assert!(backend.is_available());
    if ws.tmp.len() < img.len() {
        ws.tmp.resize(img.len(), 0.0);
    }
    let ident = |n: usize| Sched::Exact(ExactSchedule { n, start: (0..=n as u32).collect(), terms: (0..n as u32).map(|i| Term { idx: i, coef: 1.0 }).collect() });
    let (mut tc, mut tr) = (0.0, 0.0);
    for st in plan.steps.iter().rev() {
        let off = st.r.y * width + st.r.x;
        let (cols, rows) = if identity { (ident(st.r.h), ident(st.r.w)) } else { ((*st.cols).clone(), (*st.rows).clone()) };
        let img_p = img.as_mut_ptr().wrapping_add(off);
        let tmp_p = ws.tmp.as_mut_ptr().wrapping_add(off);
        let (w, h) = (st.r.w, st.r.h);
        let scratch = &mut ws.scratch;
        let t0 = std::time::Instant::now();
        let t1;
        if identity {
            {
                let (s, src, dst, stride, n) = (&cols, img_p as *const f32, tmp_p, width, w);
                dispatch!(backend, column_pass(s: &Sched, src: *const f32, dst: *mut f32, stride: usize, n: usize) -> ());
            }
            t1 = std::time::Instant::now();
            let (s, src, dst, stride, n) = (&rows, tmp_p as *const f32, img_p, width, h);
            dispatch!(backend, row_pass(s: &Sched, src: *const f32, dst: *mut f32, stride: usize, n: usize, scratch: &mut Vec<f32>) -> ());
        } else {
            {
                let (stp, src, dst, w0, c1) = (st, img_p as *const f32, tmp_p, width, w);
                dispatch!(backend, cols_only(stp: &Step, src: *const f32, dst: *mut f32, w0: usize, c1: usize) -> ());
            }
            t1 = std::time::Instant::now();
            let (stp, src, dst, w0, r1) = (st, tmp_p as *const f32, img_p, width, h);
            dispatch!(backend, rows_only(stp: &Step, src: *const f32, dst: *mut f32, w0: usize, r1: usize, scratch: &mut Vec<f32>) -> ());
        }
        tc += (t1 - t0).as_secs_f64();
        tr += t1.elapsed().as_secs_f64();
    }
    (tc, tr)
}

/// Timing aid: a synthesis plan for a `width` x `height` image with the standard filters.
#[doc(hidden)]
pub fn standard_synthesis_plan(width: usize, height: usize, exact: bool) -> Plan {
    let tree = Tree::new(width, height);
    let bank = crate::nbis::standard_bank();
    let f = if exact {
        // the synthesis filters NBIS builds from the transform table of its encoder
        let mut lo = vec![0f32; 7];
        let mut hi = vec![0f32; 9];
        let h0 = &crate::nbis::ENCODER_LO[4..];
        let h1 = &crate::nbis::ENCODER_HI[3..];
        for (c, &v) in h0.iter().enumerate() {
            let s = if c % 2 == 0 { 1.0 } else { -1.0 };
            hi[4 + c] = s * v;
            hi[4 - c] = s * v;
        }
        for (c, &v) in h1.iter().enumerate() {
            let s = if c % 2 == 0 { 1.0 } else { -1.0 };
            lo[3 + c] = s * v;
            lo[3 - c] = s * v;
        }
        Filters::Nbis { lo, hi, bank }
    } else {
        Filters::Folded { bank }
    };
    Plan::synthesis(&tree, &f, true, &mut ScheduleCache::default())
}

/// Timing aid: coverage of the row-direct plans (n, m0, m1) of a synthesis plan.
#[doc(hidden)]
pub fn row_coverage(plan: &Plan) -> Vec<(usize, usize, usize, usize)> {
    plan.steps.iter().filter_map(|st| st.row_plan.as_ref().map(|p| (st.r.h, p.sched.n(), p.m0, p.m1))).collect()
}

#[derive(Clone, Copy)]
struct Shared(*mut f32);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// Accessor, so that closures capture the whole (Send) wrapper rather than the raw field.
    fn get(self) -> *mut f32 {
        self.0
    }
}

/// Regions smaller than this are transformed by one thread.
const PARALLEL_MIN_PIXELS: usize = 1 << 16;

#[inline(always)]
unsafe fn parallel_step<V: Vf32>(st: &Step, img: *mut f32, tmp: *mut f32, width: usize, t: usize, threads: usize, barrier: &std::sync::Barrier) {
    unsafe {
        let off = st.r.y * width + st.r.x;
        let l = V::LANES;
        // columns in chunks that keep whole 64-lane strips
        let chunk = st.r.w.div_ceil(threads).div_ceil(64) * 64;
        let (c0, c1) = ((t * chunk).min(st.r.w), ((t + 1) * chunk).min(st.r.w));
        if c0 < c1 {
            synth_cols::<V>(st, img.add(off), tmp.add(off), width, c0, c1);
        }
        barrier.wait();
        let rows = st.r.h.div_ceil(threads);
        let (r0, r1) = ((t * rows).min(st.r.h), ((t + 1) * rows).min(st.r.h));
        if r0 < r1 {
            let mut scratch = Vec::new();
            synth_rows::<V>(st, tmp.add(off), img.add(off), width, r0, r1, &mut scratch);
        }
        barrier.wait();
        let _ = l;
    }
}

#[inline(always)]
unsafe fn small_steps_impl<V: Vf32>(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, scratch: &mut Vec<f32>, big: usize) {
    unsafe {
        for st in plan.steps[big..].iter().rev() {
            let off = st.r.y * width + st.r.x;
            synth_cols::<V>(st, img.add(off), tmp.add(off), width, 0, st.r.w);
            synth_rows::<V>(st, tmp.add(off), img.add(off), width, 0, st.r.h, scratch);
        }
    }
}

#[inline(always)]
unsafe fn worker_impl<V: Vf32>(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, big: usize, t: usize, threads: usize, barrier: &std::sync::Barrier) {
    unsafe {
        for st in plan.steps[..big].iter().rev() {
            parallel_step::<V>(st, img, tmp, width, t, threads, barrier);
        }
    }
}

/// Inverse transform with `threads` threads (the large splits are shared out).
pub fn inverse_threads(plan: &Plan, img: &mut [f32], width: usize, backend: Backend, ws: &mut Workspace, threads: usize) {
    if threads <= 1 {
        return inverse(plan, img, width, backend, ws);
    }
    assert!(backend.is_available());
    if ws.tmp.len() < img.len() {
        ws.tmp.resize(img.len(), 0.0);
    }
    let img = img.as_mut_ptr();
    let tmp = ws.tmp.as_mut_ptr();
    let big = plan.steps.iter().take_while(|st| st.r.len() >= PARALLEL_MIN_PIXELS).count();
    {
        let scratch = &mut ws.scratch;
        // small steps first, on this thread (synthesis runs from the deepest split)
        dispatch!(backend, small_steps_impl(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, scratch: &mut Vec<f32>, big: usize) -> ());
    }
    if big == 0 {
        return;
    }
    let barrier = std::sync::Barrier::new(threads);
    let (imgs, tmps) = (Shared(img), Shared(tmp));
    // each thread dispatches on its own, so that the kernels are compiled with the backend's
    // instruction set (closures do not inherit target features)
    let work = |t: usize| {
        let (img, tmp, barrier) = (imgs.get(), tmps.get(), &barrier);
        dispatch!(backend, worker_impl(plan: &Plan, img: *mut f32, tmp: *mut f32, width: usize, big: usize, t: usize, threads: usize, barrier: &std::sync::Barrier) -> ())
    };
    std::thread::scope(|s| {
        for t in 1..threads {
            let work = &work;
            s.spawn(move || work(t));
        }
        work(0);
    });
}

/// Pixel conversion with `threads` threads.
pub fn pixels_threads(src: &[f32], dst: &mut [u8], r: f32, m: f32, exact: bool, backend: Backend, threads: usize) {
    if threads <= 1 || src.len() < PARALLEL_MIN_PIXELS {
        return pixels(src, dst, r, m, exact, backend);
    }
    let chunk = src.len().div_ceil(threads).div_ceil(64) * 64;
    std::thread::scope(|s| {
        for (a, b) in src.chunks(chunk).zip(dst.chunks_mut(chunk)) {
            s.spawn(move || pixels(a, b, r, m, exact, backend));
        }
    });
}

/// Dequantize a row of indices (see `kernels::dequant_row`).
pub fn dequantize(src: &[f32], dst: &mut [f32], q: f32, z: f32, c: f32, exact: bool, backend: Backend) {
    assert!(backend.is_available() && dst.len() == src.len());
    let n = src.len();
    let zh = (z as f64 / 2.0) as f32;
    let (p, out) = (src.as_ptr(), dst.as_mut_ptr());
    dispatch!(backend, dequant_row(p: *const f32, out: *mut f32, n: usize, q: f32, zh: f32, c: f32, exact: bool) -> ())
}

/// Quantize a row of coefficients into float indices (see `kernels::quantize_row`); returns
/// the largest index magnitude.
pub fn quantize(src: &[f32], dst: &mut [f32], q: f32, zbin: f32, backend: Backend) -> f32 {
    assert!(backend.is_available() && dst.len() == src.len());
    let n = src.len();
    let (src, out) = (src.as_ptr(), dst.as_mut_ptr());
    dispatch!(backend, quantize_row(src: *const f32, out: *mut f32, n: usize, q: f32, zbin: f32) -> f32)
}

/// Symbols of a block of indices (see `kernels::tokenize`).
pub fn tokens(v: &[f32], out: &mut Vec<(u8, u16, u8)>, backend: Backend) {
    assert!(backend.is_available());
    let n = v.len();
    let v = v.as_ptr();
    dispatch!(backend, tokenize(v: *const f32, n: usize, out: &mut Vec<(u8, u16, u8)>) -> ())
}

/// Normalize pixels (see `kernels::normalize_row`).
pub fn normalize(src: &[u8], dst: &mut [f32], m: f32, r: f32, backend: Backend) {
    assert!(backend.is_available() && dst.len() == src.len());
    let n = src.len();
    let (src, out) = (src.as_ptr(), dst.as_mut_ptr());
    dispatch!(backend, normalize_row(src: *const u8, out: *mut f32, n: usize, m: f32, r: f32) -> ())
}

