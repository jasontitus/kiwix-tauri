use std::{
    io::{self, Result},
    mem::size_of,
};

use crate::ByteOrder;

/// Extends [`Read`] with methods for reading numbers. (For `std::io`.)
///
/// Most of the methods defined here have an unconstrained type parameter that
/// must be explicitly instantiated. Typically, it is instantiated with either
/// the [`BigEndian`] or [`LittleEndian`] types defined in this crate.
///
/// # Examples
///
/// Read unsigned 16 bit big-endian integers from a [`Read`]:
///
/// ```rust
/// use std::io::Cursor;
/// use byteorder::{BigEndian, ReadBytesExt};
///
/// let mut rdr = Cursor::new(vec![2, 5, 3, 0]);
/// assert_eq!(517, rdr.read_u16::<BigEndian>().unwrap());
/// assert_eq!(768, rdr.read_u16::<BigEndian>().unwrap());
/// ```
///
/// [`BigEndian`]: enum.BigEndian.html
/// [`LittleEndian`]: enum.LittleEndian.html
/// [`Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
pub trait ReadBytesExt: io::Read {
    /// Reads an unsigned 8 bit integer from the underlying reader.
    ///
    /// Note that since this reads a single byte, no byte order conversions
    /// are used. It is included for completeness.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read unsigned 8 bit integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::ReadBytesExt;
    ///
    /// let mut rdr = Cursor::new(vec![2, 5]);
    /// assert_eq!(2, rdr.read_u8().unwrap());
    /// assert_eq!(5, rdr.read_u8().unwrap());
    /// ```
    #[inline]
    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    /// Reads a signed 8 bit integer from the underlying reader.
    ///
    /// Note that since this reads a single byte, no byte order conversions
    /// are used. It is included for completeness.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read signed 8 bit integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::ReadBytesExt;
    ///
    /// let mut rdr = Cursor::new(vec![0x02, 0xfb]);
    /// assert_eq!(2, rdr.read_i8().unwrap());
    /// assert_eq!(-5, rdr.read_i8().unwrap());
    /// ```
    #[inline]
    fn read_i8(&mut self) -> Result<i8> {
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0] as i8)
    }

    /// Reads an unsigned 16 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read unsigned 16 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![2, 5, 3, 0]);
    /// assert_eq!(517, rdr.read_u16::<BigEndian>().unwrap());
    /// assert_eq!(768, rdr.read_u16::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_u16<T: ByteOrder>(&mut self) -> Result<u16> {
        let mut buf = [0; 2];
        self.read_exact(&mut buf)?;
        Ok(T::read_u16(&buf))
    }

    /// Reads a signed 16 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read signed 16 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x00, 0xc1, 0xff, 0x7c]);
    /// assert_eq!(193, rdr.read_i16::<BigEndian>().unwrap());
    /// assert_eq!(-132, rdr.read_i16::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_i16<T: ByteOrder>(&mut self) -> Result<i16> {
        let mut buf = [0; 2];
        self.read_exact(&mut buf)?;
        Ok(T::read_i16(&buf))
    }

    /// Reads an unsigned 24 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read unsigned 24 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x00, 0x01, 0x0b]);
    /// assert_eq!(267, rdr.read_u24::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_u24<T: ByteOrder>(&mut self) -> Result<u32> {
        let mut buf = [0; 3];
        self.read_exact(&mut buf)?;
        Ok(T::read_u24(&buf))
    }

    /// Reads a signed 24 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read signed 24 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0xff, 0x7a, 0x33]);
    /// assert_eq!(-34253, rdr.read_i24::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_i24<T: ByteOrder>(&mut self) -> Result<i32> {
        let mut buf = [0; 3];
        self.read_exact(&mut buf)?;
        Ok(T::read_i24(&buf))
    }

    /// Reads an unsigned 32 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read unsigned 32 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x00, 0x00, 0x01, 0x0b]);
    /// assert_eq!(267, rdr.read_u32::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_u32<T: ByteOrder>(&mut self) -> Result<u32> {
        let mut buf = [0; 4];
        self.read_exact(&mut buf)?;
        Ok(T::read_u32(&buf))
    }

    /// Reads a signed 32 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read signed 32 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0xff, 0xff, 0x7a, 0x33]);
    /// assert_eq!(-34253, rdr.read_i32::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_i32<T: ByteOrder>(&mut self) -> Result<i32> {
        let mut buf = [0; 4];
        self.read_exact(&mut buf)?;
        Ok(T::read_i32(&buf))
    }

    /// Reads an unsigned 48 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read unsigned 48 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0xb6, 0x71, 0x6b, 0xdc, 0x2b, 0x31]);
    /// assert_eq!(200598257150769, rdr.read_u48::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_u48<T: ByteOrder>(&mut self) -> Result<u64> {
        let mut buf = [0; 6];
        self.read_exact(&mut buf)?;
        Ok(T::read_u48(&buf))
    }

    /// Reads a signed 48 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read signed 48 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x9d, 0x71, 0xab, 0xe7, 0x97, 0x8f]);
    /// assert_eq!(-108363435763825, rdr.read_i48::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_i48<T: ByteOrder>(&mut self) -> Result<i64> {
        let mut buf = [0; 6];
        self.read_exact(&mut buf)?;
        Ok(T::read_i48(&buf))
    }

    /// Reads an unsigned 64 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read an unsigned 64 bit big-endian integer from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x00, 0x03, 0x43, 0x95, 0x4d, 0x60, 0x86, 0x83]);
    /// assert_eq!(918733457491587, rdr.read_u64::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_u64<T: ByteOrder>(&mut self) -> Result<u64> {
        let mut buf = [0; 8];
        self.read_exact(&mut buf)?;
        Ok(T::read_u64(&buf))
    }

    /// Reads a signed 64 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a signed 64 bit big-endian integer from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x80, 0, 0, 0, 0, 0, 0, 0]);
    /// assert_eq!(i64::min_value(), rdr.read_i64::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_i64<T: ByteOrder>(&mut self) -> Result<i64> {
        let mut buf = [0; 8];
        self.read_exact(&mut buf)?;
        Ok(T::read_i64(&buf))
    }

    /// Reads an unsigned 128 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read an unsigned 128 bit big-endian integer from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x00, 0x03, 0x43, 0x95, 0x4d, 0x60, 0x86, 0x83,
    ///     0x00, 0x03, 0x43, 0x95, 0x4d, 0x60, 0x86, 0x83
    /// ]);
    /// assert_eq!(16947640962301618749969007319746179, rdr.read_u128::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_u128<T: ByteOrder>(&mut self) -> Result<u128> {
        let mut buf = [0; 16];
        self.read_exact(&mut buf)?;
        Ok(T::read_u128(&buf))
    }

    /// Reads a signed 128 bit integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a signed 128 bit big-endian integer from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    /// assert_eq!(i128::min_value(), rdr.read_i128::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_i128<T: ByteOrder>(&mut self) -> Result<i128> {
        let mut buf = [0; 16];
        self.read_exact(&mut buf)?;
        Ok(T::read_i128(&buf))
    }

    /// Reads an unsigned n-bytes integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read an unsigned n-byte big-endian integer from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0x80, 0x74, 0xfa]);
    /// assert_eq!(8418554, rdr.read_uint::<BigEndian>(3).unwrap());
    #[inline]
    fn read_uint<T: ByteOrder>(&mut self, nbytes: usize) -> Result<u64> {
        let mut buf = [0; 8];
        self.read_exact(&mut buf[..nbytes])?;
        Ok(T::read_uint(&buf[..nbytes], nbytes))
    }

    /// Reads a signed n-bytes integer from the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read an unsigned n-byte big-endian integer from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0xc1, 0xff, 0x7c]);
    /// assert_eq!(-4063364, rdr.read_int::<BigEndian>(3).unwrap());
    #[inline]
    fn read_int<T: ByteOrder>(&mut self, nbytes: usize) -> Result<i64> {
        let mut buf = [0; 8];
        self.read_exact(&mut buf[..nbytes])?;
        Ok(T::read_int(&buf[..nbytes], nbytes))
    }

    /// Reads an unsigned n-bytes integer from the underlying reader.
    #[inline]
    fn read_uint128<T: ByteOrder>(&mut self, nbytes: usize) -> Result<u128> {
        let mut buf = [0; 16];
        self.read_exact(&mut buf[..nbytes])?;
        Ok(T::read_uint128(&buf[..nbytes], nbytes))
    }

    /// Reads a signed n-bytes integer from the underlying reader.
    #[inline]
    fn read_int128<T: ByteOrder>(&mut self, nbytes: usize) -> Result<i128> {
        let mut buf = [0; 16];
        self.read_exact(&mut buf[..nbytes])?;
        Ok(T::read_int128(&buf[..nbytes], nbytes))
    }

    /// Reads a IEEE754 single-precision (4 bytes) floating point number from
    /// the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a big-endian single-precision floating point number from a `Read`:
    ///
    /// ```rust
    /// use std::f32;
    /// use std::io::Cursor;
    ///
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x40, 0x49, 0x0f, 0xdb,
    /// ]);
    /// assert_eq!(f32::consts::PI, rdr.read_f32::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_f32<T: ByteOrder>(&mut self) -> Result<f32> {
        let mut buf = [0; 4];
        self.read_exact(&mut buf)?;
        Ok(T::read_f32(&buf))
    }

    /// Reads a IEEE754 double-precision (8 bytes) floating point number from
    /// the underlying reader.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a big-endian double-precision floating point number from a `Read`:
    ///
    /// ```rust
    /// use std::f64;
    /// use std::io::Cursor;
    ///
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x40, 0x09, 0x21, 0xfb, 0x54, 0x44, 0x2d, 0x18,
    /// ]);
    /// assert_eq!(f64::consts::PI, rdr.read_f64::<BigEndian>().unwrap());
    /// ```
    #[inline]
    fn read_f64<T: ByteOrder>(&mut self) -> Result<f64> {
        let mut buf = [0; 8];
        self.read_exact(&mut buf)?;
        Ok(T::read_f64(&buf))
    }

    /// Reads a sequence of unsigned 16 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of unsigned 16 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![2, 5, 3, 0]);
    /// let mut dst = [0; 2];
    /// rdr.read_u16_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_u16_into<T: ByteOrder>(&mut self, dst: &mut [u16]) -> Result<()> {
        read_ordered(self, dst, T::read_u16_into)
    }

    /// Reads a sequence of unsigned 32 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of unsigned 32 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0, 0, 2, 5, 0, 0, 3, 0]);
    /// let mut dst = [0; 2];
    /// rdr.read_u32_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_u32_into<T: ByteOrder>(&mut self, dst: &mut [u32]) -> Result<()> {
        read_ordered(self, dst, T::read_u32_into)
    }

    /// Reads a sequence of unsigned 64 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of unsigned 64 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0, 0, 0, 0, 0, 0, 2, 5,
    ///     0, 0, 0, 0, 0, 0, 3, 0,
    /// ]);
    /// let mut dst = [0; 2];
    /// rdr.read_u64_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_u64_into<T: ByteOrder>(&mut self, dst: &mut [u64]) -> Result<()> {
        read_ordered(self, dst, T::read_u64_into)
    }

    /// Reads a sequence of unsigned 128 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of unsigned 128 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 5,
    ///     0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0,
    /// ]);
    /// let mut dst = [0; 2];
    /// rdr.read_u128_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_u128_into<T: ByteOrder>(
        &mut self,
        dst: &mut [u128],
    ) -> Result<()> {
        read_ordered(self, dst, T::read_u128_into)
    }

    /// Reads a sequence of signed 8 bit integers from the underlying reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// Note that since each `i8` is a single byte, no byte order conversions
    /// are used. This method is included because it provides a safe, simple
    /// way for the caller to read into a `&mut [i8]` buffer. (Without this
    /// method, the caller would have to either use `unsafe` code or convert
    /// each byte to `i8` individually.)
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of signed 8 bit integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![2, 251, 3]);
    /// let mut dst = [0; 3];
    /// rdr.read_i8_into(&mut dst).unwrap();
    /// assert_eq!([2, -5, 3], dst);
    /// ```
    #[inline]
    fn read_i8_into(&mut self, dst: &mut [i8]) -> Result<()> {
        read_ordered(self, dst, |input, output| {
            for (&byte, value) in input.iter().zip(output) {
                *value = byte as i8;
            }
        })
    }

    /// Reads a sequence of signed 16 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of signed 16 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![2, 5, 3, 0]);
    /// let mut dst = [0; 2];
    /// rdr.read_i16_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_i16_into<T: ByteOrder>(&mut self, dst: &mut [i16]) -> Result<()> {
        read_ordered(self, dst, T::read_i16_into)
    }

    /// Reads a sequence of signed 32 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of signed 32 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![0, 0, 2, 5, 0, 0, 3, 0]);
    /// let mut dst = [0; 2];
    /// rdr.read_i32_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_i32_into<T: ByteOrder>(&mut self, dst: &mut [i32]) -> Result<()> {
        read_ordered(self, dst, T::read_i32_into)
    }

    /// Reads a sequence of signed 64 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of signed 64 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0, 0, 0, 0, 0, 0, 2, 5,
    ///     0, 0, 0, 0, 0, 0, 3, 0,
    /// ]);
    /// let mut dst = [0; 2];
    /// rdr.read_i64_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_i64_into<T: ByteOrder>(&mut self, dst: &mut [i64]) -> Result<()> {
        read_ordered(self, dst, T::read_i64_into)
    }

    /// Reads a sequence of signed 128 bit integers from the underlying
    /// reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of signed 128 bit big-endian integers from a `Read`:
    ///
    /// ```rust
    /// use std::io::Cursor;
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 5,
    ///     0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0,
    /// ]);
    /// let mut dst = [0; 2];
    /// rdr.read_i128_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([517, 768], dst);
    /// ```
    #[inline]
    fn read_i128_into<T: ByteOrder>(
        &mut self,
        dst: &mut [i128],
    ) -> Result<()> {
        read_ordered(self, dst, T::read_i128_into)
    }

    /// Reads a sequence of IEEE754 single-precision (4 bytes) floating
    /// point numbers from the underlying reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of big-endian single-precision floating point number
    /// from a `Read`:
    ///
    /// ```rust
    /// use std::f32;
    /// use std::io::Cursor;
    ///
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x40, 0x49, 0x0f, 0xdb,
    ///     0x3f, 0x80, 0x00, 0x00,
    /// ]);
    /// let mut dst = [0.0; 2];
    /// rdr.read_f32_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([f32::consts::PI, 1.0], dst);
    /// ```
    #[inline]
    fn read_f32_into<T: ByteOrder>(&mut self, dst: &mut [f32]) -> Result<()> {
        read_ordered(self, dst, T::read_f32_into)
    }

    /// **DEPRECATED**.
    ///
    /// This method is deprecated. Use `read_f32_into` instead.
    ///
    /// Reads a sequence of IEEE754 single-precision (4 bytes) floating
    /// point numbers from the underlying reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of big-endian single-precision floating point number
    /// from a `Read`:
    ///
    /// ```rust
    /// use std::f32;
    /// use std::io::Cursor;
    ///
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x40, 0x49, 0x0f, 0xdb,
    ///     0x3f, 0x80, 0x00, 0x00,
    /// ]);
    /// let mut dst = [0.0; 2];
    /// rdr.read_f32_into_unchecked::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([f32::consts::PI, 1.0], dst);
    /// ```
    #[inline]
    #[deprecated(since = "1.2.0", note = "please use `read_f32_into` instead")]
    fn read_f32_into_unchecked<T: ByteOrder>(
        &mut self,
        dst: &mut [f32],
    ) -> Result<()> {
        self.read_f32_into::<T>(dst)
    }

    /// Reads a sequence of IEEE754 double-precision (8 bytes) floating
    /// point numbers from the underlying reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of big-endian single-precision floating point number
    /// from a `Read`:
    ///
    /// ```rust
    /// use std::f64;
    /// use std::io::Cursor;
    ///
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x40, 0x09, 0x21, 0xfb, 0x54, 0x44, 0x2d, 0x18,
    ///     0x3f, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    /// ]);
    /// let mut dst = [0.0; 2];
    /// rdr.read_f64_into::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([f64::consts::PI, 1.0], dst);
    /// ```
    #[inline]
    fn read_f64_into<T: ByteOrder>(&mut self, dst: &mut [f64]) -> Result<()> {
        read_ordered(self, dst, T::read_f64_into)
    }

    /// **DEPRECATED**.
    ///
    /// This method is deprecated. Use `read_f64_into` instead.
    ///
    /// Reads a sequence of IEEE754 double-precision (8 bytes) floating
    /// point numbers from the underlying reader.
    ///
    /// The given buffer is either filled completely or an error is returned.
    /// If an error is returned, the contents of `dst` are unspecified.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Read::read_exact`].
    ///
    /// [`Read::read_exact`]: https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact
    ///
    /// # Examples
    ///
    /// Read a sequence of big-endian single-precision floating point number
    /// from a `Read`:
    ///
    /// ```rust
    /// use std::f64;
    /// use std::io::Cursor;
    ///
    /// use byteorder::{BigEndian, ReadBytesExt};
    ///
    /// let mut rdr = Cursor::new(vec![
    ///     0x40, 0x09, 0x21, 0xfb, 0x54, 0x44, 0x2d, 0x18,
    ///     0x3f, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    /// ]);
    /// let mut dst = [0.0; 2];
    /// rdr.read_f64_into_unchecked::<BigEndian>(&mut dst).unwrap();
    /// assert_eq!([f64::consts::PI, 1.0], dst);
    /// ```
    #[inline]
    #[deprecated(since = "1.2.0", note = "please use `read_f64_into` instead")]
    fn read_f64_into_unchecked<T: ByteOrder>(
        &mut self,
        dst: &mut [f64],
    ) -> Result<()> {
        self.read_f64_into::<T>(dst)
    }
}

