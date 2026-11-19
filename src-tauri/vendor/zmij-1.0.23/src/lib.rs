//! [![github]](https://github.com/dtolnay/zmij)&ensp;[![crates-io]](https://crates.io/crates/zmij)&ensp;[![docs-rs]](https://docs.rs/zmij)
//!
//! [github]: https://img.shields.io/badge/github-8da0cb?style=for-the-badge&labelColor=555555&logo=github
//! [crates-io]: https://img.shields.io/badge/crates.io-fc8d62?style=for-the-badge&labelColor=555555&logo=rust
//! [docs-rs]: https://img.shields.io/badge/docs.rs-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs
//!
//! <br>
//!
//! A double-to-string conversion algorithm based on [Schubfach] and [yy].
//!
//! The upstream Rust crate ports Victor Zverovich's implementation in C++,
//! <https://github.com/vitaut/zmij>. This copy retains the decimal conversion
//! and writes its digits into checked byte slices.
//!
//! [Schubfach]: https://fmt.dev/papers/Schubfach4.pdf
//! [yy]: https://github.com/ibireme/c_numconv_benchmark/blob/master/vendor/yy_double/yy_double.c
//!
//! <br>
//!
//! # Example
//!
//! ```
//! let mut buffer = zmij::Buffer::new();
//! let printed = buffer.format(1.234);
//! assert_eq!(printed, "1.234");
//! ```
//!
#![no_std]
#![forbid(unsafe_code)]
#![doc(html_root_url = "https://docs.rs/zmij/1.0.23")]
#![allow(non_camel_case_types)]

#[cfg(test)]
mod tests;
mod traits;

use core::mem;
use core::ops::RangeInclusive;
use core::str;

const BUFFER_SIZE: usize = 24;
const NAN: &str = "NaN";
const INFINITY: &str = "inf";
const NEG_INFINITY: &str = "-inf";

#[derive(Copy, Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
struct uint128 {
    hi: u64,
    lo: u64,
}

// Select the high-multiply form of the regular decimal exponent on Apple.
const USE_WIDE_EXP_MULTIPLY: bool = cfg!(target_vendor = "apple");

// Computes 128-bit result of multiplication of two 64-bit unsigned integers.
const fn umul128(x: u64, y: u64) -> u128 {
    x as u128 * y as u128
}

#[inline]
const fn umul128_hi64(x: u64, y: u64) -> u64 {
    (umul128(x, y) >> 64) as u64
}

// Returns (x * y + c) >> 64.
fn umul128_add_hi64(x: u64, y: u64, c: u64) -> u64 {
    ((u128::from(x) * u128::from(y) + u128::from(c)) >> 64) as u64
}

fn umul192_hi128(x_hi: u64, x_lo: u64, y: u64) -> uint128 {
    let p = umul128(x_hi, y);
    let lo = (p as u64).wrapping_add((umul128(x_lo, y) >> 64) as u64);
    uint128 {
        hi: (p >> 64) as u64 + u64::from(lo < p as u64),
        lo,
    }
}

// Returns x / 10 for x <= 2**62.
fn div10(x: u64) -> u64 {
    debug_assert!(x < (1 << 62));
    // ceil(2**64 / 10) computed as (1 << 63) / 5 + 1 to avoid int128.
    const DIV10_SIG64: u64 = (1 << 63) / 5 + 1;
    umul128_hi64(x, DIV10_SIG64)
}

// Computes the decimal exponent as floor(log10(2**bin_exp)) if regular or
// floor(log10(3/4 * 2**bin_exp)) otherwise, without branching.
const fn compute_dec_exp(bin_exp: i32, regular: bool) -> i32 {
    debug_assert!(bin_exp >= -1334 && bin_exp <= 2620);
    // log10_3_over_4_sig = -log10(3/4) * 2**log10_2_exp rounded to a power of 2
    const LOG10_3_OVER_4_SIG: i32 = 131_072;
    // log10_2_sig = round(log10(2) * 2**log10_2_exp)
    const LOG10_2_SIG: i32 = 315_653;
    const LOG10_2_EXP: i32 = 20;
    (bin_exp * LOG10_2_SIG - !regular as i32 * LOG10_3_OVER_4_SIG) >> LOG10_2_EXP
}

