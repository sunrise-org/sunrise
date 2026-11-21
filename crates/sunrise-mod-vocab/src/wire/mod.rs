//! Canonical binary encoding for owned API values, including typed payload tokens.
//! Typed payload bodies are encoded by `PayloadCodec`, not Wire.
//!
//! Everything is little-endian and self-delimiting: a length is a `u32` before its items, an
//! `Option` is a tag byte, an enum is a `u32` case then its fields, a record is its fields in
//! order. A reader trusts nothing: it checks every length against the bytes left and against the
//! smallest encoding of the items it promises, every tag against the cases the type has, strings
//! as UTF-8, and nesting against [`MAX_DEPTH`]. A bad input is a [`WireError`], never a panic and
//! never an allocation larger than the input can back.
//!
//! Derive [`Wire`](crate::Wire) on types that cross a target boundary. The derive encodes
//! fields in declaration order; checked constructors can use `#[wire(via = T)]`.
//! Hand-written implementations declare how they cross with `layout_fields!` or `layout_via!`.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::Arc;

mod static_strings;

/// Maximum static strings retained by the process.
// hidden: transport plumbing used by generated boundary code.
#[doc(hidden)]
pub const MAX_STATIC_STRINGS: usize = 4096;
/// Maximum bytes in one static API string (the existing interning bound).
// hidden: transport plumbing used by generated boundary code.
#[doc(hidden)]
pub const MAX_STATIC_STRING_BYTES: usize = 256;

/// Intern a static API string using the platform's bounded process-lifetime storage.
// hidden: transport plumbing used by generated boundary code.
#[doc(hidden)]
pub fn intern(value: &str) -> Result<&'static str, WireError> {
    static_strings::intern(value)
}

/// The deepest nesting of one value (a component of a component of…) a reader accepts.
///
/// Structural codecs refuse before entering level 33. `Tag` and `Text` use
/// stack-independent tree codecs to preserve their vanilla depth of 512.
/// Container iteration does not itself consume reader depth.
pub const MAX_DEPTH: u16 = 32;

/// Why a buffer is not a value of the type it was read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: decoder and encoder diagnostics are returned locally, not encoded as values.
pub enum WireError {
    /// The buffer ends inside a value.
    Eof,
    /// A string is not UTF-8.
    Utf8,
    /// An enum case or option tag the type does not have.
    Tag(u32),
    /// A length longer than the rest of the buffer could hold.
    Length,
    /// Values nested deeper than [`MAX_DEPTH`].
    Depth,
    /// A value its type refuses (an unknown key, a count out of range), and why.
    Invalid(&'static str),
    /// Bytes left over after the value.
    Trailing,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eof => f.write_str("the data ends inside a value"),
            Self::Utf8 => f.write_str("a string is not UTF-8"),
            Self::Tag(tag) => write!(f, "case {tag} does not exist"),
            Self::Length => f.write_str("a length is longer than the data"),
            Self::Depth => f.write_str("values are nested too deeply"),
            Self::Invalid(why) => f.write_str(why),
            Self::Trailing => f.write_str("data left over after the value"),
        }
    }
}

impl std::error::Error for WireError {}

// exhaustive: fields are private; use the codec methods.
/// A buffer values are written to. Reuse one to keep the allocation.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
    error: Option<WireError>,
    limit: Option<usize>,
    depth: u16,
}

