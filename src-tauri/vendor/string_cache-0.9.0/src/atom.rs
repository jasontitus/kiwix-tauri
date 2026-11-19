// Copyright 2014 The Servo Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.

use crate::StaticAtomSet;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;

const MAX_INLINE_LEN: usize = 7;

#[derive(PartialEq, Eq)]
enum Name {
    Static(u32),
    Inline([u8; 8]),
    Owned(Arc<OwnedName>),
}

#[derive(PartialEq, Eq)]
struct OwnedName {
    hash: u32,
    text: Box<str>,
}

/// A generated static name, a short UTF-8 name, or shared owned UTF-8.
/// Independently parsed long names compare by value. They live only as long
/// as an atom that owns them, with no process-global table.
#[derive(PartialEq, Eq)]
pub struct Atom<Static> {
    name: Name,
    set: PhantomData<Static>,
}

impl<Static> Atom<Static> {
    fn new(name: Name) -> Self {
        Self {
            name,
            set: PhantomData,
        }
    }

    /// For constants emitted by string_cache_codegen.
    #[doc(hidden)]
    pub const fn pack_static(index: u32) -> Self {
        Self {
            name: Name::Static(index),
            set: PhantomData,
        }
    }

    /// For constants emitted by string_cache_codegen.
    #[doc(hidden)]
    pub const fn pack_inline(value: u64, len: u8) -> Self {
        assert!(len > 0 && len as usize <= MAX_INLINE_LEN);
        let mut bytes = value.to_le_bytes();
        bytes[0] = 1 | (len << 4);
        let mut padding = 1 + len as usize;
        while padding < bytes.len() {
            bytes[padding] = 0;
            padding += 1;
        }
        let (_, text) = bytes.split_at(1);
        let (text, _) = text.split_at(len as usize);
        assert!(std::str::from_utf8(text).is_ok());
        Self {
            name: Name::Inline(bytes),
            set: PhantomData,
        }
    }
}

impl<Static: StaticAtomSet> Atom<Static> {
    #[doc(hidden)]
    pub fn is_static(&self) -> bool {
        matches!(self.name, Name::Static(_))
    }

    #[doc(hidden)]
    pub fn is_dynamic(&self) -> bool {
        matches!(self.name, Name::Owned(_))
    }

    #[doc(hidden)]
    pub fn is_inline(&self) -> bool {
        matches!(self.name, Name::Inline(_))
    }

    #[inline]
    pub fn get_hash(&self) -> u32 {
        match &self.name {
            Name::Static(index) => Static::get().hashes[*index as usize],
            Name::Inline(bytes) => {
                let value = u64::from_le_bytes(*bytes);
                ((value >> 32) ^ value) as u32
            }
            Name::Owned(name) => name.hash,
        }
    }

    pub fn try_static(text: &str) -> Option<Self> {
        Self::lookup(text).ok()
    }

    #[inline]
    fn lookup(text: &str) -> Result<Self, u32> {
        let set = Static::get();
        let hash = phf_shared::hash(text, &set.key);
        let index = phf_shared::get_index(&hash, set.disps, set.atoms.len());
        if set.atoms[index as usize] == text {
            Ok(Self::pack_static(index))
        } else {
            Err(hash.g)
        }
    }

    pub fn to_ascii_uppercase(&self) -> Self {
        if self.bytes().any(|b| b.is_ascii_lowercase()) {
            Self::from(str::to_ascii_uppercase(self))
        } else {
            self.clone()
        }
    }

    pub fn to_ascii_lowercase(&self) -> Self {
        if self.bytes().any(|b| b.is_ascii_uppercase()) {
            Self::from(str::to_ascii_lowercase(self))
        } else {
            self.clone()
        }
    }

    pub fn eq_ignore_ascii_case(&self, other: &Self) -> bool {
        self == other || self.eq_str_ignore_ascii_case(other)
    }

    pub fn eq_str_ignore_ascii_case(&self, other: &str) -> bool {
        str::eq_ignore_ascii_case(self, other)
    }
}

