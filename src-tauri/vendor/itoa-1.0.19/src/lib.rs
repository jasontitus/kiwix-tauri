//! [![github]](https://github.com/dtolnay/itoa)&ensp;[![crates-io]](https://crates.io/crates/itoa)&ensp;[![docs-rs]](https://docs.rs/itoa)
//!
//! [github]: https://img.shields.io/badge/github-8da0cb?style=for-the-badge&labelColor=555555&logo=github
//! [crates-io]: https://img.shields.io/badge/crates.io-fc8d62?style=for-the-badge&labelColor=555555&logo=rust
//! [docs-rs]: https://img.shields.io/badge/docs.rs-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs
//!
//! <br>
//!
//! This crate provides a fast conversion of integer primitives to decimal
//! strings. The implementation comes straight from [libcore] but avoids the
//! performance penalty of going through [`core::fmt::Formatter`].
//!
//! See also [`zmij`] for printing floating point primitives.
//!
//! [libcore]: https://github.com/rust-lang/rust/blob/1.92.0/library/core/src/fmt/num.rs#L190-L253
//! [`zmij`]: https://github.com/dtolnay/zmij
//!
//! # Example
//!
//! ```
//! fn main() {
//!     let mut buffer = itoa::Buffer::new();
//!     let printed = buffer.format(128u64);
//!     assert_eq!(printed, "128");
//! }
//! ```
//!
//! # Performance
//!
//! The [itoa-benchmark] compares this library and other Rust integer formatting
//! implementations across a range of integer sizes. The vertical axis in this
//! chart shows nanoseconds taken by a single execution of
//! `itoa::Buffer::new().format(value)` so a lower result indicates a faster
//! library.
//!
//! [itoa-benchmark]: https://github.com/dtolnay/itoa-benchmark
//!
//! ![performance](https://raw.githubusercontent.com/dtolnay/itoa/master/itoa-benchmark.png)

#![doc(html_root_url = "https://docs.rs/itoa/1.0.19")]
#![no_std]
#![forbid(unsafe_code)]
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::expl_impl_clone_on_copy,
    clippy::identity_op,
    clippy::items_after_statements,
    clippy::must_use_candidate,
    clippy::needless_doctest_main,
    clippy::unreadable_literal
)]

mod u128_ext;

use core::mem;
use core::str;

/// A correctly sized stack allocation for the formatted integer to be written
/// into.
///
/// # Example
///
/// ```
/// let mut buffer = itoa::Buffer::new();
/// let printed = buffer.format(1234);
/// assert_eq!(printed, "1234");
/// ```
pub struct Buffer {
    bytes: [u8; i128::MAX_STR_LEN],
}

impl Default for Buffer {
    #[inline]
    fn default() -> Buffer {
        Buffer::new()
    }
}

impl Copy for Buffer {}

#[allow(clippy::non_canonical_clone_impl)]
impl Clone for Buffer {
    #[inline]
    fn clone(&self) -> Self {
        Buffer::new()
    }
}

impl Buffer {
    /// This is a cheap operation; you don't need to worry about reusing buffers
    /// for efficiency.
    #[inline]
    pub fn new() -> Buffer {
        let bytes = [0; i128::MAX_STR_LEN];
        Buffer { bytes }
    }

    /// Print an integer into this buffer and return a reference to its string
    /// representation within the buffer.
    pub fn format<I: Integer>(&mut self, i: I) -> &str {
        let buf = &mut self.bytes[..I::MAX_STR_LEN];
        let offset = i.write(buf);
        str::from_utf8(&buf[offset..]).expect("integer formatting produces ASCII")
    }
}

/// An integer that can be written into an [`itoa::Buffer`][Buffer].
///
/// This trait is sealed and cannot be implemented for types outside of itoa.
pub trait Integer: private::Sealed {
    /// The maximum length of string that formatting an integer of this type can
    /// produce on the current target platform.
    const MAX_STR_LEN: usize;
}