trait FloatTraits: traits::Float {
    // Note: Rust port uses wider fixed-notation ranges than upstream.
    const FIXED_DEC_EXP: RangeInclusive<i32>;

    const NUM_BITS: i32;
    const NUM_SIG_BITS: i32 = Self::MANTISSA_DIGITS as i32 - 1;
    const NUM_EXP_BITS: i32 = Self::NUM_BITS - Self::NUM_SIG_BITS - 1;
    const EXP_MASK: i32 = (1 << Self::NUM_EXP_BITS) - 1;
    const EXP_BIAS: i32 = (1 << (Self::NUM_EXP_BITS - 1)) - 1;
    const EXP_OFFSET: i32 = Self::EXP_BIAS + Self::NUM_SIG_BITS;

    type SigType: traits::UInt;
    const IMPLICIT_BIT: Self::SigType;

    fn to_bits(self) -> Self::SigType;

    fn is_negative(bits: Self::SigType) -> bool {
        (bits >> (Self::NUM_BITS - 1)) != Self::SigType::from(0)
    }

    fn get_sig(bits: Self::SigType) -> Self::SigType {
        bits & (Self::IMPLICIT_BIT - Self::SigType::from(1))
    }

    fn get_exp(bits: Self::SigType) -> i64 {
        (bits << 1u8 >> (Self::NUM_SIG_BITS + 1)).into() as i64
    }
}

impl FloatTraits for f32 {
    const FIXED_DEC_EXP: RangeInclusive<i32> = -6..=12;
    const NUM_BITS: i32 = 32;
    const IMPLICIT_BIT: u32 = 1 << Self::NUM_SIG_BITS;
    type SigType = u32;

    #[inline]
    fn to_bits(self) -> Self::SigType {
        self.to_bits()
    }
}

impl FloatTraits for f64 {
    const FIXED_DEC_EXP: RangeInclusive<i32> = -5..=15;
    const NUM_BITS: i32 = 64;
    const IMPLICIT_BIT: u64 = 1 << Self::NUM_SIG_BITS;
    type SigType = u64;

    #[inline]
    fn to_bits(self) -> Self::SigType {
        self.to_bits()
    }
}

#[rustfmt::skip]
const POW10_MINOR: [u64; 28] = [
    0x8000000000000000, 0xa000000000000000, 0xc800000000000000,
    0xfa00000000000000, 0x9c40000000000000, 0xc350000000000000,
    0xf424000000000000, 0x9896800000000000, 0xbebc200000000000,
    0xee6b280000000000, 0x9502f90000000000, 0xba43b74000000000,
    0xe8d4a51000000000, 0x9184e72a00000000, 0xb5e620f480000000,
    0xe35fa931a0000000, 0x8e1bc9bf04000000, 0xb1a2bc2ec5000000,
    0xde0b6b3a76400000, 0x8ac7230489e80000, 0xad78ebc5ac620000,
    0xd8d726b7177a8000, 0x878678326eac9000, 0xa968163f0a57b400,
    0xd3c21bcecceda100, 0x84595161401484a0, 0xa56fa5b99019a5c8,
    0xcecb8f27f4200f3a,
];