impl Writer {
    /// Creates an empty writer with no byte limit.
    pub fn new() -> Self {
        Self::default()
    }
    /// Empties it, keeping its allocation and resetting any per-call error and byte limit.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.error = None;
        self.limit = None;
    }
    /// Takes `buffer` as its storage, emptied, resetting the error and byte limit.
    pub fn replace(&mut self, mut buffer: Vec<u8>) {
        buffer.clear();
        self.bytes = buffer;
        self.error = None;
        self.limit = None;
    }
    /// The bytes written, leaving the writer's storage to the caller.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    /// Borrows the bytes written so far; check `error` before sending them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Bound this call's encoded bytes. A refusal is recorded before any growth or copy.
    /// A new writer has no limit; `clear` and `replace` remove it.
    pub fn limit(&mut self, max_bytes: usize) {
        self.limit = Some(max_bytes);
        self.can_fit(0);
    }
    fn can_fit(&mut self, bytes: usize) -> bool {
        if self.error.is_some() {
            return false;
        }
        let fits = self.bytes.len().checked_add(bytes).is_some_and(|total| {
            total <= self.limit.unwrap_or(isize::MAX as usize) && total <= isize::MAX as usize
        });
        if !fits {
            self.fail(WireError::Length);
        }
        fits
    }
    /// Appends raw bytes without a length prefix.
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        if self.can_fit(bytes.len()) {
            self.bytes.extend_from_slice(bytes);
        }
    }
    /// A string: its length, then its bytes.
    pub fn put_str(&mut self, text: &str) {
        let Some(bytes) = text.len().checked_add(4) else {
            self.fail(WireError::Length);
            return;
        };
        if !self.can_fit(bytes) {
            return;
        }
        self.put_len(text.len());
        self.put_bytes(text.as_bytes());
    }
    /// A list of `u16`: its count, then each item, in one pass (the bytes of `Vec<u16>`'s own form).
    pub(crate) fn put_u16s(&mut self, items: &[u16]) {
        u16::put_list(items, self);
    }
    /// A length or count.
    pub fn put_len(&mut self, len: usize) {
        match u32::try_from(len) {
            Ok(len) => self.put(&len),
            Err(_) => self.fail(WireError::Length),
        }
    }
    /// An optional borrowed value, written as the `Option` it stands for.
    pub fn put_opt<T: Put + ?Sized>(&mut self, value: Option<&T>) {
        match value {
            Some(value) => {
                self.put(&1u8);
                self.put(value);
            }
            None => self.put(&0u8),
        }
    }
    /// Encodes a borrowed value unless an earlier write failed.
    /// The caller keeps ownership and is responsible for disposing of the value.
    pub fn put<T: Put + ?Sized>(&mut self, value: &T) {
        if self.error.is_none() {
            value.put_to(self);
        }
    }
    /// Writes one structural level, refusing before recursing past [`MAX_DEPTH`].
    pub fn nested(&mut self, write: impl FnOnce(&mut Self)) {
        if self.error.is_some() {
            return;
        }
        if self.depth >= MAX_DEPTH {
            self.fail(WireError::Depth);
            return;
        }
        self.depth += 1;
        write(self);
        self.depth -= 1;
    }
    /// The first encoding failure; callers refuse the call rather than sending partial bytes.
    pub fn error(&self) -> Option<WireError> {
        self.error
    }
    /// Records the first failure, preventing subsequent writes.
    pub fn fail(&mut self, error: WireError) {
        self.error.get_or_insert(error);
    }
}

/// What can be written: every [`Wire`] value, and the borrowed forms a call passes
/// (`&str`, `&[T]`, `Option<&T>`), which are written as the owned value they stand for.
pub trait Put {
    /// Writes it.
    fn put_to(&self, out: &mut Writer);
}

impl<T: Wire> Put for T {
    fn put_to(&self, out: &mut Writer) {
        self.put(out);
    }
}

impl Put for str {
    fn put_to(&self, out: &mut Writer) {
        out.put_str(self);
    }
}

impl Wire for &str {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        intern(&input.take::<String>()?)
    }
}

impl<T: Put> Put for [T] {
    fn put_to(&self, out: &mut Writer) {
        out.put_len(self.len());
        for item in self {
            out.put(item);
        }
    }
}

// exhaustive: fields are private; use the codec methods.
/// A buffer values are read from.
pub struct Reader<'a> {
    data: &'a [u8],
    depth: u16,
}

