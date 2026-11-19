use crate::error::Error;
use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use core::fmt::{self, Debug, Display};
use serde::de::value::BorrowedStrDeserializer;
use serde::de::{
    self, Deserialize, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, Unexpected,
    Visitor,
};
use serde::forward_to_deserialize_any;
use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Owned text for a JSON value whose original formatting is retained.
///
/// A `RawValue` can be used to defer parsing parts of a payload until later,
/// or to avoid parsing it at all in the case that part of the payload just
/// needs to be transferred verbatim into a different output object.
///
/// When serializing, a value of this type will retain its original formatting
/// and will not be minified or pretty-printed.
///
/// # Note
///
/// `RawValue` is only available if serde\_json is built with the `"raw_value"`
/// feature.
///
/// ```toml
/// [dependencies]
/// serde_json = { version = "1.0", features = ["raw_value"] }
/// ```
///
/// Use [`BorrowedRawValue`] to borrow a value from an input string or slice.
/// Use `Box<RawValue>` when reading from an I/O stream or retaining an owned
/// value independently of its input.
#[cfg_attr(docsrs, doc(cfg(feature = "raw_value")))]
pub struct RawValue {
    json: RawText,
}

enum RawText {
    Owned(Box<str>),
    Static(&'static str),
}

impl RawValue {
    const fn from_static(json: &'static str) -> Self {
        RawValue {
            json: RawText::Static(json),
        }
    }

    fn from_owned(json: Box<str>) -> Box<Self> {
        Box::new(RawValue {
            json: RawText::Owned(json),
        })
    }
}

impl Clone for Box<RawValue> {
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

impl ToOwned for RawValue {
    type Owned = Box<RawValue>;

    fn to_owned(&self) -> Self::Owned {
        RawValue::from_owned(self.get().into())
    }
}

impl Default for Box<RawValue> {
    fn default() -> Self {
        RawValue::NULL.to_owned()
    }
}

fn debug_raw(json: &str, formatter: &mut fmt::Formatter) -> fmt::Result {
    formatter
        .debug_tuple("RawValue")
        .field(&format_args!("{}", json))
        .finish()
}

impl Debug for RawValue {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        debug_raw(self.get(), formatter)
    }
}

impl Display for RawValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.get())
    }
}

impl RawValue {
    /// A constant RawValue with the JSON value `null`.
    pub const NULL: &'static RawValue = &RawValue::from_static("null");
    /// A constant RawValue with the JSON value `true`.
    pub const TRUE: &'static RawValue = &RawValue::from_static("true");
    /// A constant RawValue with the JSON value `false`.
    pub const FALSE: &'static RawValue = &RawValue::from_static("false");

    /// Validate a single JSON value and retain its original text.
    ///
    /// The result excludes surrounding whitespace. If the input contains
    /// only the value and has capacity equal to its length, the result
    /// takes ownership of the existing string allocation.
    pub fn from_string(json: String) -> Result<Box<Self>, Error> {
        let borrowed = tri!(crate::from_str::<BorrowedRawValue<'_>>(&json));
        if borrowed.get().len() < json.len() {
            return Ok(borrowed.into_owned());
        }
        Ok(Self::from_owned(json.into_boxed_str()))
    }

    /// Access the original JSON text.
    pub fn get(&self) -> &str {
        match &self.json {
            RawText::Owned(json) => json,
            RawText::Static(json) => json,
        }
    }
}

impl From<Box<RawValue>> for Box<str> {
    fn from(raw_value: Box<RawValue>) -> Self {
        match raw_value.json {
            RawText::Owned(json) => json,
            RawText::Static(json) => json.into(),
        }
    }
}

/// A JSON value borrowed from an input string or byte slice.
///
/// The borrowed value retains a slice of the input text without copying it.
/// Serializing it writes the value with the same formatting. Equality and
/// hashing use that text, including whitespace within the value.
///
/// ```
/// use serde::{Deserialize, Serialize};
/// use serde_json::value::BorrowedRawValue;
///
/// #[derive(Deserialize, Serialize)]
/// struct Message<'a> {
///     code: u32,
///     #[serde(borrow)]
///     payload: BorrowedRawValue<'a>,
/// }
///
/// let message: Message = serde_json::from_str(r#"{"code":200,"payload":[1, 2]}"#)?;
/// assert_eq!(message.payload.get(), "[1, 2]");
/// assert_eq!(serde_json::to_string(&message)?, r#"{"code":200,"payload":[1, 2]}"#);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "raw_value")))]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct BorrowedRawValue<'a> {
    json: &'a str,
}

