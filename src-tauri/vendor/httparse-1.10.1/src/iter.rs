// Modified by Kiwix Tauri. Track the parser position with checked slices.

#![forbid(unsafe_code)]

use core::convert::{TryFrom, TryInto};

#[allow(missing_docs)]
pub struct Bytes<'a> {
    committed: &'a [u8],
    remaining: &'a [u8],
}

#[allow(missing_docs)]
impl<'a> Bytes<'a> {
    #[inline]
    pub fn new(slice: &'a [u8]) -> Self {
        Self {
            committed: slice,
            remaining: slice,
        }
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.committed.len() - self.remaining.len()
    }

    #[inline]
    pub fn peek(&self) -> Option<u8> {
        self.remaining.first().copied()
    }

    #[inline]
    pub fn peek_ahead(&self, offset: usize) -> Option<u8> {
        self.remaining.get(offset).copied()
    }

    #[inline]
    pub fn peek_n<U: TryFrom<&'a [u8]>>(&self, count: usize) -> Option<U> {
        self.remaining.get(..count)?.try_into().ok()
    }

    #[inline]
    pub fn bump(&mut self) {
        self.advance(1)
    }

    #[inline]
    pub fn advance(&mut self, count: usize) {
        self.remaining = &self.remaining[count..];
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.remaining.len()
    }

    #[inline]
    pub fn slice(&mut self) -> &'a [u8] {
        self.slice_skip(0)
    }

    #[inline]
    pub fn slice_skip(&mut self, skip: usize) -> &'a [u8] {
        let part = &self.committed[..self.pos() - skip];
        self.commit();
        part
    }

    #[inline]
    fn commit(&mut self) {
        self.committed = self.remaining;
    }

    #[inline]
    pub fn advance_and_commit(&mut self, count: usize) {
        self.advance(count);
        self.commit();
    }
}

impl AsRef<[u8]> for Bytes<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.remaining
    }
}

impl Iterator for Bytes<'_> {
    type Item = u8;

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let (&byte, rest) = self.remaining.split_first()?;
        self.remaining = rest;
        Some(byte)
    }
}