impl<'a> Reader<'a> {
    /// Reads values from a borrowed buffer without copying it.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, depth: 0 }
    }
    /// The bytes not yet read.
    pub fn rest(&self) -> &'a [u8] {
        self.data
    }
    /// Takes exactly `len` bytes, refusing a length beyond the remaining buffer.
    pub fn take_bytes(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let (head, tail) = self.data.split_at_checked(len).ok_or(WireError::Eof)?;
        self.data = tail;
        Ok(head)
    }
    /// A length read before `items` values of at least `min` bytes each: refused when the rest
    /// of the buffer could not hold them, so no allocation outruns its input.
    pub fn take_len(&mut self, min: usize) -> Result<usize, WireError> {
        let len = u32::take(self)? as usize;
        match len.checked_mul(min.max(1)) {
            Some(needed) if needed <= self.data.len() || min == 0 && len <= self.data.len() => {
                Ok(len)
            }
            _ => Err(WireError::Length),
        }
    }
    /// Decodes one value from the current position.
    pub fn take<T: Wire>(&mut self) -> Result<T, WireError> {
        T::take(self)
    }
    /// Read one value from at most `max_bytes`, preserving the enclosing reader's depth and
    /// leaving the following value untouched. No nested allocation can outrun this byte window.
    pub fn bounded<T>(
        &mut self,
        max_bytes: usize,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        let data = self.data;
        let len = data.len().min(max_bytes);
        let mut inner = Self {
            data: &data[..len],
            depth: self.depth,
        };
        let value = read(&mut inner);
        self.data = &data[len - inner.data.len()..];
        value
    }
    /// A list of `u16` written by [`Writer::put_u16s`], read in one pass: the length is checked
    /// against the bytes left before anything is allocated.
    pub(crate) fn take_u16s(&mut self) -> Result<Vec<u16>, WireError> {
        let len = self.take_len(2)?;
        let bytes = self.take_bytes(len * 2)?;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect())
    }
    /// A normal Wire list, with its element cap checked before allocating storage. `read`
    /// may impose a shared payload budget across its elements without a second list codec.
    pub fn list_with<T: Wire>(
        &mut self,
        max_items: usize,
        mut read: impl FnMut(&mut Self) -> Result<T, WireError>,
    ) -> Result<Vec<T>, WireError> {
        let len = self.take_len(T::MIN)?;
        if len > max_items {
            return Err(WireError::Length);
        }
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(read(self)?);
        }
        Ok(items)
    }
    // hidden: transport plumbing used by generated boundary code.
    /// Reads one level down: the depth the value may nest to is [`MAX_DEPTH`].
    pub fn nested<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        if self.depth >= MAX_DEPTH {
            return Err(WireError::Depth);
        }
        self.depth += 1;
        let value = read(self);
        self.depth -= 1;
        value
    }
    /// The whole of `data` as one `T`.
    pub fn decode<T: Wire>(data: &'a [u8]) -> Result<T, WireError> {
        let mut reader = Self::new(data);
        let value = T::take(&mut reader)?;
        match reader.data.is_empty() {
            true => Ok(value),
            false => Err(WireError::Trailing),
        }
    }
}

/// A type with a wire form.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no Wire form",
    label = "add #[derive(mod_api::Wire)] to it"
)]
pub trait Wire: Sized {
    /// Minimum encoded size, used to reject impossible collection lengths.
    const MIN: usize;
    /// Encodes this value, recording failures on the writer.
    fn put(&self, out: &mut Writer);
    /// Decodes one value, leaving any following bytes for the caller.
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError>;
    // hidden: internal Vec encoding specialization, not a mod-facing API
    /// Internal list codec; primitive numbers override this with a contiguous codec.
    #[doc(hidden)]
    fn put_list(items: &[Self], out: &mut Writer) {
        out.put_len(items.len());
        for item in items {
            item.put(out);
        }
    }
    // hidden: internal Vec decoding specialization for contiguous numeric payloads
    #[doc(hidden)]
    fn take_list(input: &mut Reader<'_>) -> Result<Vec<Self>, WireError> {
        input.list_with(usize::MAX, Self::take)
    }
}