// Seal to prevent downstream implementations of the Integer trait.
mod private {
    #[doc(hidden)]
    pub trait Sealed: Copy {
        fn write(self, buf: &mut [u8]) -> usize;
    }
}

macro_rules! impl_Integer {
    ($Signed:ident, $Unsigned:ident) => {
        const _: () = {
            assert!($Signed::MIN < 0, "need signed");
            assert!($Unsigned::MIN == 0, "need unsigned");
            assert!($Signed::BITS == $Unsigned::BITS, "need counterparts");
        };

        impl Integer for $Unsigned {
            const MAX_STR_LEN: usize = $Unsigned::MAX.ilog10() as usize + 1;
        }

        impl private::Sealed for $Unsigned {
            #[inline]
            fn write(self, buf: &mut [u8]) -> usize {
                Unsigned::fmt(self, buf)
            }
        }

        impl Integer for $Signed {
            const MAX_STR_LEN: usize = $Signed::MAX.ilog10() as usize + 2;
        }

        impl private::Sealed for $Signed {
            #[inline]
            fn write(self, buf: &mut [u8]) -> usize {
                let mut offset = Unsigned::fmt(self.unsigned_abs(), buf);
                if self < 0 {
                    offset -= 1;
                    buf[offset] = b'-';
                }
                offset
            }
        }
    };
}

impl_Integer!(i8, u8);
impl_Integer!(i16, u16);
impl_Integer!(i32, u32);
impl_Integer!(i64, u64);
impl_Integer!(i128, u128);

macro_rules! impl_Integer_size {
    ($t:ty as $primitive:ident #[cfg(target_pointer_width = $width:literal)]) => {
        #[cfg(target_pointer_width = $width)]
        impl Integer for $t {
            const MAX_STR_LEN: usize = <$primitive as Integer>::MAX_STR_LEN;
        }

        #[cfg(target_pointer_width = $width)]
        impl private::Sealed for $t {
            #[inline]
            fn write(self, buf: &mut [u8]) -> usize {
                (self as $primitive).write(buf)
            }
        }
    };
}