impl<'a> BorrowedRawValue<'a> {
    /// Access the original JSON text.
    pub fn get(&self) -> &'a str {
        self.json
    }

    /// Copy the value into owned storage.
    pub fn into_owned(self) -> Box<RawValue> {
        RawValue::from_owned(self.json.into())
    }
}

impl Debug for BorrowedRawValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        debug_raw(self.get(), formatter)
    }
}

impl Display for BorrowedRawValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.json)
    }
}

/// Convert a `T` into a boxed `RawValue`.
///
/// # Example
///
/// ```
/// // Upstream crate
/// # #[derive(Serialize)]
/// pub struct Thing {
///     foo: String,
///     bar: Option<String>,
///     extra_data: Box<RawValue>,
/// }
///
/// // Local crate
/// use serde::Serialize;
/// use serde_json::value::{to_raw_value, RawValue};
///
/// #[derive(Serialize)]
/// struct MyExtraData {
///     a: u32,
///     b: u32,
/// }
///
/// let my_thing = Thing {
///     foo: "FooVal".into(),
///     bar: None,
///     extra_data: to_raw_value(&MyExtraData { a: 1, b: 2 }).unwrap(),
/// };
/// # assert_eq!(
/// #     serde_json::to_value(my_thing).unwrap(),
/// #     serde_json::json!({
/// #         "foo": "FooVal",
/// #         "bar": null,
/// #         "extra_data": { "a": 1, "b": 2 }
/// #     })
/// # );
/// ```
///
/// # Errors
///
/// This conversion can fail if `T`'s implementation of `Serialize` decides to
/// fail, or if `T` contains a map with non-string keys.
///
/// ```
/// use std::collections::BTreeMap;
///
/// // The keys in this map are vectors, not strings.
/// let mut map = BTreeMap::new();
/// map.insert(vec![32, 64], "x86");
///
/// println!("{}", serde_json::value::to_raw_value(&map).unwrap_err());
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "raw_value")))]
pub fn to_raw_value<T>(value: &T) -> Result<Box<RawValue>, Error>
where
    T: ?Sized + Serialize,
{
    let json_string = tri!(crate::to_string(value));
    Ok(RawValue::from_owned(json_string.into_boxed_str()))
}

pub const TOKEN: &str = "$serde_json::private::RawValue";

fn serialize_raw<S>(json: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut output = tri!(serializer.serialize_struct(TOKEN, 1));
    tri!(output.serialize_field(TOKEN, json));
    output.end()
}

impl Serialize for RawValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_raw(self.get(), serializer)
    }
}

impl Serialize for BorrowedRawValue<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_raw(self.get(), serializer)
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for BorrowedRawValue<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ReferenceVisitor;

        impl<'de> Visitor<'de> for ReferenceVisitor {
            type Value = BorrowedRawValue<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "any valid JSON value")
            }

            fn visit_map<V>(self, mut visitor: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let value = tri!(visitor.next_key::<RawKey>());
                if value.is_none() {
                    return Err(de::Error::invalid_type(Unexpected::Map, &self));
                }
                visitor.next_value_seed(ReferenceFromString)
            }
        }

        deserializer.deserialize_newtype_struct(TOKEN, ReferenceVisitor)
    }
}

impl<'de> Deserialize<'de> for Box<RawValue> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BoxedVisitor;

        impl<'de> Visitor<'de> for BoxedVisitor {
            type Value = Box<RawValue>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "any valid JSON value")
            }

            fn visit_map<V>(self, mut visitor: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let value = tri!(visitor.next_key::<RawKey>());
                if value.is_none() {
                    return Err(de::Error::invalid_type(Unexpected::Map, &self));
                }
                visitor.next_value_seed(BoxedFromString)
            }
        }

        deserializer.deserialize_newtype_struct(TOKEN, BoxedVisitor)
    }
}

struct RawKey;

