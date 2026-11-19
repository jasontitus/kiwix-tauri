//! This module contains backend-specific code.

use crate::mem::{CompressError, DecompressError, FlushCompress, FlushDecompress, Status};
use crate::Compression;
use core::mem::MaybeUninit;

/// Traits specifying the interface of the backends.
///
/// Sync + Send are added as a condition to ensure they are available
/// for the frontend.
pub trait Backend: Sync + Send {
    fn total_in(&self) -> u64;
    fn total_out(&self) -> u64;
}

pub trait InflateBackend: Backend {
    fn make(zlib_header: bool, window_bits: u8) -> Self;
    fn decompress(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: FlushDecompress,
    ) -> Result<Status, DecompressError>;
    fn decompress_uninit(
        &mut self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
        flush: FlushDecompress,
    ) -> Result<Status, DecompressError>;
    fn reset(&mut self, zlib_header: bool);
}

pub trait DeflateBackend: Backend {
    fn make(level: Compression, zlib_header: bool, window_bits: u8) -> Self;
    fn compress(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: FlushCompress,
    ) -> Result<Status, CompressError>;
    fn compress_uninit(
        &mut self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
        flush: FlushCompress,
    ) -> Result<Status, CompressError>;
    fn reset(&mut self);
}

// Default to Rust implementation unless explicitly opted in to a different backend.
#[cfg(feature = "any_c_zlib")]
mod c;
#[cfg(feature = "any_c_zlib")]
pub use self::c::*;

// Only bring in `zlib-rs` if there is no C-based backend.
#[cfg(all(not(feature = "any_c_zlib"), feature = "zlib-rs"))]
mod zlib_rs;
#[cfg(all(not(feature = "any_c_zlib"), feature = "zlib-rs"))]
pub use self::zlib_rs::*;

// Use miniz_oxide when no fully compliant zlib is selected.
#[cfg(all(not(feature = "any_zlib"), feature = "miniz_oxide"))]
mod miniz_oxide;
#[cfg(all(not(feature = "any_zlib"), feature = "miniz_oxide"))]
pub use self::miniz_oxide::*;

// If no backend is enabled, fail fast with a clear error message.
#[cfg(not(feature = "any_impl"))]
compile_error!("No compression backend selected; enable one of `zlib`, `zlib-ng`, `zlib-rs`, or the default `rust_backend` feature.");

impl core::fmt::Debug for ErrorMessage {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        self.get().fmt(f)
    }
}
