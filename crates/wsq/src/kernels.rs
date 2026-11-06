//! Transform kernels, generic over the vector type: they apply a schedule to many lines at
//! once, one line per vector lane, so every lane performs exactly the operations a scalar
//! evaluation of that line would.
//!
//! # Safety
//!
//! The kernels take raw pointers and strides; callers (in `transform`) guarantee that every
//! index a schedule produces stays inside the region the pointers describe, and run them only
//! through `dispatch!` with an available backend.
#![allow(clippy::missing_safety_doc, clippy::too_many_arguments)]

use crate::schedule::{ExactSchedule, FoldedSchedule};
use crate::simd::{Scalar, Vf32};

/// A schedule to apply on one pass.
#[derive(Debug, Clone)]
pub enum Sched {
    /// Operation order fixed (NBIS order or unfused specification order).
    Exact(ExactSchedule),
    /// Folded terms with fused multiply-add.
    Folded(FoldedSchedule),
}

impl Sched {
    pub fn n(&self) -> usize {
        match self {
            Sched::Exact(s) => s.n,
            Sched::Folded(s) => s.n,
        }
    }
}

/// Apply `s` to `lanes` lines: sample i of lane l at `src[i * ss + l]`, output o at
/// `dst[o * ds + l]`. `lanes` must be a multiple of `V::LANES`.
#[inline(always)]
pub unsafe fn apply<V: Vf32>(s: &Sched, src: *const f32, ss: usize, dst: *mut f32, ds: usize, lanes: usize) {
    unsafe {
        match s {
            Sched::Exact(e) => apply_exact::<V>(e, src, ss, dst, ds, lanes),
            Sched::Folded(f) => apply_folded::<V>(f, src, ss, dst, ds, lanes),
        }
    }
}

#[inline(always)]
unsafe fn apply_exact<V: Vf32>(s: &ExactSchedule, src: *const f32, ss: usize, dst: *mut f32, ds: usize, lanes: usize) {
    unsafe {
        let l = V::LANES;
        let nvec = lanes / l;
        let terms = s.terms.as_ptr();
        let mut v = 0;
        while v + 4 <= nvec {
            let off = v * l;
            for o in 0..s.n {
                let (a, b) = (*s.start.get_unchecked(o) as usize, *s.start.get_unchecked(o + 1) as usize);
                let q = dst.add(o * ds + off);
                if a == b {
                    let z = V::splat(0.0);
                    z.store(q);
                    z.store(q.add(l));
                    z.store(q.add(2 * l));
                    z.store(q.add(3 * l));
                    continue;
                }
                let t = &*terms.add(a);
                let c = V::splat(t.coef);
                let p = src.add(t.idx as usize * ss + off);
                let mut a0 = V::load(p).mul(c);
                let mut a1 = V::load(p.add(l)).mul(c);
                let mut a2 = V::load(p.add(2 * l)).mul(c);
                let mut a3 = V::load(p.add(3 * l)).mul(c);
                for k in a + 1..b {
                    let t = &*terms.add(k);
                    let c = V::splat(t.coef);
                    let p = src.add(t.idx as usize * ss + off);
                    a0 = a0.add(V::load(p).mul(c));
                    a1 = a1.add(V::load(p.add(l)).mul(c));
                    a2 = a2.add(V::load(p.add(2 * l)).mul(c));
                    a3 = a3.add(V::load(p.add(3 * l)).mul(c));
                }
                a0.store(q);
                a1.store(q.add(l));
                a2.store(q.add(2 * l));
                a3.store(q.add(3 * l));
            }
            v += 4;
        }
        while v < nvec {
            let off = v * l;
            for o in 0..s.n {
                let (a, b) = (*s.start.get_unchecked(o) as usize, *s.start.get_unchecked(o + 1) as usize);
                let q = dst.add(o * ds + off);
                let mut acc = V::splat(0.0);
                if a < b {
                    let t = &*terms.add(a);
                    acc = V::load(src.add(t.idx as usize * ss + off)).mul(V::splat(t.coef));
                    for k in a + 1..b {
                        let t = &*terms.add(k);
                        acc = acc.add(V::load(src.add(t.idx as usize * ss + off)).mul(V::splat(t.coef)));
                    }
                }
                acc.store(q);
            }
            v += 1;
        }
    }
}

