// Modified by Kiwix to access the cursor through initialized array storage.
use hybrid_array::{
    sizes::{U0, U1, U256},
    typenum::{IsLess, NonZero, True, Unsigned},
};

use super::{Array, ArraySize};

/// Sealed trait for buffer kinds.
pub trait Sealed {
    #[cfg(not(feature = "zeroize"))]
    type Pos: Default + Clone;
    #[cfg(feature = "zeroize")]
    type Pos: Default + Clone + zeroize::Zeroize;

    type Overhead: ArraySize;

    const NAME: &'static str;

    fn get_pos<N: ArraySize>(buf: &Array<u8, N>, pos: &Self::Pos) -> usize;

    fn set_pos<N: ArraySize>(buf: &mut Array<u8, N>, pos: &mut Self::Pos, val: u8);

    /// Invariant guaranteed by a buffer kind, i.e. with correct
    /// buffer code this function always returns true.
    fn invariant(pos: usize, block_size: usize) -> bool;

    /// Split input data into slice of blocks and tail.
    fn split_blocks<N: ArraySize>(data: &[u8]) -> (&[Array<u8, N>], &[u8]);
}

impl Sealed for super::Eager {
    type Pos = ();
    type Overhead = U0;
    const NAME: &'static str = "BlockBuffer<Eager>";

    fn get_pos<N: ArraySize>(buf: &Array<u8, N>, _pos: &Self::Pos) -> usize {
        usize::from(buf.0.as_ref()[N::USIZE - 1])
    }

    fn set_pos<N: ArraySize>(buf: &mut Array<u8, N>, _pos: &mut Self::Pos, val: u8) {
        buf.0.as_mut()[N::USIZE - 1] = val;
    }

    #[inline(always)]
    fn invariant(pos: usize, block_size: usize) -> bool {
        pos < block_size
    }

    #[inline(always)]
    fn split_blocks<N: ArraySize>(data: &[u8]) -> (&[Array<u8, N>], &[u8]) {
        Array::slice_as_chunks(data)
    }
}

impl Sealed for super::Lazy {
    type Pos = u8;
    type Overhead = U1;
    const NAME: &'static str = "BlockBuffer<Lazy>";

    fn get_pos<N: ArraySize>(_buf_val: &Array<u8, N>, pos: &Self::Pos) -> usize {
        usize::from(*pos)
    }

    fn set_pos<N: ArraySize>(_: &mut Array<u8, N>, pos: &mut Self::Pos, val: u8) {
        *pos = val;
    }

    #[inline(always)]
    fn invariant(pos: usize, block_size: usize) -> bool {
        pos <= block_size
    }

    #[inline(always)]
    fn split_blocks<N: ArraySize>(data: &[u8]) -> (&[Array<u8, N>], &[u8]) {
        let (blocks, tail) = Array::slice_as_chunks(data);
        if data.is_empty() || !tail.is_empty() {
            (blocks, tail)
        } else {
            let (tail, blocks) = blocks.split_last().expect("`blocks` can not be empty");
            (blocks, tail)
        }
    }
}

/// Sealed trait implemented for sizes from `U1` to `U255`.
pub trait BlockSizes {}

impl<T: Unsigned> BlockSizes for T where Self: IsLess<U256, Output = True> + NonZero {}
