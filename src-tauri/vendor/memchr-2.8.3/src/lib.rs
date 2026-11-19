//! Checked byte and substring searches over slices.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![no_std]

#[cfg(not(any(
    target_pointer_width = "16",
    target_pointer_width = "32",
    target_pointer_width = "64"
)))]
compile_error!("memchr currently not supported on non-{16,32,64}");

#[cfg(any(test, feature = "std"))]
extern crate std;

#[cfg(any(test, feature = "alloc"))]
extern crate alloc;

pub use crate::memchr::{
    memchr, memchr2, memchr2_iter, memchr3, memchr3_iter, memchr_iter,
    memrchr, memrchr2, memrchr2_iter, memrchr3, memrchr3_iter, memrchr_iter,
    Memchr, Memchr2, Memchr3,
};

#[cfg(test)]
#[macro_use]
mod tests;

pub mod arch;
mod cow;
mod memchr;
pub mod memmem;
