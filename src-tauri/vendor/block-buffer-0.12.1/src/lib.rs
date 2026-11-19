//! Fixed size buffer for block processing of data.
//!
//! # Examples
//! ```
//! use block_buffer::{EagerBuffer, array::typenum::U4};
//!
//! let mut buf = EagerBuffer::<U4>::default();
//!
//! let mut accum = Vec::new();
//! let msg1: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
//! let msg2: &[u8] = &[10, 11, 12];
//!
//! buf.digest_blocks(msg1, |blocks| accum.extend_from_slice(blocks));
//! buf.digest_blocks(msg2, |blocks| accum.extend_from_slice(blocks));
//!
//! assert_eq!(accum.len(), 3);
//! assert_eq!(accum[0], [0, 1, 2, 3]);
//! assert_eq!(accum[1], [4, 5, 6, 7]);
//! assert_eq!(accum[2], [8, 9, 10, 11]);
//!
//! let padded_block = buf.pad_with_zeros();
//! assert_eq!(padded_block, [12, 0, 0, 0]);
//! ```
//!
//! Note that block size used with buffers MUST be bigger than zero and smaller than 256.
//! You will get a compilation error with an invalid block size:
//!
//! ```compile_fail
//! use block_buffer::{EagerBuffer, array::typenum::U0};
//! let buf = EagerBuffer::<U0>::default();
//! ```
//! ```compile_fail
//! use block_buffer::{EagerBuffer, array::typenum::U256};
//! let buf = EagerBuffer::<U256>::default();
//! ```
#![no_std]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/RustCrypto/media/6ee8e381/logo.svg",
    html_favicon_url = "https://raw.githubusercontent.com/RustCrypto/media/6ee8e381/logo.svg"
)]
#![forbid(unsafe_code)]

// Modified by Kiwix to use initialized storage and checked slice operations.

pub use hybrid_array as array;

use array::{Array, ArraySize, typenum::Sum};
use core::{fmt, slice};

#[cfg(feature = "zeroize")]
use zeroize::{Zeroize, ZeroizeOnDrop};

mod read;
mod sealed;

pub use read::ReadBuffer;

/// Trait implemented for supported block sizes, i.e. for types from `U1` to `U255`.
pub trait BlockSizes: ArraySize + sealed::BlockSizes {}

impl<T: ArraySize + sealed::BlockSizes> BlockSizes for T {}

/// Trait for buffer kinds.
pub trait BufferKind: sealed::Sealed {}

/// Eager block buffer kind, which guarantees that buffer position
/// always lies in the range of `0..BlockSize`.
#[derive(Copy, Clone, Debug, Default)]
pub struct Eager {}

/// Lazy block buffer kind, which guarantees that buffer position
/// always lies in the range of `0..=BlockSize`.
#[derive(Copy, Clone, Debug, Default)]
pub struct Lazy {}

impl BufferKind for Eager {}

impl BufferKind for Lazy {}

/// Eager block buffer.
pub type EagerBuffer<B> = BlockBuffer<B, Eager>;
/// Lazy block buffer.
pub type LazyBuffer<B> = BlockBuffer<B, Lazy>;

/// Block buffer error.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.write_str("Block buffer error")
    }
}

enum BlockBatch<'a, BS: BlockSizes> {
    Buffered(&'a Array<u8, BS>),
    Input(&'a [u8]),
}

/// Buffer for block processing of data.
pub struct BlockBuffer<BS: BlockSizes, K: BufferKind> {
    buffer: Array<u8, BS>,
    pos: K::Pos,
}

impl<BS: BlockSizes, K: BufferKind> Default for BlockBuffer<BS, K> {
    #[inline]
    fn default() -> Self {
        let mut buffer = Array::default();
        let mut pos = Default::default();
        K::set_pos(&mut buffer, &mut pos, 0);
        Self { buffer, pos }
    }
}

impl<BS: BlockSizes, K: BufferKind> Clone for BlockBuffer<BS, K> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            pos: self.pos.clone(),
        }
    }
}

impl<BS: BlockSizes, K: BufferKind> fmt::Debug for BlockBuffer<BS, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct(K::NAME)
            .field("pos", &self.get_pos())
            .field("block_size", &BS::USIZE)
            .field("data", &self.get_data())
            .finish()
    }
}

impl<BS: BlockSizes, K: BufferKind> BlockBuffer<BS, K> {
    /// Create new buffer from slice.
    ///
    /// # Panics
    /// If slice length is not valid for used buffer kind.
    #[inline(always)]
    #[must_use]
    #[track_caller]
    pub fn new(buf: &[u8]) -> Self {
        Self::try_new(buf).expect("invalid slice length for buffer kind")
    }

    /// Create new buffer from slice.
    ///
    /// # Errors
    /// If slice length is not valid for used buffer kind.
    #[inline(always)]
    pub fn try_new(buf: &[u8]) -> Result<Self, Error> {
        if !K::invariant(buf.len(), BS::USIZE) {
            return Err(Error);
        }
        let mut res = Self::default();
        res.set_data(buf);
        Ok(res)
    }