macro_rules! numbers {
    ($($ty:ty),*) => {$(
        impl Wire for $ty {
    const MIN: usize = std::mem::size_of::<$ty>();
            fn put(&self, out: &mut Writer) {
                out.put_bytes(&self.to_le_bytes());
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                let bytes = input.take_bytes(std::mem::size_of::<$ty>())?;
                Ok(<$ty>::from_le_bytes(bytes.try_into().expect("a fixed-size slice")))
            }
            fn put_list(items: &[Self], out: &mut Writer) {
                let Some(size) = items.len().checked_mul(<Self as Wire>::MIN).and_then(|n| n.checked_add(4)) else {
                    out.fail(WireError::Length);
                    return;
                };
                if !out.can_fit(size) { return; }
                out.put_len(items.len());
                #[cfg(target_endian = "little")]
                out.put_bytes(bytemuck::cast_slice(items));
                #[cfg(target_endian = "big")]
                for item in items { out.put_bytes(&item.to_le_bytes()); }
            }
            fn take_list(input: &mut Reader<'_>) -> Result<Vec<Self>, WireError> {
                let len = input.take_len(<Self as Wire>::MIN)?;
                let size = len.checked_mul(<Self as Wire>::MIN).ok_or(WireError::Length)?;
                let bytes = input.take_bytes(size)?;
                Ok(bytes.chunks_exact(<Self as Wire>::MIN).map(|chunk| {
                    Self::from_le_bytes(chunk.try_into().expect("a fixed-size chunk"))
                }).collect())
            }
        }
    )*};
}
numbers!(u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, f32, f64);

/// A `usize` crosses as 64 bits: a guest's is 32.
impl Wire for usize {
    const MIN: usize = 8;
    fn put(&self, out: &mut Writer) {
        out.put(&(*self as u64));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        usize::try_from(u64::take(input)?).map_err(|_| WireError::Invalid("a size too large"))
    }
}

impl Wire for bool {
    const MIN: usize = 1;
    fn put(&self, out: &mut Writer) {
        out.put(&u8::from(*self));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::take(input)? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(WireError::Tag(tag.into())),
        }
    }
}

impl Wire for char {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put(&u32::from(*self));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        char::from_u32(u32::take(input)?).ok_or(WireError::Invalid("not a character"))
    }
}

impl Wire for () {
    const MIN: usize = 0;
    fn put(&self, _: &mut Writer) {}
    fn take(_: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(())
    }
}

impl Wire for String {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let len = input.take_len(1)?;
        let bytes = input.take_bytes(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| WireError::Utf8)
    }
}

/// A mod's own text, owned once read: a `Cow<'static, str>` field is how the API takes either.
impl Wire for std::borrow::Cow<'_, str> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        String::take(input).map(Self::Owned)
    }
}

impl Wire for Box<str> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        String::take(input).map(String::into_boxed_str)
    }
}

impl Wire for Arc<str> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        String::take(input).map(Arc::from)
    }
}

impl<T: Wire> Wire for Option<T> {
    const MIN: usize = 1;
    fn put(&self, out: &mut Writer) {
        match self {
            None => out.put(&0u8),
            Some(value) => {
                out.put(&1u8);
                out.put(value);
            }
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::take(input)? {
            0 => Ok(None),
            1 => T::take(input).map(Some),
            tag => Err(WireError::Tag(tag.into())),
        }
    }
}

impl<T: Wire, E: Wire> Wire for Result<T, E> {
    const MIN: usize = 1;
    fn put(&self, out: &mut Writer) {
        match self {
            Ok(value) => {
                out.put(&0u8);
                out.put(value);
            }
            Err(error) => {
                out.put(&1u8);
                out.put(error);
            }
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::take(input)? {
            0 => T::take(input).map(Ok),
            1 => E::take(input).map(Err),
            tag => Err(WireError::Tag(tag.into())),
        }
    }
}

impl<T: Wire> Wire for Arc<[T]> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_len(self.len());
        for item in self.iter() {
            out.put(item);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Vec::<T>::take(input).map(Into::into)
    }
}

impl<T: Wire> Wire for Box<[T]> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_len(self.len());
        for item in self.iter() {
            out.put(item);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Vec::<T>::take(input).map(Into::into)
    }
}

impl<T: Wire> Wire for Box<T> {
    const MIN: usize = T::MIN;
    fn put(&self, out: &mut Writer) {
        out.nested(|out| (**self).put(out));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.nested(T::take).map(Box::new)
    }
}

impl<T: Wire> Wire for Arc<T> {
    const MIN: usize = T::MIN;
    fn put(&self, out: &mut Writer) {
        out.nested(|out| (**self).put(out));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.nested(T::take).map(Arc::new)
    }
}

impl<T: Wire> Wire for Vec<T> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        T::put_list(self, out);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        T::take_list(input)
    }
}

