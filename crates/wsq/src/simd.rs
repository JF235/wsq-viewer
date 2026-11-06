//! Backends: the vector operations the kernels are written in, one implementation per
//! instruction set. Kernels are generic over [`Vf32`]; [`Backend`] picks the implementation at
//! run time.
//!
//! # Safety
//!
//! Every method of [`Vf32`] is unsafe for the same two reasons: the caller must run on a CPU
//! with the instruction set of the implementation (which [`Backend::is_available`] checks), and
//! pointers must be valid for `LANES` elements (for `transpose`, `LANES` rows of `LANES`).
#![allow(clippy::missing_safety_doc)]

/// A vector of `LANES` float32 values and the handful of operations the kernels need.
///
/// `mul` and `add` are always separate IEEE operations, so a kernel that uses only them gives
/// bit-identical results on every backend. `fmadd` is fused where the hardware has FMA and is
/// used only by the fast kernels.
pub trait Vf32: Copy {
    const LANES: usize;
    unsafe fn splat(x: f32) -> Self;
    unsafe fn load(p: *const f32) -> Self;
    unsafe fn store(self, p: *mut f32);
    unsafe fn add(self, b: Self) -> Self;
    unsafe fn sub(self, b: Self) -> Self;
    unsafe fn mul(self, b: Self) -> Self;
    /// `self * b + c`, fused if the backend can.
    unsafe fn fmadd(self, b: Self, c: Self) -> Self;
    unsafe fn min(self, b: Self) -> Self;
    unsafe fn max(self, b: Self) -> Self;
    /// Truncate lanes holding values in [0, 255] and store them as bytes.
    unsafe fn store_u8(self, p: *mut u8);
    /// Interleave two vectors: (a0 b0 a1 b1 ...), returned as the first and second half.
    unsafe fn interleave(a: Self, b: Self) -> (Self, Self);
    /// Magnitude of `self` with the sign of `sign`.
    unsafe fn copysign(self, sign: Self) -> Self;
    /// `self` where `test` is nonzero, 0 where it is zero.
    unsafe fn where_nonzero(self, test: Self) -> Self;
    unsafe fn div(self, b: Self) -> Self;
    /// Round toward zero.
    unsafe fn trunc(self) -> Self;
    /// Lanes of `a` where `self > b`, of `c` elsewhere.
    unsafe fn select_gt(self, b: Self, a: Self, c: Self) -> Self;
    /// LANES bytes converted to floats.
    unsafe fn from_u8(p: *const u8) -> Self;
    /// Bit i set when lane i is nonzero.
    unsafe fn nonzero_bits(self) -> u32;
    /// Transpose a LANES x LANES block: row `i` is read at `src + i * ss`, column `i` is written
    /// as a row at `dst + i * ds`.
    unsafe fn transpose(src: *const f32, ss: usize, dst: *mut f32, ds: usize);
}