    /// Digest data in `input` in blocks of size `BlockSize` using
    /// the `compress` function, which accepts slice of blocks.
    #[inline]
    pub fn digest_blocks(&mut self, input: &[u8], mut compress: impl FnMut(&[Array<u8, BS>])) {
        self.digest_batches(input, |batch| match batch {
            BlockBatch::Buffered(block) => compress(slice::from_ref(block)),
            BlockBatch::Input(bytes) => {
                let (blocks, tail) = Array::slice_as_chunks(bytes);
                debug_assert!(tail.is_empty());
                compress(blocks);
            }
        });
    }

    /// Digest consecutive whole blocks without casting input to wrapped arrays.
    ///
    /// Each call to `compress` borrows a nonempty slice of complete blocks,
    /// either one completed buffered block or a contiguous run from `input`.
    #[inline]
    pub fn digest_block_bytes(&mut self, input: &[u8], mut compress: impl FnMut(&[u8])) {
        self.digest_batches(input, |batch| match batch {
            BlockBatch::Buffered(block) => compress(block.0.as_ref()),
            BlockBatch::Input(bytes) => compress(bytes),
        });
    }

    #[inline]
    fn digest_batches(&mut self, mut input: &[u8], mut compress: impl FnMut(BlockBatch<'_, BS>)) {
        let pos = self.get_pos();
        let rem = self.size() - pos;
        if K::invariant(input.len(), rem) {
            let end = pos + input.len();
            self.buffer.0.as_mut()[pos..end].copy_from_slice(input);
            self.set_pos(end);
            return;
        }
        if pos != 0 {
            let (left, right) = input.split_at(rem);
            input = right;

            let g = ResetGuard(self);
            let block = &mut g.0.buffer;
            // Filling an eager block replaces the stored cursor with the final byte.
            // The guard resets the cursor even if the callback panics.
            block.0.as_mut()[pos..].copy_from_slice(left);
            compress(BlockBatch::Buffered(block));
        }

        let size = self.size();
        let mut whole_len = input.len() - input.len() % size;
        if whole_len == input.len() && K::invariant(size, size) {
            whole_len = whole_len.saturating_sub(size);
        }
        let (whole, leftover) = input.split_at(whole_len);
        if !whole.is_empty() {
            compress(BlockBatch::Input(whole));
        }

        self.set_data(leftover);
    }

    /// Reset buffer by setting cursor position to zero.
    #[inline(always)]
    pub fn reset(&mut self) {
        self.set_pos(0);
    }

    /// Pad remaining data with zeros and return resulting block.
    #[inline(always)]
    pub fn pad_with_zeros(&mut self) -> Array<u8, BS> {
        let mut res = Array::<u8, BS>::default();
        let data = self.get_data();
        res.0.as_mut()[..data.len()].copy_from_slice(data);
        self.reset();
        res
    }

    /// Return current cursor position.
    #[inline(always)]
    pub fn get_pos(&self) -> usize {
        let pos = K::get_pos(&self.buffer, &self.pos);
        assert!(K::invariant(pos, BS::USIZE));
        pos
    }

    /// Return slice of data stored inside the buffer.
    #[inline(always)]
    pub fn get_data(&self) -> &[u8] {
        &self.buffer.0.as_ref()[..self.get_pos()]
    }

    /// Set buffer content and cursor position.
    ///
    /// # Panics
    /// If `pos` is invalid for the buffer kind.
    #[inline]
    pub fn set(&mut self, buf: Array<u8, BS>, pos: usize) {
        assert!(K::invariant(pos, BS::USIZE));
        self.buffer = buf;
        self.set_pos(pos);
    }

    /// Return size of the internal buffer in bytes.
    #[inline(always)]
    pub fn size(&self) -> usize {
        BS::USIZE
    }

    /// Return number of remaining bytes in the internal buffer.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.size() - self.get_pos()
    }

    #[inline(always)]
    fn set_pos(&mut self, pos: usize) {
        debug_assert!(K::invariant(pos, BS::USIZE));
        let pos = u8::try_from(pos).expect("buffer size is smaller than 256");
        K::set_pos(&mut self.buffer, &mut self.pos, pos);
    }

    #[inline(always)]
    fn set_data(&mut self, data: &[u8]) {
        self.buffer.0.as_mut()[..data.len()].copy_from_slice(data);
        self.set_pos(data.len());
    }
}

/// Size of serialized `BlockBuffer` in bytes.
pub type SerializedBufferSize<BS, K> = Sum<BS, <K as sealed::Sealed>::Overhead>;
/// `BlockBuffer` serialized as a byte array.
pub type SerializedBuffer<BS, K> = Array<u8, SerializedBufferSize<BS, K>>;