#[rustfmt::skip]
const POW10_MAJOR: [uint128; 23] = [
    uint128 { hi: 0xaf8e5410288e1b6f, lo: 0x07ecf0ae5ee44dda }, // -303
    uint128 { hi: 0xb1442798f49ffb4a, lo: 0x99cd11cfdf41779d }, // -275
    uint128 { hi: 0xb2fe3f0b8599ef07, lo: 0x861fa7e6dcb4aa15 }, // -247
    uint128 { hi: 0xb4bca50b065abe63, lo: 0x0fed077a756b53aa }, // -219
    uint128 { hi: 0xb67f6455292cbf08, lo: 0x1a3bc84c17b1d543 }, // -191
    uint128 { hi: 0xb84687c269ef3bfb, lo: 0x3d5d514f40eea742 }, // -163
    uint128 { hi: 0xba121a4650e4ddeb, lo: 0x92f34d62616ce413 }, // -135
    uint128 { hi: 0xbbe226efb628afea, lo: 0x890489f70a55368c }, // -107
    uint128 { hi: 0xbdb6b8e905cb600f, lo: 0x5400e987bbc1c921 }, //  -79
    uint128 { hi: 0xbf8fdb78849a5f96, lo: 0xde98520472bdd034 }, //  -51
    uint128 { hi: 0xc16d9a0095928a27, lo: 0x75b7053c0f178294 }, //  -23
    uint128 { hi: 0xc350000000000000, lo: 0x0000000000000000 }, //    5
    uint128 { hi: 0xc5371912364ce305, lo: 0x6c28000000000000 }, //   33
    uint128 { hi: 0xc722f0ef9d80aad6, lo: 0x424d3ad2b7b97ef6 }, //   61
    uint128 { hi: 0xc913936dd571c84c, lo: 0x03bc3a19cd1e38ea }, //   89
    uint128 { hi: 0xcb090c8001ab551c, lo: 0x5cadf5bfd3072cc6 }, //  117
    uint128 { hi: 0xcd036837130890a1, lo: 0x36dba887c37a8c10 }, //  145
    uint128 { hi: 0xcf02b2c21207ef2e, lo: 0x94f967e45e03f4bc }, //  173
    uint128 { hi: 0xd106f86e69d785c7, lo: 0xe13336d701beba52 }, //  201
    uint128 { hi: 0xd31045a8341ca07c, lo: 0x1ede48111209a051 }, //  229
    uint128 { hi: 0xd51ea6fa85785631, lo: 0x552a74227f3ea566 }, //  257
    uint128 { hi: 0xd732290fbacaf133, lo: 0xa97c177947ad4096 }, //  285
    uint128 { hi: 0xd94ad8b1c7380874, lo: 0x18375281ae7822bc }, //  313
];

#[rustfmt::skip]
const POW10_FIXUPS: [u32; 20] = [
    0x0a4e363f, 0x00001840, 0x00006400, 0x24200040, 0x00000000,
    0x0c000000, 0x82c81380, 0x5e4ce01f, 0xd730f60f, 0x0000001b,
    0x00000000, 0xcdf7fffc, 0x6e8201d8, 0x40cd3fd1, 0xdb642501,
    0x00000d0d, 0x14042400, 0x53713840, 0x11781db4, 0x00000000,
];

// 128-bit significands of powers of 10 rounded down.
#[repr(C, align(64))]
struct Pow10SignificandTable {
    data: [uint128; if Self::COMPRESS { 0 } else { Self::NUM_POW10S }],
}

impl Pow10SignificandTable {
    const COMPRESS: bool = cfg!(opt_level = "s");
    const NUM_POW10S: usize = 618;

    // Computes the 128-bit significand of 10**i using method by Dougall Johnson.
    #[inline]
    const fn compute(i: u32) -> uint128 {
        const STRIDE: u32 = POW10_MINOR.len() as u32;
        let m = POW10_MINOR[((i + 10) % STRIDE) as usize];
        let h = POW10_MAJOR[((i + 10) / STRIDE) as usize];

        let h1 = umul128_hi64(h.lo, m);

        let c0 = h.lo.wrapping_mul(m);
        let c1 = h1.wrapping_add(h.hi.wrapping_mul(m));
        let c2 = (c1 < h1) as u64 + umul128_hi64(h.hi, m);

        let mut result = if (c2 >> 63) != 0 {
            uint128 { hi: c2, lo: c1 }
        } else {
            uint128 {
                hi: (c2 << 1) | (c1 >> 63),
                lo: (c1 << 1) | (c0 >> 63),
            }
        };
        result.lo -= ((POW10_FIXUPS[(i >> 5) as usize] >> (i & 31)) & 1) as u64;
        result
    }

