//! The bytes: little-endian, written without a schema and read checked.

use crate::{Error, MAX_COORDINATE};

/// What a fragment is written into.
#[derive(Default)]
pub struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    pub fn bool(&mut self, v: bool) {
        self.out.push(u8::from(v));
    }

    pub fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i16(&mut self, v: i16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f32(&mut self, v: f32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f64(&mut self, v: f64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bytes(&mut self, v: &[u8]) {
        self.out.extend_from_slice(v);
    }

    /// A count. One past `u32::MAX` is written as that: past every limit
    /// a reader has, so it is refused there.
    pub fn count(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
}

/// What a fragment is read from: every read checked against what is left.
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    pub const fn left(&self) -> usize {
        self.rest.len()
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let (taken, rest) = self.rest.split_at_checked(n).ok_or(Error::Truncated)?;
        self.rest = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?.try_into().map_err(|_| Error::Truncated)
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    pub fn bool(&mut self) -> Result<bool, Error> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    /// A count, as a length.
    pub fn count(&mut self) -> Result<usize, Error> {
        usize::try_from(self.u32()?).map_err(|_| Error::OutOfRange)
    }

    /// An `f32`, finite and in range.
    pub fn f32(&mut self) -> Result<f32, Error> {
        let v = f32::from_le_bytes(self.array()?);
        in_range(f64::from(v)).map(|_| v)
    }

    /// An `f64`, finite and in range.
    pub fn f64(&mut self) -> Result<f64, Error> {
        in_range(f64::from_le_bytes(self.array()?))
    }

    /// A coordinate: an `f32` or an `f64` as `wide` says, as an `f64`.
    pub fn coord(&mut self, wide: bool) -> Result<f64, Error> {
        if wide {
            self.f64()
        } else {
            self.f32().map(f64::from)
        }
    }
}

fn in_range(v: f64) -> Result<f64, Error> {
    if v.is_finite() && v.abs() <= MAX_COORDINATE {
        Ok(v)
    } else {
        Err(Error::OutOfRange)
    }
}

/// `v` as an `f32`, when that is the same number.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]
pub fn narrow(v: f64) -> Option<f32> {
    let n = v as f32;
    (f64::from(n) == v).then_some(n)
}