impl<BS: BlockSizes, K: BufferKind> BlockBuffer<BS, K>
where
    BS: core::ops::Add<K::Overhead>,
    Sum<BS, K::Overhead>: ArraySize,
{
    /// Serialize buffer into a byte array.
    #[allow(clippy::missing_panics_doc)]
    pub fn serialize(&self) -> SerializedBuffer<BS, K> {
        let mut buf = SerializedBuffer::<BS, K>::default();
        let data = self.get_data();
        let (pos, block) = buf.0.as_mut().split_at_mut(1);
        pos[0] = u8::try_from(data.len()).expect("buffer size is smaller than 256");
        block[..data.len()].copy_from_slice(data);
        buf
    }

    /// Deserialize buffer from a byte array.
    ///
    /// # Errors
    /// If `buf` does not represent a valid serialization of `BlockBuffer`.
    pub fn deserialize(buf: &SerializedBuffer<BS, K>) -> Result<Self, Error> {
        let (pos, block) = buf.0.as_ref().split_at(1);
        let pos = usize::from(pos[0]);

        if !<K as sealed::Sealed>::invariant(pos, BS::USIZE) {
            return Err(Error);
        }

        let (data, tail) = block.split_at(pos);

        if tail.iter().any(|&b| b != 0) {
            return Err(Error);
        }

        let mut res = Self::default();
        res.set_data(data);
        Ok(res)
    }
}

impl<BS: BlockSizes> BlockBuffer<BS, Eager> {
    /// Compress remaining data after padding it with `delim`, zeros and
    /// the `suffix` bytes. If there is not enough unused space, `compress`
    /// will be called twice.
    ///
    /// # Panics
    /// If suffix length is bigger than block size.
    #[inline(always)]
    pub fn digest_pad(
        &mut self,
        delim: u8,
        suffix: &[u8],
        mut compress: impl FnMut(&Array<u8, BS>),
    ) {
        let pos = self.get_pos();
        let size = self.size();
        // Number of bytes remaining in the buffer after `delim` is written to it
        // This never underflows since for eager buffers `size` is always greater than `pos`.
        let pad_len = size - pos - 1;

        let suffix_dst_pos = size
            .checked_sub(suffix.len())
            .expect("suffix must be smaller than buffer block size");

        let g = ResetGuard(self);
        let buf = &mut g.0.buffer;
        let stored = buf.0.as_mut();
        stored[pos] = delim;
        stored[pos + 1..].fill(0);

        if pad_len < suffix.len() {
            compress(buf);
            buf.0.as_mut().fill(0);
        }

        buf.0.as_mut()[suffix_dst_pos..].copy_from_slice(suffix);
        compress(buf);
    }

    /// Pad message with 0x80, zeros and 64-bit message length using
    /// big-endian byte order.
    #[inline]
    pub fn len64_padding_be(&mut self, data_len: u64, compress: impl FnMut(&Array<u8, BS>)) {
        self.digest_pad(0x80, &data_len.to_be_bytes(), compress);
    }

    /// Pad message with 0x80, zeros and 64-bit message length using
    /// little-endian byte order.
    #[inline]
    pub fn len64_padding_le(&mut self, data_len: u64, compress: impl FnMut(&Array<u8, BS>)) {
        self.digest_pad(0x80, &data_len.to_le_bytes(), compress);
    }

    /// Pad message with 0x80, zeros and 128-bit message length using
    /// big-endian byte order.
    #[inline]
    pub fn len128_padding_be(&mut self, data_len: u128, compress: impl FnMut(&Array<u8, BS>)) {
        self.digest_pad(0x80, &data_len.to_be_bytes(), compress);
    }
}

#[cfg(feature = "zeroize")]
impl<BS: BlockSizes, K: BufferKind> Zeroize for BlockBuffer<BS, K> {
    #[inline]
    fn zeroize(&mut self) {
        self.buffer.0.as_mut().zeroize();
        self.pos.zeroize();
    }
}

impl<BS: BlockSizes, K: BufferKind> Drop for BlockBuffer<BS, K> {
    #[inline]
    fn drop(&mut self) {
        #[cfg(feature = "zeroize")]
        self.zeroize();
    }
}

#[cfg(feature = "zeroize")]
impl<BS: BlockSizes, K: BufferKind> ZeroizeOnDrop for BlockBuffer<BS, K> {}

/// Resets the referenced buffer on drop.
struct ResetGuard<'a, BS: BlockSizes, K: BufferKind>(&'a mut BlockBuffer<BS, K>);

impl<BS: BlockSizes, K: BufferKind> Drop for ResetGuard<'_, BS, K> {
    fn drop(&mut self) {
        self.0.reset();
    }
}

#[cfg(all(test, feature = "zeroize"))]
mod tests {
    use super::{EagerBuffer, LazyBuffer};
    use crate::array::typenum::U4;
    use zeroize::Zeroize;

    #[test]
    fn zeroize_clears_initialized_storage_and_cursors() {
        let mut eager = EagerBuffer::<U4>::new(b"abc");
        let mut lazy = LazyBuffer::<U4>::new(b"abcd");
        eager.zeroize();
        lazy.zeroize();
        assert_eq!(eager.buffer.0, [0; 4]);
        assert_eq!(lazy.buffer.0, [0; 4]);
        assert_eq!(eager.get_pos(), 0);
        assert_eq!(lazy.get_pos(), 0);
    }
}
