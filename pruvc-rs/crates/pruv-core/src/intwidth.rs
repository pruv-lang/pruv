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
        to.min <= self.min && self.max <= to.max
    }

    /// True when `v` is a value of this type.
    pub fn contains(&self, v: i128) -> bool {
        self.min <= v && v <= self.max
    }
}

pub const I64: IntWidth = IntWidth { name: "i64", signed: true, bits: 64, min: i64::MIN as i128, max: i64::MAX as i128 };
pub const U8: IntWidth = IntWidth { name: "u8", signed: false, bits: 8, min: 0, max: 255 };
pub const U16: IntWidth = IntWidth { name: "u16", signed: false, bits: 16, min: 0, max: 65535 };

/// All scalar integer types, in declaration order.
pub const SCALAR_INTS: [IntWidth; 3] = [I64, U8, U16];

/// Integer type names that are reserved for later bundles (P3) and rejected today.
pub const PENDING_INTS: [&str; 7] = ["u32", "i32", "usize", "isize", "u64", "i8", "i16"];

/// The descriptor of a scalar integer type name, if it is one.
pub fn of(name: &str) -> Option<&'static IntWidth> {
    SCALAR_INTS.iter().find(|w| w.name == name)
}

/// True for the narrow scalar integer types added in P1 (`u8`, `u16`).
pub fn is_narrow(name: &str) -> bool {
    matches!(of(name), Some(w) if w.bits < 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(of("u8"), Some(&U8));
        assert_eq!(of("u16").unwrap().max, 65535);
        assert_eq!(of("i64").unwrap().min, -(1i128 << 63));
        assert!(of("u32").is_none());
        assert!(U8.fits_in(&U16) && U16.fits_in(&I64) && !I64.fits_in(&U16));
        assert!(is_narrow("u8") && !is_narrow("i64"));
    }
}