impl<'de> Deserialize<'de> for RawKey {
    fn deserialize<D>(deserializer: D) -> Result<RawKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = ();

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("raw value")
            }

            fn visit_str<E>(self, s: &str) -> Result<(), E>
            where
                E: de::Error,
            {
                if s == TOKEN {
                    Ok(())
                } else {
                    Err(de::Error::custom("unexpected raw value"))
                }
            }
        }

        tri!(deserializer.deserialize_identifier(FieldVisitor));
        Ok(RawKey)
    }
}

pub struct ReferenceFromString;

impl<'de> DeserializeSeed<'de> for ReferenceFromString {
    type Value = BorrowedRawValue<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for ReferenceFromString {
    type Value = BorrowedRawValue<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("raw value")
    }

    fn visit_borrowed_str<E>(self, s: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(BorrowedRawValue { json: s })
    }
}

pub struct BoxedFromString;

impl<'de> DeserializeSeed<'de> for BoxedFromString {
    type Value = Box<RawValue>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for BoxedFromString {
    type Value = Box<RawValue>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("raw value")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::from_owned(s.to_owned().into_boxed_str()))
    }

    #[cfg(any(feature = "std", feature = "alloc"))]
    fn visit_string<E>(self, s: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RawValue::from_owned(s.into_boxed_str()))
    }
}

struct RawKeyDeserializer;

impl<'de> Deserializer<'de> for RawKeyDeserializer {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_borrowed_str(TOKEN)
    }

    forward_to_deserialize_any! {
        bool u8 u16 u32 u64 u128 i8 i16 i32 i64 i128 f32 f64 char str string seq
        bytes byte_buf map struct option unit newtype_struct ignored_any
        unit_struct tuple_struct tuple enum identifier
    }
}

pub struct OwnedRawDeserializer {
    pub raw_value: Option<String>,
}

impl<'de> MapAccess<'de> for OwnedRawDeserializer {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        if self.raw_value.is_none() {
            return Ok(None);
        }
        seed.deserialize(RawKeyDeserializer).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        seed.deserialize(self.raw_value.take().unwrap().into_deserializer())
    }
}

pub struct BorrowedRawDeserializer<'de> {
    pub raw_value: Option<&'de str>,
}

impl<'de> MapAccess<'de> for BorrowedRawDeserializer<'de> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        if self.raw_value.is_none() {
            return Ok(None);
        }
        seed.deserialize(RawKeyDeserializer).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        seed.deserialize(BorrowedStrDeserializer::new(self.raw_value.take().unwrap()))
    }
}

// Both representations pass the same original text to the JSON deserializer.
macro_rules! impl_raw_deserializer {
    ($raw:ty) => {
        impl<'de> IntoDeserializer<'de, Error> for $raw {
            type Deserializer = Self;

            fn into_deserializer(self) -> Self::Deserializer {
                self
            }
        }

        impl<'de> Deserializer<'de> for $raw {
            type Error = Error;

            raw_deserializer_methods! {
                deserialize_any, deserialize_bool,
                deserialize_i8, deserialize_i16, deserialize_i32, deserialize_i64, deserialize_i128,
                deserialize_u8, deserialize_u16, deserialize_u32, deserialize_u64, deserialize_u128,
                deserialize_f32, deserialize_f64, deserialize_char, deserialize_str,
                deserialize_string, deserialize_bytes, deserialize_byte_buf, deserialize_option,
                deserialize_unit, deserialize_unit_struct(name: &'static str),
                deserialize_newtype_struct(name: &'static str), deserialize_seq,
                deserialize_tuple(len: usize), deserialize_tuple_struct(name: &'static str, len: usize),
                deserialize_map, deserialize_struct(name: &'static str, fields: &'static [&'static str]),
                deserialize_enum(name: &'static str, variants: &'static [&'static str]),
                deserialize_identifier, deserialize_ignored_any,
            }
        }
    };
}

macro_rules! raw_deserializer_methods {
    ($($method:ident $(($($arg:ident: $ty:ty),+))?),+ $(,)?) => {
        $(
            fn $method<V>(
                self, $($($arg: $ty,)+)? visitor: V,
            ) -> Result<V::Value, Error>
            where
                V: Visitor<'de>,
            {
                crate::Deserializer::from_str(self.get()).$method($($($arg,)+)? visitor)
            }
        )+
    };
}

impl_raw_deserializer!(&'de RawValue);
impl_raw_deserializer!(BorrowedRawValue<'de>);