    const fn new() -> Self {
        let mut data =
            [uint128 { hi: 0, lo: 0 }; if Self::COMPRESS { 0 } else { Self::NUM_POW10S }];

        let mut i = 0;
        while i < data.len() {
            data[i] = Self::compute(i as u32);
            i += 1;
        }

        Pow10SignificandTable { data }
    }

    #[inline]
    fn get(&self, dec_exp: i32) -> uint128 {
        let i = (dec_exp + 293) as usize;
        if Self::COMPRESS {
            Self::compute(i as u32)
        } else {
            self.data[i]
        }
    }
}

// Computes a shift so that, after scaling by a power of 10, the intermediate
// result always has a fixed 128-bit fractional part (for double).
//
// Different binary exponents can map to the same decimal exponent, but place
// the decimal point at different bit positions. The shift compensates for this.
//
// For example, both 3 * 2**59 and 3 * 2**60 have dec_exp = 2, but dividing by
// 10^dec_exp puts the decimal point in different bit positions:
//   3 * 2**59 / 100 = 1.72...e+16  (needs shift = 1 + 1)
//   3 * 2**60 / 100 = 3.45...e+16  (needs shift = 2 + 1)
#[inline]
const fn compute_exp_shift(bin_exp: i32, dec_exp: i32) -> u8 {
    debug_assert!(dec_exp >= -350 && dec_exp <= 350);
    // log2_pow10_sig = round(log2(10) * 2**log2_pow10_exp) + 1
    const LOG2_POW10_SIG: i32 = 217_707;
    const LOG2_POW10_EXP: i32 = 16;
    // pow10_bin_exp = floor(log2(10**-dec_exp))
    let pow10_bin_exp = (-dec_exp * LOG2_POW10_SIG) >> LOG2_POW10_EXP;
    // pow10 = ((pow10_hi << 64) | pow10_lo) * 2**(pow10_bin_exp - 127)
    (bin_exp + pow10_bin_exp + 1) as u8
}

struct ExpShiftTable {
    data: [u8; if Self::ENABLE {
        f64::EXP_MASK as usize + 1
    } else {
        0
    }],
}

impl ExpShiftTable {
    const ENABLE: bool = cfg!(not(opt_level = "s"));
    // extra_shift must be >= 3 to keep shift non-negative and <= 11 to fit the
    // significand into 64 bits after the shift.
    const EXTRA_SHIFT: usize = 6;

    const fn new() -> Self {
        let mut data = [0u8; if Self::ENABLE {
            f64::EXP_MASK as usize + 1
        } else {
            0
        }];

        let mut raw_exp = 0;
        while raw_exp < data.len() && Self::ENABLE {
            let mut bin_exp = raw_exp as i32 - f64::EXP_OFFSET;
            if raw_exp == 0 {
                bin_exp += 1;
            }
            let dec_exp = compute_dec_exp(bin_exp, true);
            data[raw_exp] =
                compute_exp_shift(bin_exp, dec_exp + 1).wrapping_add(Self::EXTRA_SHIFT as u8);
            raw_exp += 1;
        }

        ExpShiftTable { data }
    }
}

static POW10: Pow10SignificandTable = Pow10SignificandTable::new();
static EXP_SHIFTS: ExpShiftTable = ExpShiftTable::new();

struct ToDecimalResult {
    sig: i64,
    exp: i32,
    last_digit: u8,
    has_last_digit: bool,
}