#[inline(always)]
unsafe fn apply_folded<V: Vf32>(s: &FoldedSchedule, src: *const f32, ss: usize, dst: *mut f32, ds: usize, lanes: usize) {
    unsafe {
        let l = V::LANES;
        let nvec = lanes / l;
        let pairs = s.pairs.as_ptr();
        let mut v = 0;
        while v + 4 <= nvec {
            let off = v * l;
            for o in 0..s.n {
                let (a, b) = (*s.start.get_unchecked(o) as usize, *s.start.get_unchecked(o + 1) as usize);
                let mut a0 = V::splat(0.0);
                let mut a1 = a0;
                let mut a2 = a0;
                let mut a3 = a0;
                for k in a..b {
                    let p = &*pairs.add(k);
                    let c = V::splat(p.coef);
                    let pa = src.add(p.a as usize * ss + off);
                    let pb = src.add(p.b as usize * ss + off);
                    a0 = V::load(pa).add(V::load(pb)).fmadd(c, a0);
                    a1 = V::load(pa.add(l)).add(V::load(pb.add(l))).fmadd(c, a1);
                    a2 = V::load(pa.add(2 * l)).add(V::load(pb.add(2 * l))).fmadd(c, a2);
                    a3 = V::load(pa.add(3 * l)).add(V::load(pb.add(3 * l))).fmadd(c, a3);
                }
                let q = dst.add(o * ds + off);
                a0.store(q);
                a1.store(q.add(l));
                a2.store(q.add(2 * l));
                a3.store(q.add(3 * l));
            }
            v += 4;
        }
        while v < nvec {
            let off = v * l;
            for o in 0..s.n {
                let (a, b) = (*s.start.get_unchecked(o) as usize, *s.start.get_unchecked(o + 1) as usize);
                let mut acc = V::splat(0.0);
                for k in a..b {
                    let p = &*pairs.add(k);
                    let x = V::load(src.add(p.a as usize * ss + off)).add(V::load(src.add(p.b as usize * ss + off)));
                    acc = x.fmadd(V::splat(p.coef), acc);
                }
                acc.store(dst.add(o * ds + off));
            }
            v += 1;
        }
    }
}

/// Lanes per column strip.
const STRIP: usize = 64;

/// One pass along the columns of a `cols`-wide region whose first sample is at `src` and
/// `dst` (row stride `stride`): column c is a line, sample i at row i.
#[inline(always)]
pub unsafe fn column_pass<V: Vf32>(s: &Sched, src: *const f32, dst: *mut f32, stride: usize, cols: usize) {
    unsafe {
        let l = V::LANES;
        let full = cols / l * l;
        let mut c = 0;
        while c < full {
            let n = STRIP.min(full - c);
            apply::<V>(s, src.add(c), stride, dst.add(c), stride, n);
            c += n;
        }
        if full < cols {
            apply::<Scalar>(s, src.add(full), stride, dst.add(full), stride, cols - full);
        }
    }
}

/// Row blocks transposed together in a row pass.
const ROW_BLOCKS: usize = 4;

/// One pass along the rows of a region of `rows` rows of `s.n()` samples: rows are transposed
/// in blocks into `scratch` so that each vector lane holds one row.
#[inline(always)]
pub unsafe fn row_pass<V: Vf32>(s: &Sched, src: *const f32, dst: *mut f32, stride: usize, rows: usize, scratch: &mut Vec<f32>) {
    unsafe {
        let l = V::LANES;
        let n = s.n();
        let max_lanes = ROW_BLOCKS * l;
        if scratch.len() < 2 * n * max_lanes {
            scratch.resize(2 * n * max_lanes, 0.0);
        }
        let (sin, sout) = scratch.split_at_mut(n * max_lanes);
        let (sin, sout) = (sin.as_mut_ptr(), sout.as_mut_ptr());
        let mut r = 0;
        while r + l <= rows {
            let kb = ROW_BLOCKS.min((rows - r) / l);
            let lanes = kb * l;
            for b in 0..kb {
                let rs = src.add((r + b * l) * stride);
                let mut c = 0;
                while c + l <= n {
                    V::transpose(rs.add(c), stride, sin.add(c * lanes + b * l), lanes);
                    c += l;
                }
                for cc in c..n {
                    for lane in 0..l {
                        *sin.add(cc * lanes + b * l + lane) = *rs.add(lane * stride + cc);
                    }
                }
            }
            apply::<V>(s, sin, lanes, sout, lanes, lanes);
            for b in 0..kb {
                let rd = dst.add((r + b * l) * stride);
                let mut c = 0;
                while c + l <= n {
                    V::transpose(sout.add(c * lanes + b * l), lanes, rd.add(c), stride);
                    c += l;
                }
                for cc in c..n {
                    for lane in 0..l {
                        *rd.add(lane * stride + cc) = *sout.add(cc * lanes + b * l + lane);
                    }
                }
            }
            r += lanes;
        }
        while r < rows {
            apply::<Scalar>(s, src.add(r * stride), 1, dst.add(r * stride), 1, 1);
            r += 1;
        }
    }
}