impl<T: Wire> Wire for VecDeque<T> {
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_len(self.len());
        for item in self {
            item.put(out);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Vec::take(input).map(Into::into)
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    const MIN: usize = T::MIN * N;
    fn put(&self, out: &mut Writer) {
        for item in self {
            item.put(out);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut items = Vec::with_capacity(N);
        for _ in 0..N {
            items.push(T::take(input)?);
        }
        items
            .try_into()
            .map_err(|_| WireError::Invalid("an array of the wrong length"))
    }
}

/// Sets and maps cross as a list of their entries; a key that repeats keeps the last.
macro_rules! sets {
    ($($ty:ident [$($bound:tt)+]),*) => {$(
        impl<T: Wire + $($bound)+> Wire for $ty<T> {
    const MIN: usize = 4;
            fn put(&self, out: &mut Writer) {
                out.put_len(self.len());
                for item in self {
                    item.put(out);
                }
            }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                let len = input.take_len(T::MIN)?;
                let mut set = Self::default();
                for _ in 0..len {
                    set.insert(T::take(input)?);
                }
                Ok(set)
            }
        }
    )*};
}
sets!(BTreeSet[Ord], HashSet[Eq + Hash]);

macro_rules! maps {
    ($($ty:ident [$($bound:tt)+]),*) => {$(
        impl<K: Wire + $($bound)+, V: Wire> Wire for $ty<K, V> {
    const MIN: usize = 4;
            fn put(&self, out: &mut Writer) {
                out.put_len(self.len());
                for (key, value) in self {
                    key.put(out);
                    value.put(out);
                }
            }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                let len = input.take_len(K::MIN + V::MIN)?;
                let mut map = Self::default();
                for _ in 0..len {
                    let key = K::take(input)?;
                    map.insert(key, V::take(input)?);
                }
                Ok(map)
            }
        }
    )*};
}
maps!(BTreeMap[Ord], HashMap[Eq + Hash]);

macro_rules! tuples {
    ($(($($name:ident),+)),*) => {$(
        #[allow(non_snake_case)]
        impl<$($name: Wire),+> Wire for ($($name,)+) {

    const MIN: usize = 0 $(+ $name::MIN)+;
            fn put(&self, out: &mut Writer) {
                let ($($name,)+) = self;
                $($name.put(out);)+
            }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                Ok(($($name::take(input)?,)+))
            }
        }
    )*};
}
tuples!(
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F)
);

/// A duration as seconds and nanoseconds.
impl Wire for std::time::Duration {
    crate::wire::layout_fields!("secs" => u64, "nanos" => u32);
    const MIN: usize = 12;
    fn put(&self, out: &mut Writer) {
        out.put(&self.as_secs());
        out.put(&self.subsec_nanos());
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let secs = u64::take(input)?;
        let nanos = u32::take(input)?;
        match nanos < 1_000_000_000 {
            true => Ok(Self::new(secs, nanos)),
            false => Err(WireError::Invalid("nanoseconds past a second")),
        }
    }
}

// Explicit layouts for checked codecs are read by the SDK surface generator.
macro_rules! layout_fields {
    ($($field:literal => $ty:ty),* $(,)?) => {};
}
pub(crate) use layout_fields;
macro_rules! layout_via {
    ($ty:ty) => {};
}

