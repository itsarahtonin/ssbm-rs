// SPDX-License-Identifier: GPL-2.0-or-later
// Transcribes the x86-64 code that Slippi's netplay Dolphin (project-slippi/Ishiiruka at
// 60f7b63496fb6ec7b9180a04f16f3edc0ad89fe2) emits for float instructions: Jit_FloatingPoint.cpp,
// Jit_Paired.cpp and Jit_Util.cpp, with FMA3 available and accurate NaNs off, as when recording
// Slippi replays. Each instruction becomes the same SSE/AVX/FMA operations on the same lanes.

#include <immintrin.h>

#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>

namespace ishiiruka
{
using u32 = uint32_t;
using u64 = uint64_t;
using s64 = int64_t;

namespace MathUtil
{
#include "MathUtil_estimates.inc"
}  // namespace MathUtil

static __m128d reg(u64 ps0, u64 ps1)
{
  return _mm_castsi128_pd(_mm_set_epi64x(static_cast<long long>(ps1), static_cast<long long>(ps0)));
}

static u64 lo(__m128d v)
{
  return static_cast<u64>(_mm_cvtsi128_si64(_mm_castpd_si128(v)));
}

static u64 hi(__m128d v)
{
  return lo(_mm_unpackhi_pd(v, v));
}

static double as_double(u64 bits)
{
  double d;
  std::memcpy(&d, &bits, 8);
  return d;
}

static u64 as_bits(double d)
{
  u64 bits;
  std::memcpy(&bits, &d, 8);
  return bits;
}

// Force25BitPrecision: mantissa = (mantissa & ~0x7FFFFFF) + (mantissa & (1 << 27)).
static __m128d force25(__m128d v)
{
  const __m128i x = _mm_castpd_si128(v);
  const __m128i round = _mm_and_si128(x, _mm_set1_epi64x(0x8000000));
  const __m128i trunc = _mm_and_si128(x, _mm_set1_epi64x(static_cast<long long>(0xFFFFFFFFF8000000ULL)));
  return _mm_castsi128_pd(_mm_add_epi64(trunc, round));
}

// ForceSinglePrecision, packed: both lanes.
static __m128d single_packed(__m128d v)
{
  return _mm_cvtps_pd(_mm_cvtpd_ps(v));
}

// ForceSinglePrecision, scalar with duplicate: the low lane, copied to both.
static __m128d single_dup(__m128d v)
{
  const __m128 s = _mm_cvtsd_ss(_mm_setzero_ps(), v);
  return _mm_movedup_pd(_mm_cvtss_sd(_mm_setzero_pd(), s));
}

// MOVSD d, x: the low lane replaced, the high lane kept.
static __m128d movsd(__m128d d, __m128d x)
{
  return _mm_move_sd(d, x);
}

// CR field values: LT 8, GT 4, EQ 2, SO/unordered 1.
static u32 compare(double a, double b)
{
  if (std::isnan(a) || std::isnan(b))
    return 1;
  return a < b ? 8 : a > b ? 4 : 2;
}

static const __m128d sign_lo = reg(0x8000000000000000ULL, 0);
static const __m128d sign_both = reg(0x8000000000000000ULL, 0x8000000000000000ULL);
static const __m128d abs_lo = reg(0x7FFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL);
static const __m128d abs_both = reg(0x7FFFFFFFFFFFFFFFULL, 0x7FFFFFFFFFFFFFFFULL);

// fsel/ps_sel: (0 > a ? b : c), which is (a >= -0.0 ? c : b) with NaN picking b.
static __m128d select(__m128d a, __m128d c, __m128d b)
{
  const __m128d mask = _mm_cmpnle_pd(_mm_setzero_pd(), a);
  return _mm_blendv_pd(c, b, mask);
}

// fmaddXX with FMA3: xmm1 = c (rounded to 25 bits for single ops), then the 132 form.
static __m128d fused(u32 sub5, __m128d a, __m128d c, __m128d b)
{
  switch (sub5)
  {
  case 28:  // msub: c * a - b
    return _mm_fmsub_pd(c, a, b);
  case 30:  // nmsub: -(c * a) + b
    return _mm_fnmadd_pd(c, a, b);
  case 31:  // nmadd: -(c * a) - b
    return _mm_fnmsub_pd(c, a, b);
  default:  // madd, madds0, madds1: c * a + b
    return _mm_fmadd_pd(c, a, b);
  }
}

static u32 fcti(double b, bool truncate)
{
  // MINSD with 0x7FFFFFFF, then CVT(T)PD2DQ; a NaN passes through MINSD and converts to
  // 0x80000000.
  const __m128d x = _mm_min_sd(_mm_set_sd(2147483647.0), _mm_set_sd(b));
  const __m128i r = truncate ? _mm_cvttpd_epi32(x) : _mm_cvtpd_epi32(x);
  return static_cast<u32>(_mm_cvtsi128_si32(r));
}

extern "C" int ishiiruka_exec(uint32_t hex, uint64_t* fpr, uint32_t* cr)
{
  const u32 op = hex >> 26;
  const u32 d = (hex >> 21) & 31, a_ = (hex >> 16) & 31, b_ = (hex >> 11) & 31, c_ = (hex >> 6) & 31;
  const u32 sub5 = (hex >> 1) & 31, sub10 = (hex >> 1) & 1023;
  const u32 crf = (hex >> 23) & 7;
  const __m128d A = reg(fpr[2 * a_], fpr[2 * a_ + 1]);
  const __m128d B = reg(fpr[2 * b_], fpr[2 * b_ + 1]);
  const __m128d C = reg(fpr[2 * c_], fpr[2 * c_ + 1]);
  const __m128d D = reg(fpr[2 * d], fpr[2 * d + 1]);
  __m128d R = D;
  auto set_cr = [&](u32 v) { *cr = (*cr & ~(0xFu << (28 - 4 * crf))) | (v << (28 - 4 * crf)); };

  if (op == 59)
  {
    switch (sub5)
    {
    case 18: R = single_dup(_mm_div_sd(A, B)); break;
    case 20: R = single_dup(_mm_sub_sd(A, B)); break;
    case 21: R = single_dup(_mm_add_sd(A, B)); break;
    case 25: R = single_dup(_mm_mul_sd(A, force25(C))); break;
    case 24: R = _mm_set1_pd(MathUtil::ApproximateReciprocal(as_double(lo(B)))); break;
    case 28: case 29: case 30: case 31:
    {
      const __m128d r = fused(sub5, _mm_movedup_pd(A), _mm_movedup_pd(force25(C)), _mm_movedup_pd(B));
      R = single_dup(r);
      break;
    }
    default: return 0;
    }
  }
  else if (op == 63)
  {
    switch (sub5)
    {
    // Scalar double ops take the high lane from their first operand.
    case 18: R = _mm_div_sd(A, B); break;
    case 20: R = _mm_sub_sd(A, B); break;
    case 21: R = _mm_add_sd(A, B); break;
    case 25: R = _mm_mul_sd(A, C); break;
    case 23: R = movsd(D, select(A, C, B)); break;
    case 26: R = movsd(D, _mm_set_sd(MathUtil::ApproximateReciprocalSquareRoot(as_double(lo(B))))); break;
    case 28: case 29: case 30: case 31: R = movsd(D, fused(sub5, A, C, B)); break;
    default:
      switch (sub10)
      {
      case 0: case 32: set_cr(compare(as_double(lo(A)), as_double(lo(B)))); break;
      case 12: R = single_dup(B); break;
      case 14: case 15:
      {
        const u64 v = 0xFFF8000000000000ULL | fcti(as_double(lo(B)), sub10 == 15);
        R = movsd(D, reg(v, 0));
        break;
      }
      // fneg, fnabs and fabs change the low lane of frB and keep its high lane.
      case 40: R = _mm_xor_pd(B, sign_lo); break;
      case 136: R = _mm_or_pd(B, sign_lo); break;
      case 264: R = _mm_and_pd(B, abs_lo); break;
      case 72: R = movsd(D, B); break;
      default: return 0;
      }
    }
  }
  else if (op == 4)
  {
    switch (sub5)
    {
    case 10:  // ps_sum0: {a.ps0 + b.ps1, c.ps1}
    {
      const __m128d t = _mm_add_pd(_mm_movedup_pd(A), B);
      R = single_packed(_mm_unpackhi_pd(t, C));
      break;
    }
    case 11:  // ps_sum1: {c.ps0, a.ps0 + b.ps1}
    {
      const __m128d t = _mm_add_pd(_mm_movedup_pd(A), B);
      R = single_packed(_mm_blend_pd(t, C, 1));
      break;
    }
    case 12: R = single_packed(_mm_mul_pd(force25(_mm_movedup_pd(C)), A)); break;
    case 13: R = single_packed(_mm_mul_pd(force25(_mm_unpackhi_pd(C, C)), A)); break;
    case 14: R = single_packed(fused(14, A, force25(_mm_movedup_pd(C)), B)); break;
    case 15: R = single_packed(fused(15, A, force25(_mm_unpackhi_pd(C, C)), B)); break;
    case 18: R = single_packed(_mm_div_pd(A, B)); break;
    case 20: R = single_packed(_mm_sub_pd(A, B)); break;
    case 21: R = single_packed(_mm_add_pd(A, B)); break;
    case 25: R = single_packed(_mm_mul_pd(A, force25(C))); break;
    case 23: R = select(A, C, B); break;
    case 24:
      R = single_packed(reg(as_bits(MathUtil::ApproximateReciprocal(as_double(lo(B)))),
                            as_bits(MathUtil::ApproximateReciprocal(as_double(hi(B))))));
      break;
    case 26:
      R = single_packed(reg(as_bits(MathUtil::ApproximateReciprocalSquareRoot(as_double(lo(B)))),
                            as_bits(MathUtil::ApproximateReciprocalSquareRoot(as_double(hi(B))))));
      break;
    case 28: case 29: case 30: case 31: R = single_packed(fused(sub5, A, force25(C), B)); break;
    default:
      switch (sub10)
      {
      case 0: case 32: set_cr(compare(as_double(lo(A)), as_double(lo(B)))); break;
      case 64: case 96: set_cr(compare(as_double(hi(A)), as_double(hi(B)))); break;
      case 40: R = _mm_xor_pd(B, sign_both); break;
      case 136: R = _mm_or_pd(B, sign_both); break;
      case 264: R = _mm_and_pd(B, abs_both); break;
      case 72: R = B; break;
      case 528: R = _mm_unpacklo_pd(A, B); break;
      case 560: R = _mm_shuffle_pd(A, B, 2); break;
      case 592: R = _mm_shuffle_pd(A, B, 1); break;
      case 624: R = _mm_unpackhi_pd(A, B); break;
      default: return 0;
      }
    }
  }
  else
  {
    return 0;
  }
  fpr[2 * d] = lo(R);
  fpr[2 * d + 1] = hi(R);
  return 1;
}

// lfs: ConvertSingleToDouble, CVTSS2SD with a signaling NaN kept signaling.
extern "C" uint64_t ishiiruka_convert_to_double(uint32_t value)
{
  float f;
  std::memcpy(&f, &value, 4);
  u64 d = lo(_mm_cvtss_sd(_mm_setzero_pd(), _mm_set_ss(f)));
  if ((value & 0x7F800000) == 0x7F800000 && (value & 0x007FFFFF) != 0 && !(value & 0x00400000))
    d &= ~0x0008000000000000ULL;
  return d;
}

static u32 cvtsd2ss(u64 bits)
{
  const __m128 s = _mm_cvtsd_ss(_mm_setzero_ps(), _mm_castsi128_pd(_mm_cvtsi64_si128(static_cast<long long>(bits))));
  return static_cast<u32>(_mm_cvtsi128_si32(_mm_castps_si128(s)));
}

// stfs: ConvertDoubleToSingle. CVTSD2SS (x87 for results below FLT_MIN, which rounds the same),
// and a signaling NaN kept signaling.
extern "C" uint32_t ishiiruka_convert_to_single(uint64_t value)
{
  u32 s = cvtsd2ss(value);
  if ((value & 0x7FF0000000000000ULL) == 0x7FF0000000000000ULL && (value & 0x000FFFFFFFFFFFFFULL) != 0 &&
      !(value & 0x0008000000000000ULL))
    s &= 0xFFBFFFFF;
  return s;
}

// psq_st with a float GQR: CVTPD2PS.
extern "C" uint32_t ishiiruka_convert_to_single_ftz(uint64_t value)
{
  return cvtsd2ss(value);
}
}  // namespace ishiiruka
