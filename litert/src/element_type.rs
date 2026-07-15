//! Tensor element types and the `TensorElement` trait.

use litert_sys as sys;

/// Scalar element type of a tensor, in one-to-one correspondence with the
/// LiteRT C enum `LiteRtElementType`. Variant names track the Rust primitive
/// types or IEEE/INT naming (`Float16`, `Int32`) where no Rust analogue exists.
///
/// Discriminants are the literal `LiteRtElementType` wire values (stable
/// across 2.1.4–2.1.6, per the upstream header's `// kTfLiteXxx` comments)
/// rather than `sys::kLiteRtElementTypeXxx` — bindgen infers a *different*
/// Rust type for those constants depending on the target ABI it's generating
/// for (`c_int` under MSVC, `c_uint` under Itanium/Linux/Android/macOS, for
/// this specific plain C enum), which doesn't match a single fixed `#[repr]`
/// here. Literal values sidestep that entirely.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum ElementType {
    None = 0,
    Bool = 6,
    Int2 = 20,
    Int4 = 18,
    Int8 = 9,
    Int16 = 7,
    Int32 = 2,
    Int64 = 4,
    UInt8 = 3,
    UInt16 = 17,
    UInt32 = 16,
    UInt64 = 13,
    Float16 = 10,
    BFloat16 = 19,
    Float32 = 1,
    Float64 = 11,
    Complex64 = 8,
    Complex128 = 12,
}

impl ElementType {
    pub(crate) fn from_raw(raw: sys::LiteRtElementType) -> Self {
        match raw {
            sys::kLiteRtElementTypeBool => Self::Bool,
            sys::kLiteRtElementTypeInt2 => Self::Int2,
            sys::kLiteRtElementTypeInt4 => Self::Int4,
            sys::kLiteRtElementTypeInt8 => Self::Int8,
            sys::kLiteRtElementTypeInt16 => Self::Int16,
            sys::kLiteRtElementTypeInt32 => Self::Int32,
            sys::kLiteRtElementTypeInt64 => Self::Int64,
            sys::kLiteRtElementTypeUInt8 => Self::UInt8,
            sys::kLiteRtElementTypeUInt16 => Self::UInt16,
            sys::kLiteRtElementTypeUInt32 => Self::UInt32,
            sys::kLiteRtElementTypeUInt64 => Self::UInt64,
            sys::kLiteRtElementTypeFloat16 => Self::Float16,
            sys::kLiteRtElementTypeBFloat16 => Self::BFloat16,
            sys::kLiteRtElementTypeFloat32 => Self::Float32,
            sys::kLiteRtElementTypeFloat64 => Self::Float64,
            sys::kLiteRtElementTypeComplex64 => Self::Complex64,
            sys::kLiteRtElementTypeComplex128 => Self::Complex128,
            _ => Self::None,
        }
    }
}

/// Marker trait for Rust scalars that correspond to a LiteRT `ElementType`.
///
/// Implemented for `bool`, `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`,
/// `u64`, `f32`, `f64`.
///
/// # Safety
///
/// Implementors promise that `Self` is `Sized`, `Copy`, has no drop glue, and
/// that a `&[Self]` can be safely reinterpreted from a `*mut u8` buffer of
/// `N * size_of::<Self>()` bytes that LiteRT returned as a tensor whose
/// element type equals [`Self::TYPE`].
pub unsafe trait TensorElement: Copy + Sized + 'static {
    /// The `ElementType` corresponding to `Self`.
    const TYPE: ElementType;

    /// Human-readable name, used in error messages.
    const NAME: &'static str;
}

macro_rules! impl_tensor_element {
    ($($rust:ty => ($variant:ident, $name:literal)),* $(,)?) => {
        $(
            unsafe impl TensorElement for $rust {
                const TYPE: ElementType = ElementType::$variant;
                const NAME: &'static str = $name;
            }
        )*
    };
}

impl_tensor_element! {
    bool => (Bool,    "bool"),
    i8   => (Int8,    "i8"),
    i16  => (Int16,   "i16"),
    i32  => (Int32,   "i32"),
    i64  => (Int64,   "i64"),
    u8   => (UInt8,   "u8"),
    u16  => (UInt16,  "u16"),
    u32  => (UInt32,  "u32"),
    u64  => (UInt64,  "u64"),
    f32  => (Float32, "f32"),
    f64  => (Float64, "f64"),
}
