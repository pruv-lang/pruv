//! Integer width descriptor (bundle P1): the single table of scalar integer types.
//!
//! Every stage that needs the width, signedness or value range of a scalar integer type reads it here:
//! the checker (literal ranges, type rules), the proof model (range facts and overflow obligations),
//! the Rust backend (type spelling) and the LLVM backend (IR width, signed vs unsigned instructions).
//! A type that is not in [`SCALAR_INTS`] is not a scalar integer.

/// One scalar integer type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntWidth {
    /// Pruv type name, also the Rust type name (`i64`, `u8`, `u16`).
    pub name: &'static str,
    pub signed: bool,
    pub bits: u32,
    pub min: i128,
    pub max: i128,
}

impl IntWidth {
    /// LLVM IR integer type (`i64`, `i8`, `i16`).
    pub fn llvm(&self) -> &'static str {
        match self.bits {
            8 => "i8",
            16 => "i16",
            32 => "i32",
            _ => "i64",
        }
    }

    /// True when every value of `self` is a value of `to` (widening or same type).
    pub fn fits_in(&self, to: &IntWidth) -> bool {
        to.min <= self.min && self.max <= to.max && !(to.is_char() && !self.is_char() && self.max >= SURROGATE_LO)
    }

    /// P7: the `char` type (a Unicode scalar value, SPEC §6t).
    pub fn is_char(&self) -> bool {
        self.name == "char"
    }

    /// True when `v` is a value of this type.
    pub fn contains(&self, v: i128) -> bool {
        self.min <= v && v <= self.max && !(self.is_char() && (SURROGATE_LO..=SURROGATE_HI).contains(&v))
    }
}

pub const I64: IntWidth = IntWidth { name: "i64", signed: true, bits: 64, min: i64::MIN as i128, max: i64::MAX as i128 };
pub const U8: IntWidth = IntWidth { name: "u8", signed: false, bits: 8, min: 0, max: 255 };
pub const U16: IntWidth = IntWidth { name: "u16", signed: false, bits: 16, min: 0, max: 65535 };
pub const U32: IntWidth = IntWidth { name: "u32", signed: false, bits: 32, min: 0, max: u32::MAX as i128 };
pub const I32: IntWidth = IntWidth { name: "i32", signed: true, bits: 32, min: i32::MIN as i128, max: i32::MAX as i128 };
/// Pruv targets 64-bit only (SPEC R-6p-5): `usize` is 64-bit unsigned, `isize` is 64-bit signed.
pub const USIZE: IntWidth = IntWidth { name: "usize", signed: false, bits: 64, min: 0, max: u64::MAX as i128 };
pub const ISIZE: IntWidth = IntWidth { name: "isize", signed: true, bits: 64, min: i64::MIN as i128, max: i64::MAX as i128 };
pub const U64: IntWidth = IntWidth { name: "u64", signed: false, bits: 64, min: 0, max: u64::MAX as i128 };
pub const I8: IntWidth = IntWidth { name: "i8", signed: true, bits: 8, min: i8::MIN as i128, max: i8::MAX as i128 };
pub const I16: IntWidth = IntWidth { name: "i16", signed: true, bits: 16, min: i16::MIN as i128, max: i16::MAX as i128 };

/// P7: `char` is stored as a 32-bit code and modelled like an integer width whose values are the Unicode scalar
/// values: 0..=0x10FFFF without the surrogate range [`SURROGATE_LO`]..=[`SURROGATE_HI`] (SPEC R-6t-4).
/// It has no arithmetic and converts only to and from `u32` (and from `u8`).
pub const CHAR: IntWidth = IntWidth { name: "char", signed: false, bits: 32, min: 0, max: 0x10FFFF };
pub const SURROGATE_LO: i128 = 0xD800;
pub const SURROGATE_HI: i128 = 0xDFFF;

/// All scalar integer types, in declaration order.
pub const SCALAR_INTS: [IntWidth; 11] = [I64, U8, U16, U32, I32, USIZE, ISIZE, U64, I8, I16, CHAR];

/// Integer type names that are reserved for later bundles and rejected today (none since P3).
pub const PENDING_INTS: [&str; 0] = [];

/// The descriptor of a scalar integer type name, if it is one.
pub fn of(name: &str) -> Option<&'static IntWidth> {
    SCALAR_INTS.iter().find(|w| w.name == name)
}

/// True for every scalar integer type other than `i64` (P1 `u8`/`u16`, P3 widths).
/// These share the P1 rules: no host boundary, no type arguments, typed literals and explicit casts.
pub fn is_narrow(name: &str) -> bool {
    matches!(of(name), Some(w) if w.name != "i64")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(of("u8"), Some(&U8));
        assert_eq!(of("u16").unwrap().max, 65535);
        assert_eq!(of("i64").unwrap().min, -(1i128 << 63));
        assert!(of("u128").is_none());
        assert!(CHAR.contains(0x41) && !CHAR.contains(0xD800) && !CHAR.contains(0x110000));
        assert!(U8.fits_in(&CHAR) && !U16.fits_in(&CHAR) && CHAR.fits_in(&U32) && !U32.fits_in(&CHAR));
        assert_eq!(of("u64").unwrap().max, u64::MAX as i128);
        assert!(U32.fits_in(&I64) && !U64.fits_in(&I64) && I8.fits_in(&I16) && !I8.fits_in(&U8));
        assert!(U8.fits_in(&U16) && U16.fits_in(&I64) && !I64.fits_in(&U16));
        assert!(is_narrow("u8") && is_narrow("usize") && !is_narrow("i64"));
    }
}
