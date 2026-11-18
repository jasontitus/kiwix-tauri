/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::borrow::{Borrow, Cow};
use std::rc::Rc;
use std::{cmp, fmt, hash, ops};

/// A string that borrows its input or shares an owned string across clones.
#[derive(Clone)]
pub struct CowRcStr<'a>(Repr<'a>);

#[derive(Clone)]
enum Repr<'a> {
    Borrowed(&'a str),
    Shared(Rc<String>),
}

impl<'a> From<Cow<'a, str>> for CowRcStr<'a> {
    #[inline]
    fn from(s: Cow<'a, str>) -> Self {
        match s {
            Cow::Borrowed(s) => Self::from(s),
            Cow::Owned(s) => Self::from(s),
        }
    }
}

impl<'a> From<&'a str> for CowRcStr<'a> {
    #[inline]
    fn from(s: &'a str) -> Self {
        Self(Repr::Borrowed(s))
    }
}

impl From<String> for CowRcStr<'_> {
    #[inline]
    fn from(s: String) -> Self {
        Self(Repr::Shared(Rc::new(s)))
    }
}

impl ops::Deref for CowRcStr<'_> {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        match &self.0 {
            Repr::Borrowed(s) => s,
            Repr::Shared(s) => s.as_str(),
        }
    }
}

// Boilerplate / trivial impls below.

impl AsRef<str> for CowRcStr<'_> {
    #[inline]
    fn as_ref(&self) -> &str {
        self
    }
}

impl Borrow<str> for CowRcStr<'_> {
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}

impl Default for CowRcStr<'_> {
    #[inline]
    fn default() -> Self {
        Self::from("")
    }
}

impl hash::Hash for CowRcStr<'_> {
    #[inline]
    fn hash<H: hash::Hasher>(&self, hasher: &mut H) {
        str::hash(self, hasher)
    }
}

impl<T: AsRef<str>> PartialEq<T> for CowRcStr<'_> {
    #[inline]
    fn eq(&self, other: &T) -> bool {
        str::eq(self, other.as_ref())
    }
}

impl<T: AsRef<str>> PartialOrd<T> for CowRcStr<'_> {
    #[inline]
    fn partial_cmp(&self, other: &T) -> Option<cmp::Ordering> {
        str::partial_cmp(self, other.as_ref())
    }
}

impl Eq for CowRcStr<'_> {}

impl Ord for CowRcStr<'_> {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        str::cmp(self, other)
    }
}

impl fmt::Display for CowRcStr<'_> {
    #[inline]
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        str::fmt(self, formatter)
    }
}

impl fmt::Debug for CowRcStr<'_> {
    #[inline]
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        str::fmt(self, formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_and_shared_strings_keep_their_backing_allocation() {
        let input = String::from("éclair 東京");
        let borrowed = CowRcStr::from(input.as_str());
        assert_eq!(borrowed.as_ptr(), input.as_ptr());
        assert_eq!(borrowed.clone().as_ptr(), input.as_ptr());

        let address = input.as_ptr();
        let owned = CowRcStr::from(Cow::Owned(input));
        let clone = owned.clone();
        drop(owned);
        assert_eq!(clone.as_ptr(), address);
        assert_eq!(&*clone, "éclair 東京");
    }
}
