// Copyright 2014 The Servo Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//!
//! String atoms with generated static names, inline short names and shared ownership.
//!
//! Some strings may be interned at compile time using the `string-cache-codegen` crate, or the
//! `EmptyStaticAtomSet` may be used that has no compile-time interned strings. An `Atom` is an
//! interned string for a given set (either `EmptyStaticAtomSet` or a generated `StaticAtomSet`).
//!
//! Generated `Atom`s will have assocated macros to intern static strings at compile-time.
//!
//! # Examples
//!
//! Here are two examples, one with compile-time `Atom`s, and one without.
//!
//! ## With compile-time atoms
//!
//! In `Cargo.toml`:
//! ```toml
//! [dependencies]
//! string_cache = "0.9"
//!
//! [dev-dependencies]
//! string_cache_codegen = "0.6"
//! ```
//!
//! In `build.rs`:
//!
//! ```ignore
//! extern crate string_cache_codegen;
//!
//! use std::env;
//! use std::path::Path;
//!
//! fn main() {
//!     string_cache_codegen::AtomType::new("foo::FooAtom", "foo_atom!")
//!         .atoms(&["foo", "bar"])
//!         .write_to_file(&Path::new(&env::var("OUT_DIR").unwrap()).join("foo_atom.rs"))
//!         .unwrap()
//! }
//! ```
//!
//! In `lib.rs`:
//!
//! ```ignore
//! extern crate string_cache;
//!
//! mod foo {
//!     include!(concat!(env!("OUT_DIR"), "/foo_atom.rs"));
//! }
//!
//! fn use_the_atom(t: &str) {
//!     match foo::FooAtom::from(t) {
//!         foo_atom!("foo") => println!("Found foo!"),
//!         foo_atom!("bar") => println!("Found bar!"),
//!         _ => println!("Name outside the generated list"),
//!     }
//! }
//! ```
//!
//! ## No compile-time atoms
//!
//! ```
//! # extern crate string_cache;
//! use string_cache::DefaultAtom;
//!
//! # fn main() {
//! let mut interned_stuff = Vec::new();
//! let text = "here is a sentence of text that will be tokenised and
//!             interned and some repeated tokens is of text and";
//! for word in text.split_whitespace() {
//!     let seen_before = interned_stuff.iter()
//!         // Atoms compare by name, including separately constructed long names.
//!         // This also compares an atom to a plain string.
//!         .filter(|interned_word| interned_word == &word)
//!         .count();
//!     if seen_before > 0 {
//!         println!(r#"Seen the word "{}" {} times"#, word, seen_before);
//!     } else {
//!         println!(r#"Not seen the word "{}" before"#, word);
//!     }
//!     // We use the impl From<(Cow<'a, str>, or &'a str, or String)> for
//!     // Atom<Static> to intern a new string.
//!     interned_stuff.push(DefaultAtom::from(word));
//! }
//! # }
//! ```
//!

#![forbid(unsafe_code)]

mod atom;
mod static_sets;
mod trivial_impls;

pub use atom::Atom;
pub use static_sets::{EmptyStaticAtomSet, PhfStrSet, StaticAtomSet};

/// Use this if you don’t care about static atoms.
pub type DefaultAtom = Atom<EmptyStaticAtomSet>;