// Converts a binary FP number bin_sig * 2**bin_exp to the shortest decimal
// representation, where bin_exp = raw_exp - exp_offset.
#[inline]
fn to_decimal<Float, UInt>(bin_sig: UInt, raw_exp: i64, regular: bool) -> ToDecimalResult
where
    Float: FloatTraits,
    UInt: traits::UInt,
{
    let bin_exp = raw_exp - i64::from(Float::EXP_OFFSET);
    let num_bits = mem::size_of::<UInt>() as i32 * 8;
    const EXTRA_SHIFT: usize = ExpShiftTable::EXTRA_SHIFT;

    if !regular {
        let dec_exp = compute_dec_exp(bin_exp as i32, false);
        let shift = compute_exp_shift(bin_exp as i32, dec_exp + 1).wrapping_add(EXTRA_SHIFT as u8);
        let pow10 = POW10.get(-dec_exp - 1);
        let p = umul192_hi128(pow10.hi, pow10.lo, (bin_sig << shift).into());

        let mut integral = p.hi >> EXTRA_SHIFT;
        let fractional = (p.hi << (64 - EXTRA_SHIFT)) | (p.lo >> EXTRA_SHIFT);

        let half_ulp = pow10.hi >> (EXTRA_SHIFT + 1 - shift as usize);
        let round_up = half_ulp > u64::MAX - fractional;
        let round_down = (half_ulp >> 1) > fractional;
        integral += u64::from(round_up);

        let mut digit = umul128_add_hi64(fractional, 10, (1 << 63) - 1) as i32;
        let lo = umul128_add_hi64(fractional.wrapping_sub(half_ulp >> 1), 10, !0) as i32;
        if digit < lo {
            digit = lo;
        }
        return ToDecimalResult {
            sig: integral as i64,
            exp: dec_exp,
            last_digit: digit as u8,
            has_last_digit: !(round_up || round_down),
        };
    }

    const LOG10_2_SIG: u64 = 78_913;
    const LOG10_2_EXP: i32 = 18;
    let dec_exp = if USE_WIDE_EXP_MULTIPLY {
        umul128_hi64(bin_exp as u64, LOG10_2_SIG << (64 - LOG10_2_EXP)) as i32
    } else {
        compute_dec_exp(bin_exp as i32, true)
    };
    let mut shift = if ExpShiftTable::ENABLE {
        EXP_SHIFTS.data[(bin_exp + i64::from(f64::EXP_OFFSET)) as usize]
    } else {
        compute_exp_shift(bin_exp as i32, dec_exp + 1).wrapping_add(EXTRA_SHIFT as u8)
    };
    let even = UInt::from(1) - (bin_sig & UInt::from(1));

    if num_bits == 32 {
        const EXTRA_SHIFT: usize = 34;
        shift += (EXTRA_SHIFT - ExpShiftTable::EXTRA_SHIFT) as u8;
        let pow10_hi = POW10.get(-dec_exp - 1).hi;
        let p = umul128_hi64(pow10_hi + 1, bin_sig.into() << shift);

        let mut integral = p >> EXTRA_SHIFT;
        let fractional = p & ((1u64 << EXTRA_SHIFT) - 1);

        let half_ulp = (pow10_hi >> (65 - shift as usize)) + even.into();
        let round_up = ((fractional + half_ulp) >> EXTRA_SHIFT) != 0;
        let round_down = half_ulp > fractional;
        integral += u64::from(round_up);

        let mut digit = ((fractional * 10 + (1u64 << (EXTRA_SHIFT - 1))) >> EXTRA_SHIFT) as i32;
        if fractional == (1u64 << (EXTRA_SHIFT - 2)) {
            digit = 2; // Round 2.5 to 2.
        }
        return ToDecimalResult {
            sig: integral as i64,
            exp: dec_exp,
            last_digit: digit as u8,
            has_last_digit: !(round_up || round_down),
        };
    }

    // An optimization by Xiang JunBo:
    // Scale by 10**(-dec_exp-1) to directly produce the shorter candidate
    // (15-16 digits), deriving the extra digit from the fractional part.
    // This eliminates div10 from the critical path.
    //
    // value = 5.0507837461e-27
    // next  = 5.0507837461000010e-27
    //
    // c = integral.fractional' = 5050783746100000.3153987... (value)
    //                            5050783746100001.0328635... (next)
    //                 half_ulp =                0.3587324...
    //
    // fractional = fractional' * 2**64 = 5818079786399166407
    //
    //    5050783746100000.0       c               upper    5050783746100001.0
    //             s              l|   L             |               S
    // ──┬────┬────┼────┬────┬────┼*───┼────┬────┬───*┬────┬────┬────┼─*──┬───
    //  .8   .9   .0   .1   .2   .3   .4   .5   .6   .7   .8   .9   .0 | .1
    //           └─────────────────┼─────────────────┘                next
    //                            1ulp
    //
    // s - shorter underestimate, S - shorter overestimate
    // l - longer underestimate,  L - longer overestimate
    let pow10 = POW10.get(-dec_exp - 1);
    let p = umul192_hi128(pow10.hi, pow10.lo, (bin_sig << shift).into());

    let mut integral = p.hi >> EXTRA_SHIFT;
    let fractional = (p.hi << (64 - EXTRA_SHIFT)) | (p.lo >> EXTRA_SHIFT);

    let half_ulp = (pow10.hi >> (EXTRA_SHIFT + 1 - shift as usize)) + even.into();
    let round_up = fractional.wrapping_add(half_ulp) < fractional;
    let round_down = half_ulp > fractional;
    integral += u64::from(round_up); // Compute integral before digit.

    // Derive the extra digit from the fractional part (parallel with rounding).
    let mut digit = umul128_add_hi64(fractional, 10, (1 << 63) + 6) as i32;
    if fractional == (1u64 << 62) {
        digit = 2; // Round 2.5 to 2.
    }
    ToDecimalResult {
        sig: integral as i64,
        exp: dec_exp,
        last_digit: digit as u8,
        has_last_digit: !(round_up || round_down),
    }
}