impl<Static: StaticAtomSet> Default for Atom<Static> {
    fn default() -> Self {
        Self::pack_static(Static::empty_string_index())
    }
}

impl<Static: StaticAtomSet> Hash for Atom<Static> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(self.get_hash());
    }
}

impl<Static: StaticAtomSet> From<Cow<'_, str>> for Atom<Static> {
    #[inline]
    fn from(text: Cow<'_, str>) -> Self {
        let len = text.len();
        if len == 0 {
            Self::default()
        } else if len <= MAX_INLINE_LEN {
            let mut bytes = [0; 8];
            bytes[0] = 1 | ((len as u8) << 4);
            bytes[1..=len].copy_from_slice(text.as_bytes());
            Self::new(Name::Inline(bytes))
        } else {
            Self::lookup(&text).unwrap_or_else(|hash| {
                Self::new(Name::Owned(Arc::new(OwnedName {
                    hash,
                    text: text.into_owned().into_boxed_str(),
                })))
            })
        }
    }
}

impl<Static: StaticAtomSet> Clone for Atom<Static> {
    #[inline]
    fn clone(&self) -> Self {
        Self::new(match &self.name {
            Name::Static(index) => Name::Static(*index),
            Name::Inline(bytes) => Name::Inline(*bytes),
            Name::Owned(name) => Name::Owned(Arc::clone(name)),
        })
    }
}

impl<Static: StaticAtomSet> Deref for Atom<Static> {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        match &self.name {
            Name::Static(index) => Static::get().atoms[*index as usize],
            Name::Inline(bytes) => {
                let len = usize::from(bytes[0] >> 4);
                std::str::from_utf8(&bytes[1..=len]).expect("invalid UTF-8 in inline atom")
            }
            Name::Owned(name) => &name.text,
        }
    }
}

impl<Static: StaticAtomSet> fmt::Debug for Atom<Static> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.name {
            Name::Static(_) => "static",
            Name::Inline(_) => "inline",
            Name::Owned(_) => "dynamic",
        };
        write!(f, "Atom('{}' type={})", self.as_ref(), kind)
    }
}

impl<Static: StaticAtomSet> PartialOrd for Atom<Static> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<Static: StaticAtomSet> Ord for Atom<Static> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

#[cfg(feature = "malloc_size_of")]
impl<Static: StaticAtomSet> malloc_size_of::MallocSizeOf for Atom<Static> {
    fn size_of(&self, _ops: &mut malloc_size_of::MallocSizeOfOps) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{Atom, Name};
    use crate::DefaultAtom;
    use std::sync::Arc;

    #[test]
    fn inline_constants_match_runtime_utf8_and_have_canonical_padding() {
        // A generated constant uses little-endian byte order irrespective of the host.
        const CRAB: DefaultAtom = Atom::pack_inline(0xffff_ff80_a69f_f000, 4);
        let atom = DefaultAtom::from("🦀");
        assert!(matches!(atom, CRAB));
        assert_eq!(&*atom, "🦀");
        assert_eq!(atom.get_hash(), CRAB.get_hash());
    }

    #[test]
    fn long_names_compare_by_value_and_release_the_last_owned_allocation() {
        let first = DefaultAtom::from("custom-long-Namé");
        let same = DefaultAtom::from(String::from("custom-long-Namé"));
        let Name::Owned(ref data) = first.name else {
            panic!("expected an owned name")
        };
        let observer = Arc::downgrade(data);
        assert_eq!(first, same);
        assert_eq!(first.get_hash(), same.get_hash());
        assert_eq!(first.cmp(&same), std::cmp::Ordering::Equal);
        let clone = first.clone();
        drop(first);
        assert!(observer.upgrade().is_some());
        assert_eq!(&*clone.to_ascii_lowercase(), "custom-long-namé");
        assert_eq!(&*clone.to_ascii_uppercase(), "CUSTOM-LONG-NAMé");
        drop(clone);
        assert!(observer.upgrade().is_none());
        assert_eq!(&*same, "custom-long-Namé");
    }
}
