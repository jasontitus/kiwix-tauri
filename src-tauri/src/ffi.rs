// SPDX-License-Identifier: GPL-3.0-or-later
//! Proof-carrying values for the remaining synchronous C string bridges.
//!
//! A `CPath` can only be built from a live `CStr`, and its lifetime keeps that
//! string borrowed through the call. The native functions below read the path
//! synchronously and retain no pointer, so a safe declaration is sound.
#![forbid(unsafe_code)]

use std::{ffi::CStr, marker::PhantomData, os::raw::c_char};

#[repr(transparent)]
pub(crate) struct CPath<'a> {
    pointer: *const c_char,
    lifetime: PhantomData<&'a CStr>,
}

impl<'a> CPath<'a> {
    pub(crate) fn new(path: &'a CStr) -> Self {
        Self { pointer: path.as_ptr(), lifetime: PhantomData }
    }
}
