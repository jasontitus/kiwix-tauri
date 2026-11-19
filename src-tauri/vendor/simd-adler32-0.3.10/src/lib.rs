//! The simd-adler32 checksum API backed by checked Rust.
//!
//! This Workbench fork uses `adler2` for every target, including `no_std`.
//! `std` enables the stream helpers. `const-generics` enables arbitrary array
//! lengths for the one-shot API. `nightly` is retained for compatibility.
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

#[doc(hidden)]
pub mod hash;
#[doc(hidden)]
pub mod imp;


/// An adler32 hash generator type.
#[derive(Clone)]
pub struct Adler32(adler2::Adler32);

impl Adler32 {
  /// Constructs a new `Adler32`.
  ///
  ///
  /// # Examples
  /// ```rust
  /// use simd_adler32::Adler32;
  ///
  /// let mut adler = Adler32::new();
  /// ```
  pub fn new() -> Self {
    Default::default()
  }

  /// Constructs a new `Adler32` using existing checksum.
  ///
  ///
  /// # Examples
  /// ```rust
  /// use simd_adler32::Adler32;
  ///
  /// let mut adler = Adler32::from_checksum(0xdeadbeaf);
  /// ```
  pub fn from_checksum(checksum: u32) -> Self {
    Self(adler2::Adler32::from_checksum(checksum))
  }

  /// Computes hash for supplied data and stores results in internal state.
  pub fn write(&mut self, data: &[u8]) {
    self.0.write_slice(data);
  }

  /// Returns the hash value for the values written so far.
  ///
  /// Despite its name, the method does not reset the hasher’s internal state. Additional
  /// writes will continue from the current value. If you need to start a fresh hash
  /// value, you will have to use `reset`.
  pub fn finish(&self) -> u32 {
    self.0.checksum()
  }

  /// Resets the internal state.
  pub fn reset(&mut self) {
    *self = Self::default();
  }
}

/// Compute Adler-32 hash on `Adler32Hash` type.
///
/// # Arguments
/// * `hash` - A Adler-32 hash-able type.
///
/// # Examples
/// ```rust
/// use simd_adler32::adler32;
///
/// let hash = adler32(b"Adler-32");
/// println!("{}", hash); // 800813569
/// ```
pub fn adler32<H: Adler32Hash>(hash: &H) -> u32 {
  hash.hash()
}

/// A Adler-32 hash-able type.
pub trait Adler32Hash {
  /// Feeds this value into `Adler32`.
  fn hash(&self) -> u32;
}

impl Default for Adler32 {
  fn default() -> Self {
    Self(adler2::Adler32::new())
  }
}

#[cfg(feature = "std")]
pub mod read {
  //! Reader-based hashing.
  //!
  //! # Example
  //! ```rust
  //! use std::io::Cursor;
  //! use simd_adler32::read::adler32;
  //!
  //! let mut reader = Cursor::new(b"Hello there");
  //! let hash = adler32(&mut reader).unwrap();
  //!
  //! println!("{}", hash) // 800813569
  //! ```
  use crate::Adler32;
  use std::io::{Read, Result};

  /// Compute Adler-32 hash on reader until EOF.
  ///
  /// # Example
  /// ```rust
  /// use std::io::Cursor;
  /// use simd_adler32::read::adler32;
  ///
  /// let mut reader = Cursor::new(b"Hello there");
  /// let hash = adler32(&mut reader).unwrap();
  ///
  /// println!("{}", hash) // 800813569
  /// ```
  pub fn adler32<R: Read>(reader: &mut R) -> Result<u32> {
    let mut hash = Adler32::new();
    let mut buf = [0; 4096];

    loop {
      match reader.read(&mut buf) {
        Ok(0) => return Ok(hash.finish()),
        Ok(n) => {
          hash.write(&buf[..n]);
        }
        Err(err) => return Err(err),
      }
    }
  }
}

#[cfg(feature = "std")]
pub mod bufread {
  //! BufRead-based hashing.
  //!
  //! Separate `BufRead` trait implemented to allow for custom buffer size optimization.
  //!
  //! # Example
  //! ```rust
  //! use std::io::{Cursor, BufReader};
  //! use simd_adler32::bufread::adler32;
  //!
  //! let mut reader = Cursor::new(b"Hello there");
  //! let mut reader = BufReader::new(reader);
  //! let hash = adler32(&mut reader).unwrap();
  //!
  //! println!("{}", hash) // 800813569
  //! ```
  use crate::Adler32;
  use std::io::{BufRead, ErrorKind, Result};

  /// Compute Adler-32 hash on buf reader until EOF.
  ///
  /// # Example
  /// ```rust
  /// use std::io::{Cursor, BufReader};
  /// use simd_adler32::bufread::adler32;
  ///
  /// let mut reader = Cursor::new(b"Hello there");
  /// let mut reader = BufReader::new(reader);
  /// let hash = adler32(&mut reader).unwrap();
  ///
  /// println!("{}", hash) // 800813569
  /// ```
  pub fn adler32<R: BufRead>(reader: &mut R) -> Result<u32> {
    let mut hash = Adler32::new();

    loop {
      let consumed = match reader.fill_buf() {
        Ok(buf) => {
          if buf.is_empty() {
            return Ok(hash.finish());
          }

          hash.write(buf);
          buf.len()
        }
        Err(err) => match err.kind() {
          ErrorKind::Interrupted => continue,
          ErrorKind::UnexpectedEof => return Ok(hash.finish()),
          _ => return Err(err),
        },
      };

      reader.consume(consumed);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::Adler32;

  fn reference(initial: u32, data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = (initial & 0xffff) % MOD;
    let mut b = (initial >> 16) % MOD;
    for &byte in data {
      a = (a + u32::from(byte)) % MOD;
      b = (b + a) % MOD;
    }
    b << 16 | a
  }

  #[test]
  fn seeded_writes_match_reference_at_chunk_limits() {
    let data = [255; 22500];
    for seed in [0, 1, 0xdeadbeaf, 0xffffffff] {
      for len in [0, 3, 4, 5551, 5552, 5553, 22207, 22208, 22209, data.len()] {
        let bytes = &data[..len];
        let mut checksum = Adler32::from_checksum(seed);
        assert_eq!(checksum.finish(), seed);
        checksum.write(bytes);
        assert_eq!(checksum.finish(), reference(seed, bytes));

        let mut resumed = Adler32::from_checksum(seed);
        resumed.write(&bytes[..len / 2]);
        let mut resumed = Adler32::from_checksum(resumed.finish());
        resumed.write(&bytes[len / 2..]);
        assert_eq!(resumed.finish(), checksum.finish());
        resumed.reset();
        assert_eq!(resumed.finish(), 1);
      }
    }
  }
}
