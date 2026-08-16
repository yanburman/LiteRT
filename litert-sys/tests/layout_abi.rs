//! Guards `LiteRtLayout`'s in-memory layout against the `static_assert`s that
//! `litert/c/litert_layout.h` and `litert/c/litert_model_types.h` carry.
//!
//! `build.rs` runs bindgen with `layout_tests(false)`, so nothing else in this
//! crate notices when a regenerated binding file disagrees with the header it
//! was generated from. That gap shipped a real bug once: through LiteRT 2.1.6
//! `has_strides` was a `bool : 1` bitfield next to an `unsigned int rank : 7`,
//! MSVC declined to coalesce the two into one storage unit, and the
//! Windows-generated binding file — copied to every target — put `dimensions`
//! at offset 8 instead of 4. Every tensor shape read on Android came back
//! shifted by one `i32` ([1, 3, 640, 640] logged as [3, 640, 640, 0]), with no
//! compile error anywhere.
//!
//! LiteRT 2.2.0 made `has_strides` an `unsigned int : 1` (upstream issue 7459),
//! so a single layout is now correct for every 64-bit target. Both the header's
//! asserts and these are unconditional on the compiler for that reason.
//!
//! The upstream asserts are themselves gated on a 64-bit pointer, so this test
//! is too — the wasm32 bindings are 32-bit and pinned to an older LiteRT.

#![cfg(target_pointer_width = "64")]

use std::mem::{offset_of, size_of};

use litert_sys::{LiteRtLayout, LiteRtRankedTensorType};

#[test]
fn litert_layout_matches_header_static_asserts() {
    assert_eq!(size_of::<LiteRtLayout>(), 68, "LiteRtLayout size mismatch");
    assert_eq!(
        offset_of!(LiteRtLayout, dimensions),
        4,
        "LiteRtLayout dimensions offset mismatch"
    );
    assert_eq!(
        offset_of!(LiteRtLayout, strides),
        36,
        "LiteRtLayout strides offset mismatch"
    );
}

#[test]
fn ranked_tensor_type_matches_header_static_asserts() {
    assert_eq!(
        size_of::<LiteRtRankedTensorType>(),
        72,
        "LiteRtRankedTensorType size mismatch"
    );
    assert_eq!(
        offset_of!(LiteRtRankedTensorType, layout),
        4,
        "LiteRtRankedTensorType layout offset mismatch"
    );
}

/// `rank` and `has_strides` must share one 4-byte storage unit — the property
/// that makes the layout above compiler-independent.
#[test]
fn rank_and_has_strides_share_a_storage_unit() {
    let mut layout = LiteRtLayout::default();
    layout.set_rank(4);
    layout.set_has_strides(1);
    assert_eq!(layout.rank(), 4);
    assert_eq!(layout.has_strides(), 1);

    // Writing the bitfields must not reach into `dimensions`.
    assert_eq!(layout.dimensions, [0; 8]);
}