fn write<Float>(value: Float, buffer: &mut [u8; BUFFER_SIZE]) -> usize
where
    Float: FloatTraits,
{
    let bits = value.to_bits();
    // It is beneficial to extract exponent and significand early.
    let bin_exp = Float::get_exp(bits); // binary exponent
    let bin_sig = Float::get_sig(bits); // binary significand

    let sign_len = usize::from(Float::is_negative(bits));
    buffer[0] = b'-';
    let output = &mut buffer[sign_len..];
    let threshold: i64 = if Float::NUM_BITS == 64 {
        1_000_000_000_000_000
    } else {
        10_000_000
    };

    let mut dec;
    if bin_exp == 0 {
        if bin_sig == Float::SigType::from(0) {
            output[..3].copy_from_slice(b"0.0");
            return sign_len + 3;
        }
        dec = to_decimal::<Float, Float::SigType>(bin_sig, 1, true);
        let mut dec_sig =
            dec.sig * 10 + (-i64::from(dec.has_last_digit) & i64::from(dec.last_digit));
        let mut dec_exp = dec.exp;
        while dec_sig < threshold {
            dec_sig *= 10;
            dec_exp -= 1;
        }
        let d = div10(dec_sig as u64);
        let last_digit = dec_sig - d as i64 * 10;
        dec = ToDecimalResult {
            sig: d as i64,
            exp: dec_exp,
            last_digit: last_digit as u8,
            has_last_digit: last_digit != 0,
        };
    } else {
        dec = to_decimal::<Float, Float::SigType>(
            bin_sig | Float::IMPLICIT_BIT,
            bin_exp,
            bin_sig != Float::SigType::from(0),
        );
    }
    let mut has_last_digit = dec.has_last_digit;
    let has_extra_digit = dec.sig >= threshold;
    let mut dec_exp = dec.exp + Float::MAX_DIGITS10 as i32 - 2 + i32::from(has_extra_digit);
    if Float::NUM_BITS == 32 && dec.sig < 1_000_000 {
        dec.sig = 10 * dec.sig + (-i64::from(has_last_digit) & i64::from(dec.last_digit));
        has_last_digit = false;
        dec_exp -= 1;
    }

    let mut significand = dec.sig as u64;
    if has_last_digit {
        significand = significand * 10 + u64::from(dec.last_digit);
    }

    let mut storage = [0; 17];
    let mut digits = decimal_digits(significand, &mut storage);
    if !has_last_digit {
        let end = digits.iter().rposition(|&digit| digit != b'0').unwrap_or(0) + 1;
        digits = &digits[..end];
    }

    let count = if Float::FIXED_DEC_EXP.contains(&dec_exp) {
        write_fixed(digits, dec_exp, output)
    } else {
        write_exponential(digits, dec_exp, output)
    };
    sign_len + count
}