/// All types that implement `Read` get methods defined in `ReadBytesExt`
/// for free.
impl<R: io::Read + ?Sized> ReadBytesExt for R {}

/// Extends [`Write`] with methods for writing numbers. (For `std::io`.)
///
/// Most of the methods defined here have an unconstrained type parameter that
/// must be explicitly instantiated. Typically, it is instantiated with either
/// the [`BigEndian`] or [`LittleEndian`] types defined in this crate.
///
/// # Examples
///
/// Write unsigned 16 bit big-endian integers to a [`Write`]:
///
/// ```rust
/// use byteorder::{BigEndian, WriteBytesExt};
///
/// let mut wtr = vec![];
/// wtr.write_u16::<BigEndian>(517).unwrap();
/// wtr.write_u16::<BigEndian>(768).unwrap();
/// assert_eq!(wtr, vec![2, 5, 3, 0]);
/// ```
///
/// [`BigEndian`]: enum.BigEndian.html
/// [`LittleEndian`]: enum.LittleEndian.html
/// [`Write`]: https://doc.rust-lang.org/std/io/trait.Write.html
pub trait WriteBytesExt: io::Write {
    /// Writes an unsigned 8 bit integer to the underlying writer.
    ///
    /// Note that since this writes a single byte, no byte order conversions
    /// are used. It is included for completeness.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write unsigned 8 bit integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::WriteBytesExt;
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_u8(2).unwrap();
    /// wtr.write_u8(5).unwrap();
    /// assert_eq!(wtr, b"\x02\x05");
    /// ```
    #[inline]
    fn write_u8(&mut self, n: u8) -> Result<()> {
        self.write_all(&[n])
    }