/// Instruction set used by the transform kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Scalar,
    Sse2,
    Avx2,
    Avx512,
    Simd128,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Scalar => "scalar",
            Backend::Sse2 => "sse2",
            Backend::Avx2 => "avx2",
            Backend::Avx512 => "avx512",
            Backend::Simd128 => "simd128",
        }
    }

    pub fn from_name(s: &str) -> Option<Backend> {
        [Backend::Scalar, Backend::Sse2, Backend::Avx2, Backend::Avx512, Backend::Simd128]
            .into_iter()
            .find(|b| b.name() == s)
    }

    pub fn lanes(self) -> usize {
        match self {
            Backend::Scalar => 1,
            Backend::Sse2 | Backend::Simd128 => 4,
            Backend::Avx2 => 8,
            Backend::Avx512 => 16,
        }
    }

    /// Whether this machine (or this build, for wasm) supports the backend.
    pub fn is_available(self) -> bool {
        match self {
            Backend::Scalar => true,
            #[cfg(target_arch = "x86_64")]
            Backend::Sse2 => true,
            #[cfg(target_arch = "x86_64")]
            Backend::Avx2 => is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma"),
            #[cfg(target_arch = "x86_64")]
            Backend::Avx512 => is_x86_feature_detected!("avx512f"),
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            Backend::Simd128 => true,
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    /// Every backend this machine supports, from the simplest to the widest.
    pub fn available() -> Vec<Backend> {
        [Backend::Scalar, Backend::Sse2, Backend::Simd128, Backend::Avx2, Backend::Avx512]
            .into_iter()
            .filter(|b| b.is_available())
            .collect()
    }

    /// The widest backend available.
    pub fn best() -> Backend {
        *Backend::available().last().unwrap()
    }
}

// ---------------------------------------------------------------------------------------------
// Scalar
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Scalar(f32);

impl Vf32 for Scalar {
    const LANES: usize = 1;
    #[inline(always)]
    unsafe fn splat(x: f32) -> Self {
        Scalar(x)
    }
    #[inline(always)]
    unsafe fn load(p: *const f32) -> Self {
        Scalar(unsafe { *p })
    }
    #[inline(always)]
    unsafe fn store(self, p: *mut f32) {
        unsafe { *p = self.0 }
    }
    #[inline(always)]
    unsafe fn add(self, b: Self) -> Self {
        Scalar(self.0 + b.0)
    }
    #[inline(always)]
    unsafe fn sub(self, b: Self) -> Self {
        Scalar(self.0 - b.0)
    }
    #[inline(always)]
    unsafe fn mul(self, b: Self) -> Self {
        Scalar(self.0 * b.0)
    }
    #[inline(always)]
    unsafe fn fmadd(self, b: Self, c: Self) -> Self {
        Scalar(self.0 * b.0 + c.0)
    }
    #[inline(always)]
    unsafe fn min(self, b: Self) -> Self {
        Scalar(if self.0 < b.0 { self.0 } else { b.0 })
    }
    #[inline(always)]
    unsafe fn max(self, b: Self) -> Self {
        Scalar(if self.0 > b.0 { self.0 } else { b.0 })
    }
    #[inline(always)]
    unsafe fn store_u8(self, p: *mut u8) {
        unsafe { *p = self.0 as u8 }
    }
    #[inline(always)]
    unsafe fn interleave(a: Self, b: Self) -> (Self, Self) {
        (a, b)
    }
    #[inline(always)]
    unsafe fn copysign(self, sign: Self) -> Self {
        Scalar(self.0.copysign(sign.0))
    }
    #[inline(always)]
    unsafe fn where_nonzero(self, test: Self) -> Self {
        Scalar(if test.0 != 0.0 { self.0 } else { 0.0 })
    }
    #[inline(always)]
    unsafe fn div(self, b: Self) -> Self {
        Scalar(self.0 / b.0)
    }
    #[inline(always)]
    unsafe fn trunc(self) -> Self {
        Scalar(self.0.trunc())
    }
    #[inline(always)]
    unsafe fn select_gt(self, b: Self, a: Self, c: Self) -> Self {
        if self.0 > b.0 { a } else { c }
    }
    #[inline(always)]
    unsafe fn from_u8(p: *const u8) -> Self {
        Scalar(unsafe { *p } as f32)
    }
    #[inline(always)]
    unsafe fn nonzero_bits(self) -> u32 {
        (self.0 != 0.0) as u32
    }
    #[inline(always)]
    unsafe fn transpose(src: *const f32, _ss: usize, dst: *mut f32, _ds: usize) {
        unsafe { *dst = *src }
    }
}

// ---------------------------------------------------------------------------------------------
// x86-64: SSE2, AVX2 + FMA, AVX-512F
// ---------------------------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
pub use x86::{Avx2, Avx512, Sse};

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::Vf32;
    use std::arch::x86_64::*;

    #[derive(Clone, Copy)]
    pub struct Sse(__m128);

    impl Vf32 for Sse {
        const LANES: usize = 4;
        #[inline(always)]
        unsafe fn splat(x: f32) -> Self {
            Sse(unsafe { _mm_set1_ps(x) })
        }
        #[inline(always)]
        unsafe fn load(p: *const f32) -> Self {
            Sse(unsafe { _mm_loadu_ps(p) })
        }
        #[inline(always)]
        unsafe fn store(self, p: *mut f32) {
            unsafe { _mm_storeu_ps(p, self.0) }
        }
        #[inline(always)]
        unsafe fn add(self, b: Self) -> Self {
            Sse(unsafe { _mm_add_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn sub(self, b: Self) -> Self {
            Sse(unsafe { _mm_sub_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn mul(self, b: Self) -> Self {
            Sse(unsafe { _mm_mul_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn fmadd(self, b: Self, c: Self) -> Self {
            Sse(unsafe { _mm_add_ps(_mm_mul_ps(self.0, b.0), c.0) })
        }
        #[inline(always)]
        unsafe fn min(self, b: Self) -> Self {
            Sse(unsafe { _mm_min_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn max(self, b: Self) -> Self {
            Sse(unsafe { _mm_max_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn store_u8(self, p: *mut u8) {
            unsafe {
                let i = _mm_cvttps_epi32(self.0);
                let w = _mm_packs_epi32(i, i);
                let b = _mm_packus_epi16(w, w);
                (p as *mut i32).write_unaligned(_mm_cvtsi128_si32(b));
            }
        }
        #[inline(always)]
        unsafe fn interleave(a: Self, b: Self) -> (Self, Self) {
            unsafe { (Sse(_mm_unpacklo_ps(a.0, b.0)), Sse(_mm_unpackhi_ps(a.0, b.0))) }
        }
        #[inline(always)]
        unsafe fn copysign(self, sign: Self) -> Self {
            unsafe {
                let m = _mm_set1_ps(-0.0);
                Sse(_mm_or_ps(_mm_andnot_ps(m, self.0), _mm_and_ps(m, sign.0)))
            }
        }
        #[inline(always)]
        unsafe fn where_nonzero(self, test: Self) -> Self {
            unsafe { Sse(_mm_and_ps(self.0, _mm_cmpneq_ps(test.0, _mm_setzero_ps()))) }
        }
        #[inline(always)]
        unsafe fn div(self, b: Self) -> Self {
            Sse(unsafe { _mm_div_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn trunc(self) -> Self {
            Sse(unsafe { _mm_cvtepi32_ps(_mm_cvttps_epi32(self.0)) })
        }
        #[inline(always)]
        unsafe fn select_gt(self, b: Self, a: Self, c: Self) -> Self {
            unsafe {
                let m = _mm_cmpgt_ps(self.0, b.0);
                Sse(_mm_or_ps(_mm_and_ps(m, a.0), _mm_andnot_ps(m, c.0)))
            }
        }
        #[inline(always)]
        unsafe fn from_u8(p: *const u8) -> Self {
            unsafe {
                let b = _mm_cvtsi32_si128((p as *const i32).read_unaligned());
                let z = _mm_setzero_si128();
                Sse(_mm_cvtepi32_ps(_mm_unpacklo_epi16(_mm_unpacklo_epi8(b, z), z)))
            }
        }
        #[inline(always)]
        unsafe fn nonzero_bits(self) -> u32 {
            unsafe { (!_mm_movemask_ps(_mm_cmpeq_ps(self.0, _mm_setzero_ps())) & 0xF) as u32 }
        }
        #[inline(always)]
        unsafe fn transpose(src: *const f32, ss: usize, dst: *mut f32, ds: usize) {
            unsafe {
                let r0 = _mm_loadu_ps(src);
                let r1 = _mm_loadu_ps(src.add(ss));
                let r2 = _mm_loadu_ps(src.add(2 * ss));
                let r3 = _mm_loadu_ps(src.add(3 * ss));
                let t0 = _mm_unpacklo_ps(r0, r1);
                let t1 = _mm_unpacklo_ps(r2, r3);
                let t2 = _mm_unpackhi_ps(r0, r1);
                let t3 = _mm_unpackhi_ps(r2, r3);
                _mm_storeu_ps(dst, _mm_movelh_ps(t0, t1));
                _mm_storeu_ps(dst.add(ds), _mm_movehl_ps(t1, t0));
                _mm_storeu_ps(dst.add(2 * ds), _mm_movelh_ps(t2, t3));
                _mm_storeu_ps(dst.add(3 * ds), _mm_movehl_ps(t3, t2));
            }
        }
    }

    #[derive(Clone, Copy)]
    pub struct Avx2(__m256);

    impl Vf32 for Avx2 {
        const LANES: usize = 8;
        #[inline(always)]
        unsafe fn splat(x: f32) -> Self {
            Avx2(unsafe { _mm256_set1_ps(x) })
        }
        #[inline(always)]
        unsafe fn load(p: *const f32) -> Self {
            Avx2(unsafe { _mm256_loadu_ps(p) })
        }
        #[inline(always)]
        unsafe fn store(self, p: *mut f32) {
            unsafe { _mm256_storeu_ps(p, self.0) }
        }
        #[inline(always)]
        unsafe fn add(self, b: Self) -> Self {
            Avx2(unsafe { _mm256_add_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn sub(self, b: Self) -> Self {
            Avx2(unsafe { _mm256_sub_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn mul(self, b: Self) -> Self {
            Avx2(unsafe { _mm256_mul_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn fmadd(self, b: Self, c: Self) -> Self {
            Avx2(unsafe { _mm256_fmadd_ps(self.0, b.0, c.0) })
        }
        #[inline(always)]
        unsafe fn min(self, b: Self) -> Self {
            Avx2(unsafe { _mm256_min_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn max(self, b: Self) -> Self {
            Avx2(unsafe { _mm256_max_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn store_u8(self, p: *mut u8) {
            unsafe {
                let i = _mm256_cvttps_epi32(self.0);
                let w = _mm_packs_epi32(_mm256_castsi256_si128(i), _mm256_extracti128_si256::<1>(i));
                let b = _mm_packus_epi16(w, w);
                _mm_storel_epi64(p as *mut __m128i, b);
            }
        }
        #[inline(always)]
        unsafe fn interleave(a: Self, b: Self) -> (Self, Self) {
            unsafe {
                let lo = _mm256_unpacklo_ps(a.0, b.0);
                let hi = _mm256_unpackhi_ps(a.0, b.0);
                (Avx2(_mm256_permute2f128_ps::<0x20>(lo, hi)), Avx2(_mm256_permute2f128_ps::<0x31>(lo, hi)))
            }
        }
        #[inline(always)]
        unsafe fn copysign(self, sign: Self) -> Self {
            unsafe {
                let m = _mm256_set1_ps(-0.0);
                Avx2(_mm256_or_ps(_mm256_andnot_ps(m, self.0), _mm256_and_ps(m, sign.0)))
            }
        }
        #[inline(always)]
        unsafe fn where_nonzero(self, test: Self) -> Self {
            unsafe { Avx2(_mm256_and_ps(self.0, _mm256_cmp_ps::<_CMP_NEQ_UQ>(test.0, _mm256_setzero_ps()))) }
        }
        #[inline(always)]
        unsafe fn div(self, b: Self) -> Self {
            Avx2(unsafe { _mm256_div_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn trunc(self) -> Self {
            Avx2(unsafe { _mm256_round_ps::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(self.0) })
        }
        #[inline(always)]
        unsafe fn select_gt(self, b: Self, a: Self, c: Self) -> Self {
            unsafe { Avx2(_mm256_blendv_ps(c.0, a.0, _mm256_cmp_ps::<_CMP_GT_OQ>(self.0, b.0))) }
        }
        #[inline(always)]
        unsafe fn from_u8(p: *const u8) -> Self {
            unsafe { Avx2(_mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(_mm_loadl_epi64(p as *const __m128i)))) }
        }
        #[inline(always)]
        unsafe fn nonzero_bits(self) -> u32 {
            unsafe { (!_mm256_movemask_ps(_mm256_cmp_ps::<_CMP_EQ_OQ>(self.0, _mm256_setzero_ps())) & 0xFF) as u32 }
        }
        #[inline(always)]
        unsafe fn transpose(src: *const f32, ss: usize, dst: *mut f32, ds: usize) {
            unsafe {
                let r = |i: usize| _mm256_loadu_ps(src.add(i * ss));
                let (r0, r1, r2, r3, r4, r5, r6, r7) = (r(0), r(1), r(2), r(3), r(4), r(5), r(6), r(7));
                let t0 = _mm256_unpacklo_ps(r0, r1);
                let t1 = _mm256_unpackhi_ps(r0, r1);
                let t2 = _mm256_unpacklo_ps(r2, r3);
                let t3 = _mm256_unpackhi_ps(r2, r3);
                let t4 = _mm256_unpacklo_ps(r4, r5);
                let t5 = _mm256_unpackhi_ps(r4, r5);
                let t6 = _mm256_unpacklo_ps(r6, r7);
                let t7 = _mm256_unpackhi_ps(r6, r7);
                let u0 = _mm256_shuffle_ps::<0x44>(t0, t2);
                let u1 = _mm256_shuffle_ps::<0xEE>(t0, t2);
                let u2 = _mm256_shuffle_ps::<0x44>(t1, t3);
                let u3 = _mm256_shuffle_ps::<0xEE>(t1, t3);
                let u4 = _mm256_shuffle_ps::<0x44>(t4, t6);
                let u5 = _mm256_shuffle_ps::<0xEE>(t4, t6);
                let u6 = _mm256_shuffle_ps::<0x44>(t5, t7);
                let u7 = _mm256_shuffle_ps::<0xEE>(t5, t7);
                let w = |i: usize, v: __m256| _mm256_storeu_ps(dst.add(i * ds), v);
                w(0, _mm256_permute2f128_ps::<0x20>(u0, u4));
                w(1, _mm256_permute2f128_ps::<0x20>(u1, u5));
                w(2, _mm256_permute2f128_ps::<0x20>(u2, u6));
                w(3, _mm256_permute2f128_ps::<0x20>(u3, u7));
                w(4, _mm256_permute2f128_ps::<0x31>(u0, u4));
                w(5, _mm256_permute2f128_ps::<0x31>(u1, u5));
                w(6, _mm256_permute2f128_ps::<0x31>(u2, u6));
                w(7, _mm256_permute2f128_ps::<0x31>(u3, u7));
            }
        }
    }

    #[derive(Clone, Copy)]
    pub struct Avx512(__m512);

    impl Vf32 for Avx512 {
        const LANES: usize = 16;
        #[inline(always)]
        unsafe fn splat(x: f32) -> Self {
            Avx512(unsafe { _mm512_set1_ps(x) })
        }
        #[inline(always)]
        unsafe fn load(p: *const f32) -> Self {
            Avx512(unsafe { _mm512_loadu_ps(p) })
        }
        #[inline(always)]
        unsafe fn store(self, p: *mut f32) {
            unsafe { _mm512_storeu_ps(p, self.0) }
        }
        #[inline(always)]
        unsafe fn add(self, b: Self) -> Self {
            Avx512(unsafe { _mm512_add_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn sub(self, b: Self) -> Self {
            Avx512(unsafe { _mm512_sub_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn mul(self, b: Self) -> Self {
            Avx512(unsafe { _mm512_mul_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn fmadd(self, b: Self, c: Self) -> Self {
            Avx512(unsafe { _mm512_fmadd_ps(self.0, b.0, c.0) })
        }
        #[inline(always)]
        unsafe fn min(self, b: Self) -> Self {
            Avx512(unsafe { _mm512_min_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn max(self, b: Self) -> Self {
            Avx512(unsafe { _mm512_max_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn store_u8(self, p: *mut u8) {
            unsafe { _mm_storeu_si128(p as *mut __m128i, _mm512_cvtusepi32_epi8(_mm512_cvttps_epi32(self.0))) }
        }
        #[inline(always)]
        unsafe fn interleave(a: Self, b: Self) -> (Self, Self) {
            unsafe {
                let i0 = _mm512_setr_epi32(0, 16, 1, 17, 2, 18, 3, 19, 4, 20, 5, 21, 6, 22, 7, 23);
                let i1 = _mm512_setr_epi32(8, 24, 9, 25, 10, 26, 11, 27, 12, 28, 13, 29, 14, 30, 15, 31);
                (Avx512(_mm512_permutex2var_ps(a.0, i0, b.0)), Avx512(_mm512_permutex2var_ps(a.0, i1, b.0)))
            }
        }
        #[inline(always)]
        unsafe fn copysign(self, sign: Self) -> Self {
            unsafe {
                let m = _mm512_set1_epi32(i32::MIN);
                let mag = _mm512_andnot_si512(m, _mm512_castps_si512(self.0));
                let sg = _mm512_and_si512(m, _mm512_castps_si512(sign.0));
                Avx512(_mm512_castsi512_ps(_mm512_or_si512(mag, sg)))
            }
        }
        #[inline(always)]
        unsafe fn where_nonzero(self, test: Self) -> Self {
            unsafe { Avx512(_mm512_maskz_mov_ps(_mm512_cmp_ps_mask::<_CMP_NEQ_UQ>(test.0, _mm512_setzero_ps()), self.0)) }
        }
        #[inline(always)]
        unsafe fn div(self, b: Self) -> Self {
            Avx512(unsafe { _mm512_div_ps(self.0, b.0) })
        }
        #[inline(always)]
        unsafe fn trunc(self) -> Self {
            Avx512(unsafe { _mm512_roundscale_ps::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(self.0) })
        }
        #[inline(always)]
        unsafe fn select_gt(self, b: Self, a: Self, c: Self) -> Self {
            unsafe { Avx512(_mm512_mask_mov_ps(c.0, _mm512_cmp_ps_mask::<_CMP_GT_OQ>(self.0, b.0), a.0)) }
        }
        #[inline(always)]
        unsafe fn from_u8(p: *const u8) -> Self {
            unsafe { Avx512(_mm512_cvtepi32_ps(_mm512_cvtepu8_epi32(_mm_loadu_si128(p as *const __m128i)))) }
        }
        #[inline(always)]
        unsafe fn nonzero_bits(self) -> u32 {
            unsafe { _mm512_cmp_ps_mask::<_CMP_NEQ_UQ>(self.0, _mm512_setzero_ps()) as u32 }
        }
        #[inline(always)]
        unsafe fn transpose(src: *const f32, ss: usize, dst: *mut f32, ds: usize) {
            unsafe {
                let mut r = [_mm512_setzero_ps(); 16];
                for (i, v) in r.iter_mut().enumerate() {
                    *v = _mm512_loadu_ps(src.add(i * ss));
                }
                // 32-bit interleave within 128-bit lanes
                let mut t = [_mm512_setzero_ps(); 16];
                for i in 0..8 {
                    t[2 * i] = _mm512_unpacklo_ps(r[2 * i], r[2 * i + 1]);
                    t[2 * i + 1] = _mm512_unpackhi_ps(r[2 * i], r[2 * i + 1]);
                }
                // 64-bit interleave: u[4i+m] holds, in 128-bit lane j, column 4j+m of rows 4i..4i+3
                let mut u = [_mm512_setzero_ps(); 16];
                for i in 0..4 {
                    let pd = |v: __m512| _mm512_castps_pd(v);
                    u[4 * i] = _mm512_castpd_ps(_mm512_unpacklo_pd(pd(t[4 * i]), pd(t[4 * i + 2])));
                    u[4 * i + 1] = _mm512_castpd_ps(_mm512_unpackhi_pd(pd(t[4 * i]), pd(t[4 * i + 2])));
                    u[4 * i + 2] = _mm512_castpd_ps(_mm512_unpacklo_pd(pd(t[4 * i + 1]), pd(t[4 * i + 3])));
                    u[4 * i + 3] = _mm512_castpd_ps(_mm512_unpackhi_pd(pd(t[4 * i + 1]), pd(t[4 * i + 3])));
                }
                // gather the 128-bit lanes
                for m in 0..4 {
                    let v0 = _mm512_shuffle_f32x4::<0x88>(u[m], u[4 + m]);
                    let v1 = _mm512_shuffle_f32x4::<0xDD>(u[m], u[4 + m]);
                    let w0 = _mm512_shuffle_f32x4::<0x88>(u[8 + m], u[12 + m]);
                    let w1 = _mm512_shuffle_f32x4::<0xDD>(u[8 + m], u[12 + m]);
                    _mm512_storeu_ps(dst.add(m * ds), _mm512_shuffle_f32x4::<0x88>(v0, w0));
                    _mm512_storeu_ps(dst.add((4 + m) * ds), _mm512_shuffle_f32x4::<0x88>(v1, w1));
                    _mm512_storeu_ps(dst.add((8 + m) * ds), _mm512_shuffle_f32x4::<0xDD>(v0, w0));
                    _mm512_storeu_ps(dst.add((12 + m) * ds), _mm512_shuffle_f32x4::<0xDD>(v1, w1));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// WebAssembly SIMD
// ---------------------------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub use wasm::Wasm128;

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm {
    use super::Vf32;
    use std::arch::wasm32::*;

    #[derive(Clone, Copy)]
    pub struct Wasm128(v128);

    impl Vf32 for Wasm128 {
        const LANES: usize = 4;
        #[inline(always)]
        unsafe fn splat(x: f32) -> Self {
            Wasm128(f32x4_splat(x))
        }
        #[inline(always)]
        unsafe fn load(p: *const f32) -> Self {
            Wasm128(unsafe { v128_load(p as *const v128) })
        }
        #[inline(always)]
        unsafe fn store(self, p: *mut f32) {
            unsafe { v128_store(p as *mut v128, self.0) }
        }
        #[inline(always)]
        unsafe fn add(self, b: Self) -> Self {
            Wasm128(f32x4_add(self.0, b.0))
        }
        #[inline(always)]
        unsafe fn sub(self, b: Self) -> Self {
            Wasm128(f32x4_sub(self.0, b.0))
        }
        #[inline(always)]
        unsafe fn mul(self, b: Self) -> Self {
            Wasm128(f32x4_mul(self.0, b.0))
        }
        #[inline(always)]
        unsafe fn fmadd(self, b: Self, c: Self) -> Self {
            Wasm128(f32x4_add(f32x4_mul(self.0, b.0), c.0))
        }
        #[inline(always)]
        unsafe fn min(self, b: Self) -> Self {
            Wasm128(f32x4_pmin(self.0, b.0))
        }
        #[inline(always)]
        unsafe fn max(self, b: Self) -> Self {
            Wasm128(f32x4_pmax(self.0, b.0))
        }
        #[inline(always)]
        unsafe fn store_u8(self, p: *mut u8) {
            unsafe {
                let i = i32x4_trunc_sat_f32x4(self.0);
                let w = i16x8_narrow_i32x4(i, i);
                let b = u8x16_narrow_i16x8(w, w);
                v128_store32_lane::<0>(b, p as *mut u32);
            }
        }
        #[inline(always)]
        unsafe fn interleave(a: Self, b: Self) -> (Self, Self) {
            (Wasm128(i32x4_shuffle::<0, 4, 1, 5>(a.0, b.0)), Wasm128(i32x4_shuffle::<2, 6, 3, 7>(a.0, b.0)))
        }
        #[inline(always)]
        unsafe fn copysign(self, sign: Self) -> Self {
            let m = f32x4_splat(-0.0);
            Wasm128(v128_or(v128_andnot(self.0, m), v128_and(sign.0, m)))
        }
        #[inline(always)]
        unsafe fn where_nonzero(self, test: Self) -> Self {
            Wasm128(v128_and(self.0, f32x4_ne(test.0, f32x4_splat(0.0))))
        }
        #[inline(always)]
        unsafe fn div(self, b: Self) -> Self {
            Wasm128(f32x4_div(self.0, b.0))
        }
        #[inline(always)]
        unsafe fn trunc(self) -> Self {
            Wasm128(f32x4_trunc(self.0))
        }
        #[inline(always)]
        unsafe fn select_gt(self, b: Self, a: Self, c: Self) -> Self {
            Wasm128(v128_bitselect(a.0, c.0, f32x4_gt(self.0, b.0)))
        }
        #[inline(always)]
        unsafe fn from_u8(p: *const u8) -> Self {
            unsafe {
                let b = v128_load32_zero(p as *const u32);
                Wasm128(f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(b))))
            }
        }
        #[inline(always)]
        unsafe fn nonzero_bits(self) -> u32 {
            (!(i32x4_bitmask(f32x4_eq(self.0, f32x4_splat(0.0))) as u32)) & 0xF
        }
        #[inline(always)]
        unsafe fn transpose(src: *const f32, ss: usize, dst: *mut f32, ds: usize) {
            unsafe {
                let r0 = v128_load(src as *const v128);
                let r1 = v128_load(src.add(ss) as *const v128);
                let r2 = v128_load(src.add(2 * ss) as *const v128);
                let r3 = v128_load(src.add(3 * ss) as *const v128);
                let t0 = i32x4_shuffle::<0, 4, 1, 5>(r0, r1);
                let t1 = i32x4_shuffle::<0, 4, 1, 5>(r2, r3);
                let t2 = i32x4_shuffle::<2, 6, 3, 7>(r0, r1);
                let t3 = i32x4_shuffle::<2, 6, 3, 7>(r2, r3);
                v128_store(dst as *mut v128, i64x2_shuffle::<0, 2>(t0, t1));
                v128_store(dst.add(ds) as *mut v128, i64x2_shuffle::<1, 3>(t0, t1));
                v128_store(dst.add(2 * ds) as *mut v128, i64x2_shuffle::<0, 2>(t2, t3));
                v128_store(dst.add(3 * ds) as *mut v128, i64x2_shuffle::<1, 3>(t2, t3));
            }
        }
    }
}

/// Call `$generic::<V>(args)` with the vector type of `$backend`, inside a function compiled
/// for that instruction set so that the generic code is inlined with it.
#[macro_export]
#[doc(hidden)]
macro_rules! dispatch {
    ($backend:expr, $generic:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $ret:ty) => {{
        match $backend {
            #[cfg(target_arch = "x86_64")]
            $crate::simd::Backend::Sse2 => {
                #[target_feature(enable = "sse2")]
                unsafe fn run($($arg: $ty),*) -> $ret { unsafe { $generic::<$crate::simd::Sse>($($arg),*) } }
                unsafe { run($($arg),*) }
            }
            #[cfg(target_arch = "x86_64")]
            $crate::simd::Backend::Avx2 => {
                #[target_feature(enable = "avx2,fma")]
                unsafe fn run($($arg: $ty),*) -> $ret { unsafe { $generic::<$crate::simd::Avx2>($($arg),*) } }
                unsafe { run($($arg),*) }
            }
            #[cfg(target_arch = "x86_64")]
            $crate::simd::Backend::Avx512 => {
                #[target_feature(enable = "avx512f,avx2,fma")]
                unsafe fn run($($arg: $ty),*) -> $ret { unsafe { $generic::<$crate::simd::Avx512>($($arg),*) } }
                unsafe { run($($arg),*) }
            }
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            $crate::simd::Backend::Simd128 => unsafe { $generic::<$crate::simd::Wasm128>($($arg),*) },
            _ => unsafe { $generic::<$crate::simd::Scalar>($($arg),*) },
        }
    }};
}