const DECIMAL_PAIRS: [[u8; 2]; 100] = {
    let mut pairs = [[0; 2]; 100];
    let mut index = 0;
    while index < pairs.len() {
        pairs[index] = [b'0' + (index / 10) as u8, b'0' + (index % 10) as u8];
        index += 1;
    }
    pairs
};

#[inline]
fn decimal_digits(mut value: u64, storage: &mut [u8; 17]) -> &[u8] {
    let mut start = storage.len();
    while value >= 100 {
        let pair = (value % 100) as usize;
        value /= 100;
        start -= 2;
        storage[start..start + 2].copy_from_slice(&DECIMAL_PAIRS[pair]);
    }
    if value < 10 {
        start -= 1;
        storage[start] = b'0' + value as u8;
    } else {
        start -= 2;
        storage[start..start + 2].copy_from_slice(&DECIMAL_PAIRS[value as usize]);
    }
    &storage[start..]
}

#[inline]
fn write_fixed(digits: &[u8], exp: i32, output: &mut [u8]) -> usize {
    if exp < 0 {
        let start = (1 - exp) as usize;
        output[..start].fill(b'0');
        output[1] = b'.';
        output[start..start + digits.len()].copy_from_slice(digits);
        start + digits.len()
    } else {
        let point = exp as usize + 1;
        if point >= digits.len() {
            output[..digits.len()].copy_from_slice(digits);
            output[digits.len()..point].fill(b'0');
            output[point..point + 2].copy_from_slice(b".0");
            point + 2
        } else {
            output[..point].copy_from_slice(&digits[..point]);
            output[point] = b'.';
            output[point + 1..digits.len() + 1].copy_from_slice(&digits[point..]);
            digits.len() + 1
        }
    }
}

#[inline]
fn write_exponential(digits: &[u8], exp: i32, output: &mut [u8]) -> usize {
    output[0] = digits[0];
    let mut end = 1;
    if digits.len() > 1 {
        output[1] = b'.';
        end = digits.len() + 1;
        output[2..end].copy_from_slice(&digits[1..]);
    }
    output[end] = b'e';
    output[end + 1] = if exp < 0 { b'-' } else { b'+' };

    let exponent = exp.unsigned_abs() as usize;
    end += 2;
    if exponent < 10 {
        output[end] = b'0' + exponent as u8;
        end + 1
    } else {
        if exponent >= 100 {
            output[end] = b'0' + (exponent / 100) as u8;
            end += 1;
        }
        output[end..end + 2].copy_from_slice(&DECIMAL_PAIRS[exponent % 100]);
        end + 2
    }
}

/// Safe API for formatting floating point numbers to text.
///
/// ## Example
///
/// ```
/// let mut buffer = zmij::Buffer::new();
/// let printed = buffer.format_finite(1.234);
/// assert_eq!(printed, "1.234");
/// ```
pub struct Buffer {
    bytes: [u8; BUFFER_SIZE],
}

impl Buffer {
    /// This is a cheap operation; you don't need to worry about reusing buffers
    /// for efficiency.
    #[inline]
    pub fn new() -> Self {
        let bytes = [0; BUFFER_SIZE];
        Buffer { bytes }
    }

