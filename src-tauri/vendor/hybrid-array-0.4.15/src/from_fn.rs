//! Support for constructing arrays using a provided generator function.

// Modified by Kiwix to use native array construction and checked prefix access.

use crate::{Array, ArraySize};
use core::{
    mem::{self, MaybeUninit},
    ptr,
};

pub trait NativeArrayInit<T> {
    fn from_generator(f: impl FnMut(usize) -> T) -> Self;
}

impl<T, const N: usize> NativeArrayInit<T> for [T; N] {
    #[inline]
    fn from_generator(f: impl FnMut(usize) -> T) -> Self {
        core::array::from_fn(f)
    }
}

impl<T, U> Array<T, U>
where
    U: ArraySize,
{
    /// Create array where each array element `T` is returned by the `f` call.
    #[inline]
    pub fn from_fn(mut f: impl FnMut(usize) -> T) -> Self {
        let array = Self(NativeArrayInit::from_generator(&mut f));
        // Keep the completed array owned here if dropping the generator panics.
        drop(f);
        array
    }

    /// Create array fallibly where each array element `T` is returned by the `f` call, or return
    /// an error if any are encountered.
    ///
    /// # Errors
    ///
    /// Propagates the `E` type returned from the provided `F` in the event of error.
    pub fn try_from_fn<E>(mut f: impl FnMut(usize) -> Result<T, E>) -> Result<Self, E> {
        let mut array = Array::<MaybeUninit<T>, U>::uninit();
        let result = try_from_fn_erased(array.0.as_mut(), &mut f).map(|()| {
            // SAFETY: the generator successfully initialized every element.
            unsafe { array.assume_init() }
        });
        // Keep either result owned here if dropping the generator panics.
        drop(f);
        result
    }
}

/// Fills a `MaybeUninit` slice using the given fallible generator function.
///
/// Using a slice avoids monomorphizing for each array size.
#[inline]
fn try_from_fn_erased<T, E, F>(buffer: &mut [MaybeUninit<T>], mut f: F) -> Result<(), E>
where
    F: FnMut(usize) -> Result<T, E>,
{
    let mut guard = Guard {
        array_mut: buffer,
        initialized: 0,
    };

    while guard.initialized < guard.array_mut.len() {
        let item = f(guard.initialized)?;

        guard.push(item);
    }

    mem::forget(guard);
    Ok(())
}

/// Drop guard which tracks the total number of initialized items, and handles dropping them in
/// the event a panic occurs.
///
/// Use `mem::forget` when the array has been fully constructed.
struct Guard<'a, T> {
    /// Array being constructed.
    array_mut: &'a mut [MaybeUninit<T>],

    /// Number of items in the array which have been initialized.
    initialized: usize,
}

impl<T> Guard<'_, T> {
    /// Write one additional item and extend the initialized prefix.
    #[inline]
    fn push(&mut self, item: T) {
        self.array_mut[self.initialized].write(item);
        self.initialized = self.initialized.saturating_add(1);
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        let prefix = &mut self.array_mut[..self.initialized];
        // Drop the slice as a unit so the remaining elements are dropped if one panics.
        // SAFETY: push only advances the cursor after writing each slot. MaybeUninit<T>
        // has the same layout as T, and the exclusive slice covers exactly these slots.
        unsafe { ptr::drop_in_place(ptr::from_mut(prefix) as *mut [T]) };
    }
}