    /// Writes a signed 8 bit integer to the underlying writer.
    ///
    /// Note that since this writes a single byte, no byte order conversions
    /// are used. It is included for completeness.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write signed 8 bit integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::WriteBytesExt;
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_i8(2).unwrap();
    /// wtr.write_i8(-5).unwrap();
    /// assert_eq!(wtr, b"\x02\xfb");
    /// ```
    #[inline]
    fn write_i8(&mut self, n: i8) -> Result<()> {
        self.write_all(&[n as u8])
    }

    /// Writes an unsigned 16 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write unsigned 16 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_u16::<BigEndian>(517).unwrap();
    /// wtr.write_u16::<BigEndian>(768).unwrap();
    /// assert_eq!(wtr, b"\x02\x05\x03\x00");
    /// ```
    #[inline]
    fn write_u16<T: ByteOrder>(&mut self, n: u16) -> Result<()> {
        let mut buf = [0; 2];
        T::write_u16(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a signed 16 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write signed 16 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_i16::<BigEndian>(193).unwrap();
    /// wtr.write_i16::<BigEndian>(-132).unwrap();
    /// assert_eq!(wtr, b"\x00\xc1\xff\x7c");
    /// ```
    #[inline]
    fn write_i16<T: ByteOrder>(&mut self, n: i16) -> Result<()> {
        let mut buf = [0; 2];
        T::write_i16(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes an unsigned 24 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write unsigned 24 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_u24::<BigEndian>(267).unwrap();
    /// wtr.write_u24::<BigEndian>(120111).unwrap();
    /// assert_eq!(wtr, b"\x00\x01\x0b\x01\xd5\x2f");
    /// ```
    #[inline]
    fn write_u24<T: ByteOrder>(&mut self, n: u32) -> Result<()> {
        let mut buf = [0; 3];
        T::write_u24(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a signed 24 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write signed 24 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_i24::<BigEndian>(-34253).unwrap();
    /// wtr.write_i24::<BigEndian>(120111).unwrap();
    /// assert_eq!(wtr, b"\xff\x7a\x33\x01\xd5\x2f");
    /// ```
    #[inline]
    fn write_i24<T: ByteOrder>(&mut self, n: i32) -> Result<()> {
        let mut buf = [0; 3];
        T::write_i24(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes an unsigned 32 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write unsigned 32 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_u32::<BigEndian>(267).unwrap();
    /// wtr.write_u32::<BigEndian>(1205419366).unwrap();
    /// assert_eq!(wtr, b"\x00\x00\x01\x0b\x47\xd9\x3d\x66");
    /// ```
    #[inline]
    fn write_u32<T: ByteOrder>(&mut self, n: u32) -> Result<()> {
        let mut buf = [0; 4];
        T::write_u32(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a signed 32 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write signed 32 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_i32::<BigEndian>(-34253).unwrap();
    /// wtr.write_i32::<BigEndian>(1205419366).unwrap();
    /// assert_eq!(wtr, b"\xff\xff\x7a\x33\x47\xd9\x3d\x66");
    /// ```
    #[inline]
    fn write_i32<T: ByteOrder>(&mut self, n: i32) -> Result<()> {
        let mut buf = [0; 4];
        T::write_i32(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes an unsigned 48 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write unsigned 48 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_u48::<BigEndian>(52360336390828).unwrap();
    /// wtr.write_u48::<BigEndian>(541).unwrap();
    /// assert_eq!(wtr, b"\x2f\x9f\x17\x40\x3a\xac\x00\x00\x00\x00\x02\x1d");
    /// ```
    #[inline]
    fn write_u48<T: ByteOrder>(&mut self, n: u64) -> Result<()> {
        let mut buf = [0; 6];
        T::write_u48(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a signed 48 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write signed 48 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_i48::<BigEndian>(-108363435763825).unwrap();
    /// wtr.write_i48::<BigEndian>(77).unwrap();
    /// assert_eq!(wtr, b"\x9d\x71\xab\xe7\x97\x8f\x00\x00\x00\x00\x00\x4d");
    /// ```
    #[inline]
    fn write_i48<T: ByteOrder>(&mut self, n: i64) -> Result<()> {
        let mut buf = [0; 6];
        T::write_i48(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes an unsigned 64 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write unsigned 64 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_u64::<BigEndian>(918733457491587).unwrap();
    /// wtr.write_u64::<BigEndian>(143).unwrap();
    /// assert_eq!(wtr, b"\x00\x03\x43\x95\x4d\x60\x86\x83\x00\x00\x00\x00\x00\x00\x00\x8f");
    /// ```
    #[inline]
    fn write_u64<T: ByteOrder>(&mut self, n: u64) -> Result<()> {
        let mut buf = [0; 8];
        T::write_u64(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a signed 64 bit integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write signed 64 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_i64::<BigEndian>(i64::min_value()).unwrap();
    /// wtr.write_i64::<BigEndian>(i64::max_value()).unwrap();
    /// assert_eq!(wtr, b"\x80\x00\x00\x00\x00\x00\x00\x00\x7f\xff\xff\xff\xff\xff\xff\xff");
    /// ```
    #[inline]
    fn write_i64<T: ByteOrder>(&mut self, n: i64) -> Result<()> {
        let mut buf = [0; 8];
        T::write_i64(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes an unsigned 128 bit integer to the underlying writer.
    #[inline]
    fn write_u128<T: ByteOrder>(&mut self, n: u128) -> Result<()> {
        let mut buf = [0; 16];
        T::write_u128(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a signed 128 bit integer to the underlying writer.
    #[inline]
    fn write_i128<T: ByteOrder>(&mut self, n: i128) -> Result<()> {
        let mut buf = [0; 16];
        T::write_i128(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes an unsigned n-bytes integer to the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Panics
    ///
    /// If the given integer is not representable in the given number of bytes,
    /// this method panics. If `nbytes > 8`, this method panics.
    ///
    /// # Examples
    ///
    /// Write unsigned 40 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_uint::<BigEndian>(312550384361, 5).unwrap();
    /// wtr.write_uint::<BigEndian>(43, 5).unwrap();
    /// assert_eq!(wtr, b"\x48\xc5\x74\x62\xe9\x00\x00\x00\x00\x2b");
    /// ```
    #[inline]
    fn write_uint<T: ByteOrder>(
        &mut self,
        n: u64,
        nbytes: usize,
    ) -> Result<()> {
        let mut buf = [0; 8];
        T::write_uint(&mut buf, n, nbytes);
        self.write_all(&buf[0..nbytes])
    }

    /// Writes the low `nbytes` bytes of a signed integer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Panics
    ///
    /// Panics if `nbytes` is outside `1..=8`.
    ///
    /// # Examples
    ///
    /// Write signed 56 bit big-endian integers to a `Write`:
    ///
    /// ```rust
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_int::<BigEndian>(-3548172039376767, 7).unwrap();
    /// wtr.write_int::<BigEndian>(43, 7).unwrap();
    /// assert_eq!(wtr, b"\xf3\x64\xf4\xd1\xfd\xb0\x81\x00\x00\x00\x00\x00\x00\x2b");
    /// ```
    #[inline]
    fn write_int<T: ByteOrder>(
        &mut self,
        n: i64,
        nbytes: usize,
    ) -> Result<()> {
        let mut buf = [0; 8];
        T::write_int(&mut buf, n, nbytes);
        self.write_all(&buf[0..nbytes])
    }

    /// Writes an unsigned n-bytes integer to the underlying writer.
    ///
    /// If the given integer is not representable in the given number of bytes,
    /// this method panics. If `nbytes > 16`, this method panics.
    #[inline]
    fn write_uint128<T: ByteOrder>(
        &mut self,
        n: u128,
        nbytes: usize,
    ) -> Result<()> {
        let mut buf = [0; 16];
        T::write_uint128(&mut buf, n, nbytes);
        self.write_all(&buf[0..nbytes])
    }

    /// Writes the low `nbytes` bytes of a signed integer.
    ///
    /// Panics if `nbytes` is outside `1..=16`.
    #[inline]
    fn write_int128<T: ByteOrder>(
        &mut self,
        n: i128,
        nbytes: usize,
    ) -> Result<()> {
        let mut buf = [0; 16];
        T::write_int128(&mut buf, n, nbytes);
        self.write_all(&buf[0..nbytes])
    }

    /// Writes a IEEE754 single-precision (4 bytes) floating point number to
    /// the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write a big-endian single-precision floating point number to a `Write`:
    ///
    /// ```rust
    /// use std::f32;
    ///
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_f32::<BigEndian>(f32::consts::PI).unwrap();
    /// assert_eq!(wtr, b"\x40\x49\x0f\xdb");
    /// ```
    #[inline]
    fn write_f32<T: ByteOrder>(&mut self, n: f32) -> Result<()> {
        let mut buf = [0; 4];
        T::write_f32(&mut buf, n);
        self.write_all(&buf)
    }

    /// Writes a IEEE754 double-precision (8 bytes) floating point number to
    /// the underlying writer.
    ///
    /// # Errors
    ///
    /// This method returns the same errors as [`Write::write_all`].
    ///
    /// [`Write::write_all`]: https://doc.rust-lang.org/std/io/trait.Write.html#method.write_all
    ///
    /// # Examples
    ///
    /// Write a big-endian double-precision floating point number to a `Write`:
    ///
    /// ```rust
    /// use std::f64;
    ///
    /// use byteorder::{BigEndian, WriteBytesExt};
    ///
    /// let mut wtr = Vec::new();
    /// wtr.write_f64::<BigEndian>(f64::consts::PI).unwrap();
    /// assert_eq!(wtr, b"\x40\x09\x21\xfb\x54\x44\x2d\x18");
    /// ```
    #[inline]
    fn write_f64<T: ByteOrder>(&mut self, n: f64) -> Result<()> {
        let mut buf = [0; 8];
        T::write_f64(&mut buf, n);
        self.write_all(&buf)
    }
}

/// All types that implement `Write` get methods defined in `WriteBytesExt`
/// for free.
impl<W: io::Write + ?Sized> WriteBytesExt for W {}

// Bound staging independently of the caller's destination length. Whole
// numbers are decoded only after their chunk has been read successfully.
fn read_ordered<R: io::Read + ?Sized, T>(
    reader: &mut R,
    dst: &mut [T],
    mut decode: impl FnMut(&[u8], &mut [T]),
) -> Result<()> {
    if dst.is_empty() {
        return reader.read_exact(&mut []);
    }

    let width = size_of::<T>();
    let mut scratch = [0; 4096];
    for values in dst.chunks_mut(scratch.len() / width) {
        let bytes = &mut scratch[..values.len() * width];
        reader.read_exact(bytes)?;
        decode(bytes, values);
    }
    Ok(())
}

#[cfg(test)]
mod checked_io_tests {
    use super::*;
    use crate::{BigEndian, LittleEndian};
    use std::io::{Cursor, Read};

    struct Fragmented<'a> {
        bytes: Cursor<&'a [u8]>,
        interrupt: bool,
    }

    impl Read for Fragmented<'_> {
        fn read(&mut self, output: &mut [u8]) -> Result<usize> {
            if self.interrupt {
                self.interrupt = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            let fragment = output.len().min(7);
            self.bytes.read(&mut output[..fragment])
        }
    }

    #[test]
    fn bulk_read_retries_and_leaves_the_next_byte() {
        let expected: Vec<i128> = (0..900).map(|i| i - 500).collect();
        let mut input = vec![0; expected.len() * 16];
        BigEndian::write_i128_into(&expected, &mut input);
        input.push(0xdd);
        let mut reader = Fragmented { bytes: Cursor::new(&input), interrupt: true };
        let mut dst = vec![0; expected.len()];
        reader.read_i128_into::<BigEndian>(&mut dst).unwrap();
        assert_eq!(dst, expected);
        assert_eq!(reader.read_u8().unwrap(), 0xdd);
    }

    #[test]
    fn bulk_read_reports_eof_after_several_chunks_and_keeps_float_bits() {
        let mut input = Vec::new();
        let samples = [0x7ff0_0000_0000_0001u64, 0x8000_0000_0000_0000, u64::MAX];
        for i in 0..1024 {
            input.extend_from_slice(&samples[i % samples.len()].to_le_bytes());
        }
        let mut dst = vec![0.0; 1024];
        input.as_slice().read_f64_into::<LittleEndian>(&mut dst).unwrap();
        for (i, value) in dst.iter().enumerate() {
            assert_eq!(value.to_bits(), samples[i % samples.len()]);
        }
        let sentinel = 1.0f64;
        dst.fill(sentinel);
        let n = input.len() - 1;
        let error = input[..n].as_ref().read_f64_into::<LittleEndian>(&mut dst).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        // The first 4 KiB is complete. The failed second chunk never
        // reaches the typed destination.
        for (i, value) in dst[..512].iter().enumerate() {
            assert_eq!(value.to_bits(), samples[i % samples.len()]);
        }
        assert!(dst[512..].iter().all(|v| v.to_bits() == sentinel.to_bits()));
    }
}