    /// Print a floating point number into this buffer and return a reference to
    /// its string representation within the buffer.
    ///
    /// # Special cases
    ///
    /// This function formats NaN as the string "NaN", positive infinity as
    /// "inf", and negative infinity as "-inf" to match std::fmt.
    ///
    /// If your input is known to be finite, you may get better performance by
    /// calling the `format_finite` method instead of `format` to avoid the
    /// checks for special cases.
    pub fn format<F: Float>(&mut self, f: F) -> &str {
        if f.is_nonfinite() {
            f.format_nonfinite()
        } else {
            self.format_finite(f)
        }
    }

    /// Print a floating point number into this buffer and return a reference to
    /// its string representation within the buffer.
    ///
    /// # Special cases
    ///
    /// This function **does not** check for NaN or infinity. If the input
    /// number is not a finite float, the printed representation will be some
    /// correctly formatted but unspecified numerical value.
    ///
    /// Please check [`is_finite`] yourself before calling this function, or
    /// check [`is_nan`] and [`is_infinite`] and handle those cases yourself.
    ///
    /// [`is_finite`]: f64::is_finite
    /// [`is_nan`]: f64::is_nan
    /// [`is_infinite`]: f64::is_infinite
    pub fn format_finite<F: Float>(&mut self, f: F) -> &str {
        let len = f.write_to_zmij_buffer(&mut self.bytes);
        str::from_utf8(&self.bytes[..len]).expect("decimal formatting produces ASCII")
    }
}

/// A floating point number, f32 or f64, that can be written into a
/// [`zmij::Buffer`][Buffer].
///
/// This trait is sealed and cannot be implemented for types outside of the
/// `zmij` crate.
#[allow(unknown_lints)] // rustc older than 1.74
#[allow(private_bounds)]
pub trait Float: private::Sealed {}
impl Float for f32 {}
impl Float for f64 {}

mod private {
    pub trait Sealed: crate::traits::Float {
        fn is_nonfinite(self) -> bool;
        fn format_nonfinite(self) -> &'static str;
        fn write_to_zmij_buffer(self, buffer: &mut [u8; crate::BUFFER_SIZE]) -> usize;
    }

    impl Sealed for f32 {
        #[inline]
        fn is_nonfinite(self) -> bool {
            const EXP_MASK: u32 = 0x7f800000;
            let bits = self.to_bits();
            bits & EXP_MASK == EXP_MASK
        }

        #[cold]
        fn format_nonfinite(self) -> &'static str {
            const MANTISSA_MASK: u32 = 0x007fffff;
            const SIGN_MASK: u32 = 0x80000000;
            let bits = self.to_bits();
            if bits & MANTISSA_MASK != 0 {
                crate::NAN
            } else if bits & SIGN_MASK != 0 {
                crate::NEG_INFINITY
            } else {
                crate::INFINITY
            }
        }

        fn write_to_zmij_buffer(self, buffer: &mut [u8; crate::BUFFER_SIZE]) -> usize {
            crate::write(self, buffer)
        }
    }

    impl Sealed for f64 {
        #[inline]
        fn is_nonfinite(self) -> bool {
            const EXP_MASK: u64 = 0x7ff0000000000000;
            let bits = self.to_bits();
            bits & EXP_MASK == EXP_MASK
        }

        #[cold]
        fn format_nonfinite(self) -> &'static str {
            const MANTISSA_MASK: u64 = 0x000fffffffffffff;
            const SIGN_MASK: u64 = 0x8000000000000000;
            let bits = self.to_bits();
            if bits & MANTISSA_MASK != 0 {
                crate::NAN
            } else if bits & SIGN_MASK != 0 {
                crate::NEG_INFINITY
            } else {
                crate::INFINITY
            }
        }

        fn write_to_zmij_buffer(self, buffer: &mut [u8; crate::BUFFER_SIZE]) -> usize {
            crate::write(self, buffer)
        }
    }
}

impl Default for Buffer {
    #[inline]
    fn default() -> Self {
        Buffer::new()
    }
}