/// Convert coefficients back to pixels: `v * r + m`, rounded and clamped to [0, 255]. In exact
/// mode the operations are those of NBIS (`conv_img_2_uchar`): multiply, add, add one half,
/// truncate; adding 0.5 in float gives the same result as NBIS's double addition rounded to
/// float, because the double sum of a float and one half is exact.
#[inline(always)]
pub unsafe fn to_pixels<V: Vf32>(src: *const f32, dst: *mut u8, n: usize, r: f32, m: f32, exact: bool) {
    unsafe {
        let l = V::LANES;
        let (vr, vm, half, lo, hi, mh) = (V::splat(r), V::splat(m), V::splat(0.5), V::splat(0.0), V::splat(255.0), V::splat(m + 0.5));
        let mut i = 0;
        while i + l <= n {
            let v = V::load(src.add(i));
            let t = if exact { v.mul(vr).add(vm).add(half) } else { v.fmadd(vr, mh) };
            t.max(lo).min(hi).store_u8(dst.add(i));
            i += l;
        }
        while i < n {
            let v = *src.add(i);
            *dst.add(i) = if exact { crate::nbis::to_pixel(v, r, m) } else { (v * r + (m + 0.5)).clamp(0.0, 255.0) as u8 };
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Row-direct synthesis: vectors run along the row, over consecutive outputs of equal parity
// ---------------------------------------------------------------------------------------------

/// Terms of an output relative to its half-index m (output 2m + parity): `x[m + d] * c` in
/// order (exact), or `(x[m + da] + x[m + db]) * c` with fused accumulation (folded).
#[derive(Debug, Clone, PartialEq)]
pub enum Template {
    Exact(Vec<(i32, f32)>),
    Folded(Vec<(i32, i32, f32)>),
}

/// A synthesis schedule prepared for row-direct evaluation: for m in [m0, m1), outputs 2m and
/// 2m + 1 follow `even` and `odd` with indices shifted by m. Every other output is evaluated
/// from the schedule itself.
#[derive(Debug, Clone)]
pub struct RowPlan {
    pub sched: Sched,
    pub m0: usize,
    pub m1: usize,
    pub even: Template,
    pub odd: Template,
}

fn template(s: &Sched, o: usize, m: i64) -> Template {
    match s {
        Sched::Exact(e) => Template::Exact(
            e.terms[e.start[o] as usize..e.start[o + 1] as usize].iter().map(|t| ((t.idx as i64 - m) as i32, t.coef)).collect(),
        ),
        Sched::Folded(f) => Template::Folded(
            f.pairs[f.start[o] as usize..f.start[o + 1] as usize]
                .iter()
                .map(|p| ((p.a as i64 - m) as i32, (p.b as i64 - m) as i32, p.coef))
                .collect(),
        ),
    }
}

fn same(a: &Template, b: &Template) -> bool {
    match (a, b) {
        (Template::Exact(x), Template::Exact(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.0 == q.0 && p.1.to_bits() == q.1.to_bits()),
        (Template::Folded(x), Template::Folded(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.0 == q.0 && p.1 == q.1 && p.2.to_bits() == q.2.to_bits())
        }
        _ => false,
    }
}

/// Longest run of consecutive m with the same template for outputs 2m + parity.
fn longest_run(s: &Sched, parity: usize) -> (usize, usize, Template) {
    let n = s.n();
    let count = (n + 1 - parity) / 2; // m with 2m + parity < n
    let mut best = (0, 0, Template::Exact(Vec::new()));
    let mut m = 0;
    while m < count {
        let t = template(s, 2 * m + parity, m as i64);
        let mut e = m + 1;
        while e < count && same(&template(s, 2 * e + parity, e as i64), &t) {
            e += 1;
        }
        if e - m > best.1 - best.0 {
            best = (m, e, t);
        }
        m = e;
    }
    best
}

impl RowPlan {
    pub fn new(sched: Sched) -> RowPlan {
        let (e0, e1, even) = longest_run(&sched, 0);
        let (o0, o1, odd) = longest_run(&sched, 1);
        let (m0, m1) = (e0.max(o0), e1.min(o1));
        let (m0, m1) = if m1 > m0 { (m0, m1) } else { (0, 0) };
        RowPlan { sched, m0, m1, even, odd }
    }
}

#[inline(always)]
unsafe fn eval_template<V: Vf32>(t: &Template, x: *const f32) -> V {
    unsafe {
        match t {
            Template::Exact(terms) => {
                let mut acc = V::splat(0.0);
                if let Some((&(d0, c0), rest)) = terms.split_first() {
                    acc = V::load(x.offset(d0 as isize)).mul(V::splat(c0));
                    for &(d, c) in rest {
                        acc = acc.add(V::load(x.offset(d as isize)).mul(V::splat(c)));
                    }
                }
                acc
            }
            Template::Folded(pairs) => {
                let mut acc = V::splat(0.0);
                for &(da, db, c) in pairs {
                    acc = V::load(x.offset(da as isize)).add(V::load(x.offset(db as isize))).fmadd(V::splat(c), acc);
                }
                acc
            }
        }
    }
}

/// K vectors of a template at x, x + L, ..., x + (K - 1) L: the template is read once and the K
/// accumulations run as independent dependency chains.
#[inline(always)]
unsafe fn eval_template_k<V: Vf32, const K: usize>(t: &Template, x: *const f32) -> [V; K] {
    unsafe {
        let l = V::LANES;
        let mut acc = [V::splat(0.0); K];
        match t {
            Template::Exact(terms) => {
                if let Some((&(d0, c0), rest)) = terms.split_first() {
                    let c = V::splat(c0);
                    let p = x.offset(d0 as isize);
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a = V::load(p.add(k * l)).mul(c);
                    }
                    for &(d, c) in rest {
                        let c = V::splat(c);
                        let p = x.offset(d as isize);
                        for (k, a) in acc.iter_mut().enumerate() {
                            *a = a.add(V::load(p.add(k * l)).mul(c));
                        }
                    }
                }
            }
            Template::Folded(pairs) => {
                for &(da, db, c) in pairs {
                    let c = V::splat(c);
                    let (pa, pb) = (x.offset(da as isize), x.offset(db as isize));
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a = V::load(pa.add(k * l)).add(V::load(pb.add(k * l))).fmadd(c, *a);
                    }
                }
            }
        }
        acc
    }
}

/// Evaluate output `o` of the schedule on row `x`, with the scalar arithmetic of the backend
/// family (mul and add in exact mode).
#[inline(always)]
unsafe fn scalar_output(s: &Sched, x: *const f32, o: usize) -> f32 {
    unsafe {
        match s {
            Sched::Exact(e) => {
                let (a, b) = (e.start[o] as usize, e.start[o + 1] as usize);
                if a == b {
                    return 0.0;
                }
                let t = e.terms.get_unchecked(a);
                let mut acc = *x.add(t.idx as usize) * t.coef;
                for t in e.terms.get_unchecked(a + 1..b) {
                    acc += *x.add(t.idx as usize) * t.coef;
                }
                acc
            }
            Sched::Folded(f) => {
                let mut acc = 0f32;
                for p in f.pairs.get_unchecked(f.start[o] as usize..f.start[o + 1] as usize) {
                    acc += (*x.add(p.a as usize) + *x.add(p.b as usize)) * p.coef;
                }
                acc
            }
        }
    }
}

/// Synthesis of `rows` rows of a region: row r is read at `src + r * stride` and written at
/// `dst + r * stride`.
#[inline(always)]
pub unsafe fn row_direct<V: Vf32>(p: &RowPlan, src: *const f32, dst: *mut f32, stride: usize, rows: usize) {
    unsafe {
        let n = p.sched.n();
        let l = V::LANES;
        let vec_end = p.m0 + (p.m1 - p.m0) / l * l;
        let (lo_end, hi_start) = if vec_end > p.m0 { (2 * p.m0, 2 * vec_end) } else { (0, 0) };
        for r in 0..rows {
            let x = src.add(r * stride);
            let y = dst.add(r * stride);
            for o in 0..lo_end {
                *y.add(o) = scalar_output(&p.sched, x, o);
            }
            let mut m = p.m0;
            while m + 4 * l <= vec_end {
                let e: [V; 4] = eval_template_k::<V, 4>(&p.even, x.add(m));
                let od: [V; 4] = eval_template_k::<V, 4>(&p.odd, x.add(m));
                for k in 0..4 {
                    let (a, b) = V::interleave(e[k], od[k]);
                    a.store(y.add(2 * (m + k * l)));
                    b.store(y.add(2 * (m + k * l) + l));
                }
                m += 4 * l;
            }
            while m < vec_end {
                let e: V = eval_template::<V>(&p.even, x.add(m));
                let od: V = eval_template::<V>(&p.odd, x.add(m));
                let (a, b) = V::interleave(e, od);
                a.store(y.add(2 * m));
                b.store(y.add(2 * m + l));
                m += l;
            }
            for o in hi_start.max(lo_end)..n {
                *y.add(o) = scalar_output(&p.sched, x, o);
            }
        }
    }
}


/// Templates with row offsets scaled to a stride, for column-direct evaluation.
#[derive(Debug, Clone)]
enum Strided {
    Exact(Vec<(isize, f32)>),
    Folded(Vec<(isize, isize, f32)>),
}

fn strided(t: &Template, stride: usize) -> Strided {
    let s = stride as isize;
    match t {
        Template::Exact(v) => Strided::Exact(v.iter().map(|&(d, c)| (d as isize * s, c)).collect()),
        Template::Folded(v) => Strided::Folded(v.iter().map(|&(a, b, c)| (a as isize * s, b as isize * s, c)).collect()),
    }
}

/// G vectors (G * LANES adjacent columns) of a strided template at x.
#[inline(always)]
unsafe fn eval_strided<V: Vf32, const G: usize>(t: &Strided, x: *const f32) -> [V; G] {
    unsafe {
        let l = V::LANES;
        let mut acc = [V::splat(0.0); G];
        match t {
            Strided::Exact(terms) => {
                if let Some((&(d0, c0), rest)) = terms.split_first() {
                    let c = V::splat(c0);
                    let p = x.offset(d0);
                    for (g, a) in acc.iter_mut().enumerate() {
                        *a = V::load(p.add(g * l)).mul(c);
                    }
                    for &(d, c) in rest {
                        let c = V::splat(c);
                        let p = x.offset(d);
                        for (g, a) in acc.iter_mut().enumerate() {
                            *a = a.add(V::load(p.add(g * l)).mul(c));
                        }
                    }
                }
            }
            Strided::Folded(pairs) => {
                for &(da, db, c) in pairs {
                    let c = V::splat(c);
                    let (pa, pb) = (x.offset(da), x.offset(db));
                    for (g, a) in acc.iter_mut().enumerate() {
                        *a = V::load(pa.add(g * l)).add(V::load(pb.add(g * l))).fmadd(c, *a);
                    }
                }
            }
        }
        acc
    }
}

/// Column pass with the interior evaluated by templates: outputs 2m and 2m + 1 of the same
/// column strip are computed together, which doubles the independent accumulations.
#[inline(always)]
pub unsafe fn column_direct<V: Vf32>(p: &RowPlan, src: *const f32, dst: *mut f32, stride: usize, cols: usize) {
    unsafe {
        const G: usize = 4;
        let l = V::LANES;
        let n = p.sched.n();
        let (lo_end, hi_start) = if p.m1 > p.m0 { (2 * p.m0, 2 * p.m1) } else { (n, n) };
        let even = strided(&p.even, stride);
        let odd = strided(&p.odd, stride);
        let block = G * l;
        let full = cols / block * block;
        // boundary outputs (and everything when there is no interior) through the schedule
        let edge = |c0: usize, ncols: usize, from: usize, to: usize| {
            if from < to {
                let sub = crate::kernels::sub_schedule(&p.sched, from, to);
                apply::<V>(&sub, src.add(c0), stride, dst.add(from * stride + c0), stride, ncols / l * l);
                let rem = ncols % l;
                if rem > 0 {
                    let c1 = c0 + ncols - rem;
                    apply::<Scalar>(&sub, src.add(c1), stride, dst.add(from * stride + c1), stride, rem);
                }
            }
        };
        edge(0, cols, 0, lo_end);
        edge(0, cols, hi_start.max(lo_end), n);
        let mut c = 0;
        while c < full {
            for m in p.m0..p.m1 {
                let x = src.add(m * stride + c);
                let e: [V; G] = eval_strided::<V, G>(&even, x);
                let o: [V; G] = eval_strided::<V, G>(&odd, x);
                let ye = dst.add(2 * m * stride + c);
                let yo = ye.add(stride);
                for g in 0..G {
                    e[g].store(ye.add(g * l));
                    o[g].store(yo.add(g * l));
                }
            }
            c += block;
        }
        if full < cols && lo_end < hi_start {
            // leftover columns of the interior: whole vectors, then scalar lanes
            let sub = crate::kernels::sub_schedule(&p.sched, lo_end, hi_start);
            let rem = cols - full;
            apply::<V>(&sub, src.add(full), stride, dst.add(lo_end * stride + full), stride, rem / l * l);
            if !rem.is_multiple_of(l) {
                let c1 = full + rem / l * l;
                apply::<Scalar>(&sub, src.add(c1), stride, dst.add(lo_end * stride + c1), stride, rem % l);
            }
        }
    }
}

/// Outputs `from..to` of a schedule as a schedule of its own (indices unchanged).
pub fn sub_schedule(s: &Sched, from: usize, to: usize) -> Sched {
    match s {
        Sched::Exact(e) => {
            let base = e.start[from];
            Sched::Exact(crate::schedule::ExactSchedule {
                n: to - from,
                start: e.start[from..=to].iter().map(|&v| v - base).collect(),
                terms: e.terms[base as usize..e.start[to] as usize].to_vec(),
            })
        }
        Sched::Folded(f) => {
            let base = f.start[from];
            Sched::Folded(crate::schedule::FoldedSchedule {
                n: to - from,
                start: f.start[from..=to].iter().map(|&v| v - base).collect(),
                pairs: f.pairs[base as usize..f.start[to] as usize].to_vec(),
            })
        }
    }
}

/// Dequantize one row of indices (stored as floats) into coefficients:
/// `(p - sign(p) C) Q + sign(p) Z / 2`, and 0 where p is 0 (Equation 9 of the algorithm
/// document). In exact mode the multiply and the add stay separate; the double addition NBIS
/// performs gives the same float because the two terms differ by fewer than 29 binary orders.
#[inline(always)]
pub unsafe fn dequant_row<V: Vf32>(p: *const f32, out: *mut f32, n: usize, q: f32, zh: f32, c: f32, exact: bool) {
    unsafe {
        let l = V::LANES;
        let (vq, vzh, vc) = (V::splat(q), V::splat(zh), V::splat(c));
        let mut i = 0;
        while i + l <= n {
            let x = V::load(p.add(i));
            let t = x.sub(vc.copysign(x));
            let v = if exact { t.mul(vq).add(vzh.copysign(x)) } else { t.fmadd(vq, vzh.copysign(x)) };
            v.where_nonzero(x).store(out.add(i));
            i += l;
        }
        while i < n {
            let x = *p.add(i);
            *out.add(i) = if x == 0.0 { 0.0 } else { (x - c.copysign(x)) * q + zh.copysign(x) };
            i += 1;
        }
    }
}

/// Quantize one row of coefficients (Equation 8 of the algorithm document), writing the indices
/// as floats. NBIS computes `(short)((v - Z/2) / Q + 1.0)` in double; for a positive quotient
/// t that equals trunc(t) + 1 (and trunc(t) - 1 for a negative one), so the float division and
/// a float truncation give the same indices. Valid for indices below 2^24.
#[inline(always)]
pub unsafe fn quantize_row<V: Vf32>(src: *const f32, out: *mut f32, n: usize, q: f32, zbin: f32) -> f32 {
    unsafe {
        let l = V::LANES;
        let (vq, vz, one) = (V::splat(q), V::splat(zbin), V::splat(1.0));
        let mut peak = V::splat(0.0);
        let mut i = 0;
        while i + l <= n {
            let v = V::load(src.add(i));
            // (v - Z/2) / Q for v > 0, (v + Z/2) / Q for v < 0: one division
            let t = v.sub(vz.copysign(v)).div(vq).trunc();
            let r = t.add(one.copysign(v));
            let a = v.max(V::splat(0.0).sub(v)); // |v|
            let r = a.select_gt(vz, r, V::splat(0.0));
            peak = peak.max(r.max(V::splat(0.0).sub(r)));
            r.store(out.add(i));
            i += l;
        }
        let mut buf = [0f32; 16];
        peak.store(buf.as_mut_ptr());
        let mut pk = buf[..l].iter().fold(0f32, |m, &x| m.max(x));
        while i < n {
            let p = crate::nbis::quantize(*src.add(i), q, zbin) as f32;
            pk = pk.max(p.abs());
            *out.add(i) = p;
            i += 1;
        }
        pk
    }
}

/// Normalize pixels: `(p - m) / r`, as NBIS does (float subtract, float divide).
#[inline(always)]
pub unsafe fn normalize_row<V: Vf32>(src: *const u8, out: *mut f32, n: usize, m: f32, r: f32) {
    unsafe {
        let l = V::LANES;
        let (vm, vr) = (V::splat(m), V::splat(r));
        let mut i = 0;
        while i + l <= n {
            V::from_u8(src.add(i)).sub(vm).div(vr).store(out.add(i));
            i += l;
        }
        while i < n {
            *out.add(i) = (*src.add(i) as f32 - m) / r;
            i += 1;
        }
    }
}

/// Turn a block of indices (stored as floats) into symbols (Algorithm 6 of the algorithm
/// document), skipping zeros a vector at a time.
#[inline(always)]
pub unsafe fn tokenize<V: Vf32>(v: *const f32, n: usize, out: &mut Vec<(u8, u16, u8)>) {
    unsafe {
        let l = V::LANES;
        let flush = |r: u32, out: &mut Vec<(u8, u16, u8)>| {
            if r <= 100 {
                out.push((r as u8, 0, 0));
            } else if r <= 0xFF {
                out.push((105, r as u16, 8));
            } else {
                out.push((106, r as u16, 16));
            }
        };
        let mut run: u32 = 0;
        let mut i = 0;
        while i < n {
            // next nonzero at or after i
            let mut j = i;
            loop {
                if j + l <= n {
                    let bits = V::load(v.add(j)).nonzero_bits();
                    if bits != 0 {
                        j += bits.trailing_zeros() as usize;
                        break;
                    }
                    j += l;
                } else {
                    while j < n && *v.add(j) == 0.0 {
                        j += 1;
                    }
                    break;
                }
            }
            let mut zeros = (j - i) as u32;
            while zeros > 0 {
                let take = zeros.min(0xFFFF - run);
                run += take;
                zeros -= take;
                if run == 0xFFFF {
                    flush(run, out);
                    run = 0;
                }
            }
            if j >= n {
                break;
            }
            if run > 0 {
                flush(run, out);
                run = 0;
            }
            let x = *v.add(j) as i32;
            if (-73..=74).contains(&x) {
                out.push(((x + 180) as u8, 0, 0));
            } else if x > 0 {
                out.push(if x <= 0xFF { (101, x as u16, 8) } else { (103, x as u16, 16) });
            } else if -x <= 0xFF {
                out.push((102, (-x) as u16, 8));
            } else {
                out.push((104, (-x) as u16, 16));
            }
            i = j + 1;
        }
        if run > 0 {
            flush(run, out);
        }
    }
}