#[cfg(test)]
mod tests {
    #[test]
    fn derive_shapes_roundtrip_with_golden_bytes() {
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Named {
            x: u32,
            text: String,
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Tuple(u32);
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Unit;
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        enum Plain {
            First = 7,
            Second = 9,
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        enum Payload {
            Unit,
            Tuple(u8),
            Named { value: u32 },
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Generic<T> {
            value: T,
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Recursive {
            children: Vec<Recursive>,
        }
        fn check<T: super::Wire + std::fmt::Debug + PartialEq>(value: T, bytes: &[u8]) {
            let mut out = super::Writer::new();
            out.put(&value);
            assert_eq!(out.bytes(), bytes);
            assert_eq!(super::Reader::decode::<T>(bytes).unwrap(), value);
        }
        check(
            Named {
                x: 7,
                text: "ab".into(),
            },
            &[7, 0, 0, 0, 2, 0, 0, 0, 97, 98],
        );
        check(Tuple(7), &[7, 0, 0, 0]);
        check(Unit, &[]);
        check(Plain::First, &[0, 0, 0, 0]);
        check(Plain::Second, &[1, 0, 0, 0]);
        check(Payload::Tuple(5), &[1, 0, 0, 0, 5]);
        check(Payload::Named { value: 7 }, &[2, 0, 0, 0, 7, 0, 0, 0]);
        check(Generic { value: 5u8 }, &[5]);
        check(Recursive { children: vec![] }, &[0, 0, 0, 0]);
    }
    #[test]
    fn derive_field_hygiene_and_unknown_tags() {
        #[derive(mod_api::Wire, Debug, PartialEq)]
        struct Names {
            out: u8,
            input: u32,
        }
        #[derive(mod_api::Wire, Debug, PartialEq)]
        enum Event {
            Data { out: u8, input: u32 },
        }
        round(Names { out: 3, input: 7 });
        round(Event::Data { out: 3, input: 7 });
        assert_eq!(
            Reader::decode::<Event>(&[9, 0, 0, 0]),
            Err(WireError::Tag(9))
        );
        assert!(Reader::decode::<Names>(&[3, 7]).is_err());
    }

    #[test]
    fn derive_via_checks_its_constructor() {
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        #[wire(via = String)]
        struct Nonempty(String);
        impl From<&Nonempty> for String {
            fn from(value: &Nonempty) -> Self {
                value.0.clone()
            }
        }
        impl TryFrom<String> for Nonempty {
            type Error = ();
            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.is_empty() {
                    Err(())
                } else {
                    Ok(Self(value))
                }
            }
        }
        #[derive(mod_api::Wire, Debug, PartialEq)]
        struct Outer {
            first: u8,
            checked: Nonempty,
            last: u32,
        }
        assert_eq!(
            Reader::decode::<Outer>(&[1, 0, 0, 0, 0]),
            Err(WireError::Invalid("Nonempty"))
        );
        let mut out = Writer::new();
        out.put(&Nonempty("ab".into()));
        assert_eq!(out.bytes(), &[2, 0, 0, 0, 97, 98]);
        assert_eq!(
            Reader::decode::<Nonempty>(out.bytes()).unwrap(),
            Nonempty("ab".into())
        );
        assert_eq!(
            Reader::decode::<Nonempty>(&[0, 0, 0, 0]),
            Err(WireError::Invalid("Nonempty"))
        );
    }

    #[test]
    fn golden_value_bytes() {
        fn bytes<T: super::Wire>(value: T, expected: &[u8]) {
            let mut out = super::Writer::new();
            out.put(&value);
            assert_eq!(out.error(), None);
            assert_eq!(out.bytes(), expected);
        }
        bytes(
            crate::Vec3 {
                x: 1.0,
                y: 2.0,
                z: -0.5,
            },
            &[
                0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 224, 191,
            ],
        );
        bytes(crate::Direction::North, &[2, 0, 0, 0]);
        bytes(crate::BlockStateId(7), &[7, 0, 0, 0]);
        bytes(crate::Tag::Byte(5), &[1, 0, 0, 0, 5]);
        bytes(Some(1u16), &[1, 1, 0]);
        bytes(vec!["ab".to_owned()], &[1, 0, 0, 0, 2, 0, 0, 0, 97, 98]);
        bytes(serde_json::json!(1), &[1, 0, 0, 0, 49]);
    }

    use super::*;

    fn round<T: Wire + PartialEq + std::fmt::Debug>(value: T) {
        let mut out = Writer::new();
        out.put(&value);
        assert_eq!(Reader::decode::<T>(out.bytes()), Ok(value));
    }

    #[test]
    fn reduction_adjustments_survive_wire_in_order() {
        use crate::{Reduction::*, ReductionChange::*};
        let changes = vec![
            (Armor, Add(2.5)),
            (Enchantments, Scale(0.5)),
            (MobEffects, AtLeast(3.0)),
            (Absorption, AtMost(4.0)),
            (Invulnerability, Add(-1.0)),
            (InnateResistance, Scale(2.0)),
        ];
        round(changes);
        assert!(Reader::decode::<crate::Reduction>(&[6]).is_err());
        assert!(Reader::decode::<crate::ReductionChange>(&[4, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn provider_particle_state_survives_the_wire() {
        round(crate::ModParticle {
            position: [1.25, -2.5, 3.75],
            previous: [0.0, -1.0, 2.0],
            velocity: [0.1, 0.2, -0.3],
            age: 7,
            lifetime: 31,
            size: 0.5,
            rgba: [0.25, 0.5, 0.75, 1.0],
            frame: 3,
            removed: true,
            state: [8.0, -9.0, 10.0, 11.0],
            facing: crate::ParticleFacing::Fixed([0.5, -0.5, 0.5, 0.5]),
            roll: 0.75,
        });
    }

    #[test]
    fn a_particle_batch_survives_the_wire_in_order() {
        let mut second = crate::ModParticle {
            position: [9.0, 8.0, 7.0],
            previous: [0.0; 3],
            velocity: [0.0; 3],
            age: 2,
            lifetime: 4,
            size: 1.5,
            rgba: [1.0; 4],
            frame: 1,
            removed: false,
            state: [0.0; 4],
            facing: crate::ParticleFacing::AroundY,
            roll: -1.0,
        };
        let first = crate::ModParticle {
            facing: crate::ParticleFacing::Fixed([0.0, 0.0, 0.0, 1.0]),
            ..second
        };
        second.age = 3;
        round(crate::client::particle_providers::particle_batch_from_vec(
            vec![first, second],
        ));
        round(crate::client::particle_providers::particle_batch_from_vec(
            Vec::new(),
        ));
    }

    #[test]
    fn values_survive_a_round_trip() {
        round(7u32);
        round(-3i64);
        round(1.5f64);
        round(u128::MAX);
        round(true);
        round('x');
        round(String::from("héllo"));
        round(Some(vec![1u8, 2, 3]));
        round(None::<u16>);
        round((1u8, String::from("a"), [3u16; 4]));
        round(BTreeMap::from([
            (1u8, String::from("one")),
            (2, String::from("two")),
        ]));
        round(std::time::Duration::new(5, 7));
        round(vec![Some(Box::new(4i32)), None]);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic_or_a_huge_allocation() {
        // A list that promises four billion items in six bytes.
        assert_eq!(
            Reader::decode::<Vec<u64>>(&[0xff, 0xff, 0xff, 0xff, 0, 0]),
            Err(WireError::Length)
        );
        assert_eq!(Reader::decode::<u32>(&[1, 2]), Err(WireError::Eof));
        assert_eq!(Reader::decode::<bool>(&[2]), Err(WireError::Tag(2)));
        assert_eq!(Reader::decode::<Option<u8>>(&[9]), Err(WireError::Tag(9)));
        assert_eq!(
            Reader::decode::<String>(&[2, 0, 0, 0, 0xff, 0xfe]),
            Err(WireError::Utf8)
        );
        assert_eq!(Reader::decode::<u8>(&[1, 2]), Err(WireError::Trailing));
        assert!(Reader::decode::<char>(&0xd800u32.to_le_bytes()).is_err());
        assert!(
            Reader::decode::<std::time::Duration>(
                &[0; 8]
                    .iter()
                    .chain(&[0xff; 4])
                    .copied()
                    .collect::<Vec<u8>>()
            )
            .is_err()
        );
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        let mut bytes = vec![1u8; MAX_DEPTH as usize + 1];
        bytes.push(0);
        let mut shallow = bytes[bytes.len() - MAX_DEPTH as usize..].to_vec();
        shallow.push(0);
        // `Box<Option<…>>` chains are not a real type; a depth guard is exercised directly.
        let mut reader = Reader::new(&bytes);
        fn dive(reader: &mut Reader<'_>, left: usize) -> Result<(), WireError> {
            match left {
                0 => Ok(()),
                _ => reader.nested(|r| dive(r, left - 1)),
            }
        }
        assert_eq!(dive(&mut reader, MAX_DEPTH as usize), Ok(()));
        assert_eq!(
            dive(&mut reader, MAX_DEPTH as usize + 1),
            Err(WireError::Depth)
        );
        // The depth unwinds: a refused read leaves the reader reusable.
        assert_eq!(dive(&mut reader, 3), Ok(()));
    }
}

#[cfg(test)]
mod depth_tests;
mod json_tree;
mod tag_tree;
mod text_tree;
pub(crate) mod wire_types;