impl_Integer_size!(isize as i16 #[cfg(target_pointer_width = "16")]);
impl_Integer_size!(usize as u16 #[cfg(target_pointer_width = "16")]);
impl_Integer_size!(isize as i32 #[cfg(target_pointer_width = "32")]);
impl_Integer_size!(usize as u32 #[cfg(target_pointer_width = "32")]);
impl_Integer_size!(isize as i64 #[cfg(target_pointer_width = "64")]);
impl_Integer_size!(usize as u64 #[cfg(target_pointer_width = "64")]);

// The string of all two-digit numbers in range 00..99, stored as pairs
// so writing one pair only needs one lookup and one checked range.
static DECIMAL_PAIRS: [[u8; 2]; 100] = {
    let mut pairs = [[0; 2]; 100];
    let mut n = 0;
    while n < pairs.len() {
        pairs[n] = [b'0' + (n / 10) as u8, b'0' + (n % 10) as u8];
        n += 1;
    }
    pairs
};

#[inline]
fn write_pair(buf: &mut [u8], pair: u32) {
    buf.copy_from_slice(&DECIMAL_PAIRS[pair as usize]);
}

#[inline]
fn write_quad(buf: &mut [u8], quad: u32) {
    let (high, low) = divmod100(quad);
    let (first, second) = buf.split_at_mut(2);
    write_pair(first, high);
    write_pair(second, low);
}

// Returns {value / 100, value % 100} correct for values of up to 4 digits.
fn divmod100(value: u32) -> (u32, u32) {
    debug_assert!(value < 10_000);
    const EXP: u32 = 19; // 19 is faster or equal to 12 even for 3 digits.
    const SIG: u32 = (1 << EXP) / 100 + 1;
    let div = (value * SIG) >> EXP; // value / 100
    (div, value - div * 100)
}

trait Unsigned: Integer {
    fn fmt(self, buf: &mut [u8]) -> usize;
}

macro_rules! impl_Unsigned {
    ($Unsigned:ident) => {
        impl Unsigned for $Unsigned {
            fn fmt(self, buf: &mut [u8]) -> usize {
                // Offset of the first digit written so far.
                let mut offset = buf.len();
                // Consume the least-significant decimals from a working copy.
                let mut remain = self;

                // Format per four digits from the lookup table.
                // Four digits need a 16-bit $Unsigned or wider.
                while mem::size_of::<Self>() > 1
                    && remain
                        > 999
                            .try_into()
                            .expect("branch is not hit for types that cannot fit 999 (u8)")
                {
                    offset -= 4;

                    // pull two pairs
                    let scale: Self = 1_00_00
                        .try_into()
                        .expect("branch is not hit for types that cannot fit 1E4 (u8)");
                    let quad = remain % scale;
                    remain /= scale;
                    write_quad(&mut buf[offset..offset + 4], quad as u32);
                }

                // Format per two digits from the lookup table.
                if remain > 9 {
                    offset -= 2;

                    let (last, pair) = divmod100(remain as u32);
                    remain = last as Self;
                    write_pair(&mut buf[offset..offset + 2], pair);
                }

                // Format the last remaining digit, if any.
                if remain != 0 || self == 0 {
                    offset -= 1;

                    // Either the compiler sees that remain < 10, or it prevents
                    // a boundary check up next.
                    let last = remain as u8 & 15;
                    buf[offset] = b'0' + last;
                    // not used: remain = 0;
                }

                offset
            }
        }
    };
}

impl_Unsigned!(u8);
impl_Unsigned!(u16);
impl_Unsigned!(u32);
impl_Unsigned!(u64);

impl Unsigned for u128 {
    fn fmt(self, buf: &mut [u8]) -> usize {
        let mut offset = buf.len();
        let (quotient, mut last) = div_rem_1e16(self);
        if quotient != 0 {
            offset -= 16;
            write_16_digits(&mut buf[offset..offset + 16], last);

            let (leading, rest) = div_rem_1e16(quotient);
            last = rest;
            if leading != 0 {
                offset -= 16;
                write_16_digits(&mut buf[offset..offset + 16], rest);
                // At most seven decimal digits remain.
                last = leading as u64;
            }
        }

        u64::fmt(last, &mut buf[..offset])
    }
}

// Write precisely sixteen least-significant digits, including leading zeroes.
#[inline]
fn write_16_digits(buf: &mut [u8], mut number: u64) {
    for quad in buf.rchunks_exact_mut(4) {
        write_quad(quad, (number % 10_000) as u32);
        number /= 10_000;
    }
}

// Euclidean division plus remainder with constant 1E16 basically consumes 16
// decimals from n.
//
// The integer division algorithm is based on the following paper:
//
//   T. Granlund and P. Montgomery, “Division by Invariant Integers Using Multiplication”
//   in Proc. of the SIGPLAN94 Conference on Programming Language Design and
//   Implementation, 1994, pp. 61–72
//
fn div_rem_1e16(n: u128) -> (u128, u64) {
    const D: u128 = 1_0000_0000_0000_0000;
    // The check inlines well with the caller flow.
    if n < D {
        return (0, n as u64);
    }

    // These constant values are computed with the CHOOSE_MULTIPLIER procedure
    // from the Granlund & Montgomery paper, using N=128, prec=128 and d=1E16.
    const M_HIGH: u128 = 76624777043294442917917351357515459181;
    const SH_POST: u8 = 51;

    // n.widening_mul(M_HIGH).1 >> SH_POST
    let quot = u128_ext::mulhi(n, M_HIGH) >> SH_POST;
    let rem = n - quot * D;
    (quot, rem as u64)
}
