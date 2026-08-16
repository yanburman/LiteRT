# Upgrading the vendored LiteRT C headers

How we bumped `litert-sys` from LiteRT v2.1.4 → v2.1.6 (branch
`litert-2.1.6-support`), and what to do differently next time.

## Where the new headers actually come from

The public "SDK" bundle (what Google calls `litert_cc_sdk`) is **not** the
same tree as the full `google-ai-edge/LiteRT` source checkout. It's a much
smaller, curated bundle: `litert/c/**`, `litert/cc/**`, a handful of CMake
files, no test/tooling/internal-engine sources. `litert-sys` only vendors
`litert/c/**` (the plain-C surface `wrapper.h` includes), so pull headers
from the `litert_cc_sdk` bundle for the target version, not from a clone of
the main LiteRT repo — the latter also has internal-only headers
(`litert_accelerator*.h`, `litert_dispatch_delegate.h`, etc.) that aren't
part of the public C API and aren't shipped in the SDK bundle at all.

## What we actually vendored

`third_party/litert-v2.1.6/` mirrors `third_party/litert-v2.1.4/`'s layout,
but note two deliberate differences from "just copy everything":

1. **`litert/build_common/build_config.h` was *not* copied from the new SDK.**
   The SDK only ships `build_config.h.in` (a CMake `configure_file`
   template) — the real header is generated during CMake configure, which
   we don't run. We reused the old vendor tree's hand-written
   `build_config.h` (`#define LITERT_DISABLE_NPU`, GPU left on) as-is: the
   template's guard macros (`LITERT_BUILD_CONFIG_DISABLE_GPU/NPU`) didn't
   change between versions, so the old substituted output is still valid.
   **Check this by diffing the `.h.in` template against the previous
   version's template before assuming this still holds.**

2. **We dropped every file listed as "only in 2.1.4" during the diff** —
   `litert_builder.h`, `litert_webnn_options.h`, the whole `internal/`
   engine-header set (`litert_accelerator*.h`, `litert_dispatch_delegate.h`,
   `litert_runtime_context.h`, etc.), `windows_exported_symbols.def`,
   `litert_runtime_c_api_so_symbols.txt`, the `.mm` test file. Before
   assuming any of these matter: `grep -rn "<header-or-symbol-name>"` across
   the whole repo excluding `third_party/` — none of them were referenced
   anywhere outside the vendor tree itself (not by `wrapper.h`, not by any
   `.rs`). They were vendored wholesale in the original 2.1.4 import but
   never actually needed.

## CORRECTION (found during device validation): bindings are NOT fully target-independent

**The claim originally in this section was wrong and caused a real,
shipped bug** — recorded here instead of silently deleted so the mistake
doesn't get repeated. Original reasoning: all 6 committed per-target
binding files came out byte-identical after the first regen, and
`generate_bindings(out_dir)` never passed a `--target=<triple>` clang arg to
bindgen, so it seemed safe to generate once natively and copy everywhere.

That's true for every *plain* declaration (opaque pointers, ints, ordinary
enums) — but **`LiteRtLayout` has two bitfields** (`unsigned int rank : 7`,
`bool has_strides : 1`) ahead of its `dimensions[8]` array, and bitfield
packing genuinely differs between the MSVC ABI and the Itanium ABI (Linux/
Android/macOS). bindgen inherits whatever ABI libclang's *default* target
uses. Regenerating natively on this Windows host produced the MSVC layout
(`dimensions` at byte offset 8) for **every** target file, including the
Android/Linux/macOS ones, which need the Itanium layout (`dimensions` at
offset 4 — confirmed by the header's own `static_assert(offsetof(...) == 4
/* non-MSVC */ : 8 /* MSVC */)`). Every tensor shape read on the real
(Android arm64) device came back shifted by one `i32` — e.g. a true `[1, 3,
640, 640]` input shape logged as `[3, 640, 640, 0]` — because the Rust
struct had 4 bytes of spurious extra padding before `dimensions` that the
real 68-byte Android struct doesn't have. Building and `cargo check`-ing
happily the whole time — this is a runtime data-corruption bug, not a
compile error, so nothing caught it until an actual device run.

The same target-dependence turned out to also apply to **plain C enums
without an explicit underlying type** — bindgen infers `c_int` for
`LiteRtElementType`'s constants under MSVC's default target but `c_uint`
under Itanium's. `litert/src/element_type.rs`'s `ElementType` enum no
longer depends on whichever type bindgen happens to infer for
`sys::kLiteRtElementTypeXxx` on a given host — it now spells out the
literal wire values directly (see that file's doc comment) instead of
`= sys::kLiteRtElementTypeXxx`.

**Fix:** `litert-sys/build.rs`'s `generate_bindings()` now honors a
`LITERT_SYS_BINDGEN_ABI=itanium` env var, which adds
`--target=x86_64-unknown-linux-gnu` as a clang arg — enough to make bindgen
pick Itanium-ABI bitfield packing without needing a real sysroot for that
target (our headers only pull in `<stdint.h>`/`<stdbool.h>`/`<stddef.h>`,
which clang bundles regardless of target). Generate twice:

```powershell
# Windows (MSVC ABI) — leave the env var unset
cargo build -p litert-sys --features generate-bindings --target x86_64-pc-windows-msvc --release
# → copy to src/bindings/x86_64-pc-windows-msvc.rs

# Everyone else (Itanium ABI) — same host, different env var
$env:LITERT_SYS_BINDGEN_ABI = "itanium"
cargo build -p litert-sys --features generate-bindings --target x86_64-pc-windows-msvc --release
# → copy to the other 5: aarch64-apple-darwin, x86_64-unknown-linux-gnu,
#   aarch64-unknown-linux-gnu, aarch64-linux-android, x86_64-linux-android
```

**Before trusting a "generate once, copy everywhere" regen again**: diff
the generated `LiteRtLayout` (or any bitfield-bearing struct) field offsets
against the header's own `static_assert`s for the actual target ABI you're
shipping to — don't just diff the 6 output files against *each other*.
Identical-to-each-other is not the same as correct.

---

The rest of this section (now historical — the underlying regen mechanics
are still accurate, just not the "no target dependence" conclusion): the
README's "Regenerating bindings" section implies you need `cross` +
Docker/Podman for every foreign target (Linux, Android), and `cargo xtask
regen-bindings` still works and is the CI-sanctioned path if you have that
set up. But if you're offline or don't have `cross`/Docker, don't let that
block you either — a single
native `cargo build -p litert-sys --features generate-bindings --target
<host-triple>` run is sufficient, verified by diffing its output against
the previously-committed files for a couple of other targets.

If some *future* header introduces a `#ifdef __LP64__`-style split or an
actual `long`/platform-sized field, this assumption breaks — diff the
freshly generated output against more than one target's committed file
before trusting a single run again.

## Machine-specific blockers hit while regenerating (all resolved)

None of these are LiteRT/bindings problems — all machine/tooling quirks that
cost real time to diagnose. Worth checking for these first, in this order,
before assuming a fresh regen attempt is broken:

1. **`CRYPT_E_NO_REVOCATION_CHECK` downloading from crates.io.** Windows
   schannel couldn't complete a certificate-revocation check on this
   network (proxy/firewall blocking OCSP/CRL, not a cargo or LiteRT issue).
   Fix: `$env:CARGO_HTTP_CHECK_REVOKE = "false"` (or `[http] check-revoke =
   false` in `.cargo/config.toml` — cam_poc's own config already carries
   this exact setting, so it's a known quantity on this machine, not a new
   workaround). Same underlying cause also blocked reaching
   `dl.google.com/android/maven2/...` for a real 2.1.6 AAR — still relevant
   if re-pinning `LITERT_MAVEN_VERSION` later.
2. **Avast blocking `build-script-build.exe` specifically by name.**
   Manifested as `Access is denied (os error 5)` copying/linking a
   freshly-compiled build-script binary into place — reproduced
   deterministically (same file, every attempt, even after `rm -rf
   target/`), and *not* fixed by a folder-level Avast exception. Proved it
   was name-specific by copying the same bytes to a different filename by
   hand (worked instantly). Root cause: Avast's Behavior/File Shield
   heuristically flags the literal filename `build-script-build.exe` — it's
   identical across every crate with a build script, a very recognizable
   pattern. Fix: find the actual blocked-item entry in Avast's History log
   (not just adding a path exclusion) and whitelist that specific
   detection.
3. **`LNK1140: limit exceeded for program database`.** MSVC's PDB writer
   hit an internal limit under bindgen's large debug-mode dependency tree
   (rustls, regex, clang-sys, ureq, ...). Building the *bindgen-generation*
   step in `--release` isn't enough by itself — rustc still emits a minimal
   PDB on `-msvc` targets even in release unless told not to. Fix:
   `--release` **and** `RUSTFLAGS="-C link-arg=/PDB:NONE"` together.
4. **Disk space.** One run failed with `os error 112: not enough space on
   the disk` on `E:`. Unrelated to the above but easy to mistake for one of
   them if it happens mid-sequence — check free space before re-diagnosing
   a linker error.

## What's left / known gaps

- **The default (no `LITERT_LIB_DIR`) Android download path is still
  2.1.4.** `LITERT_MAVEN_VERSION` in `litert-sys/build.rs` needs a real
  `com.google.ai.edge.litert:litert:2.1.6` AAR pulled from Maven and its
  SHA-256/size re-pinned (`ANDROID_AAR_SHA256`/`ANDROID_AAR_SIZE`) before
  this is safe for any consumer that doesn't override `LITERT_LIB_DIR`.
  cam_poc always overrides, so it isn't blocked by this, but the fork
  shouldn't be published/merged to `main` with this gap open.
- **The `win_x86_64` 2.1.6 prebuilt Google gave us has no import library.**
  `lite_rt_bin_2.1.6/win_x86_64/` ships `libLiteRt.dll` +
  `libLiteRtWebGpuAccelerator.dll` but no `LiteRt.lib` — MSVC can't link
  a DLL without one. `cargo build -p litert-sys -p litert --target
  x86_64-pc-windows-msvc --release` succeeds (compiles the crates), but
  linking an actual test/example binary against it fails with `LNK1181:
  cannot open input file 'LiteRt.lib'`. The old `lite_rt_bin/win_x86_64`
  (2.1.5-era) has `LiteRt.lib`/`.def`/`.exp` alongside the DLL — whatever
  produced the 2.1.6 Windows package skipped generating those. Not blocking
  for cam_poc (Android is the real target and doesn't need an import lib —
  see below), but blocks running `litert`'s own desktop test suite against
  2.1.6 until an import lib is generated (`lib.exe /def:LiteRt.def` if a
  matching `.def` can be produced, or relink with `--export-all-symbols`).

## Validated

```powershell
$env:CARGO_HTTP_CHECK_REVOKE = "false"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"

# 1. Regenerate bindings (native run only — no cross/Docker needed, see above)
cargo build -p litert-sys --features generate-bindings --target x86_64-pc-windows-msvc --release

# 2. Copy the result to every target's binding file (all 6 non-wasm targets
#    are byte-identical — confirmed again after this regen)

# 3. Build + link against the real 2.1.6 binaries on the actual target that
#    matters (Android arm64 — this is what crashed) — SUCCEEDED:
$env:LITERT_LIB_DIR = "E:/android/cam_poc_materials/lite_rt_bin_2.1.6/android_arm64"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = "C:/Users/Darth/AppData/Local/Android/Sdk/ndk/30.0.14904198/toolchains/llvm/prebuilt/windows-x86_64/bin/aarch64-linux-android31-clang.cmd"
cargo build -p litert-sys -p litert --target aarch64-linux-android --release
```

Two knock-on type mismatches surfaced when compiling `litert` against the
new bindings (both from the header diff above, both fixed on this branch):

- `element_type.rs`: `LiteRtElementType`'s bindgen-inferred underlying type
  changed `c_uint` → `c_int` between versions (same enum, no obvious reason
  in the header diff — the new members added don't require a sign change,
  so this looks like a bindgen heuristic shift rather than an intentional
  upstream change). Fixed by changing `ElementType`'s `#[repr(u32)]` to
  `#[repr(i32)]`.
- `logging.rs`: `LiteRtLogSeverity` went from a real enum type to a bare
  `typedef int8_t` with a separate anonymous `enum` for the constants (see
  the header diff), so `kLiteRtLogSeverity*` constants no longer share a
  type with `LiteRtLogSeverity` itself. Fixed with an `as
  sys::LiteRtLogSeverity` cast in `LogSeverity::to_raw()`.

**Still not run:** the actual test suite (`cargo test`) against real 2.1.6
binaries — blocked on the missing Windows import library (see above).
`cargo build`/`check` for both `litert-sys` and `litert` succeed cleanly on
both `x86_64-pc-windows-msvc` and `aarch64-linux-android` against the real
2.1.6 native libraries.

## Remaining follow-up

```
# Point cam_poc's Cargo.toml at this branch (git dep) instead of
# crates.io litert/litert-sys 0.2.1, and update
# crates/detect/src/backends/litert.rs's `Model::from_bytes(cfg.model.to_vec())`
# call to `Model::from_bytes(&env, cfg.model.to_vec())` (env is already
# constructed earlier in that function).
```

## The actual API diff (2.1.4 → 2.1.6), for reference

Full diff of every header `wrapper.h` includes (line-ending differences
ignored — the old vendor tree is CRLF, the new SDK bundle is LF):

- **`litert_model.h` (the breaking change):** `LiteRtCreateModelFromFile`,
  `LiteRtCreateModelFromBuffer` both gained a leading `LiteRtEnvironment
  environment` parameter. `LiteRtCreateModelFromFd` (new) and
  `LiteRtCreateModelFromAllocation` (previously C++-only, unreachable from
  bindgen) also take one now, the latter via a new opaque `LiteRtAllocation`
  typedef that gives it real C linkage. **This is the root cause of the
  segfault this branch exists to fix** — our bindings (frozen at v2.1.4)
  call the old 3-arg form; on the real 2.1.6 `libLiteRt.so` every argument
  after the first shifts one register right, so `buffer_size` lands where
  `buffer_addr` is expected and the native model-buffer verifier segfaults
  dereferencing it.
- Additive-only (no action needed): `LiteRtGetBlockWiseQuantization`,
  `LiteRtGetCustomOptions`, `LiteRtGetSubgraphName` (`litert_model.h`),
  `LiteRtEnvironmentSupportsFP16` (`litert_environment.h`),
  `LiteRtGetCompiledModelEnvironment` (`litert_compiled_model.h`), several
  new option getters/setters in `litert_op_options.h` and
  `litert/c/options/*.h`.
- **Removed** (present in 2.1.4, gone in 2.1.6 — checked, unused by our safe
  wrapper): `LiteRtCustomTensorBufferHandlers` struct and
  `LiteRtSetEnvironmentOptionsValue` (`litert_environment_options.h`).
- New element types (`kLiteRtElementTypeUInt4`,
  `kLiteRtElementTypeFloat8E4M3FN/E5M2`) and new `static_assert`-pinned
  layouts for the quantization structs (`litert_model_types.h`) — additive,
  harmless for us.
- `LiteRtHwAccelerators` changed from an anonymous `typedef enum : int` to a
  named `enum LiteRtHwAccelerators` with an explicit `-1` dummy member
  "to force a signed 32-bit underlying type for Rust bindgen compatibility"
  — a deliberate upstream accommodation for bindgen, shouldn't change our
  generated bitflag values.
- `LiteRtLogSeverity` changed from an actual enum type to a plain `typedef
  int8_t` + anonymous `enum { ... }` for the constants — check the
  generated Rust type for `logging.rs` after regenerating; it was an enum
  alias before and may now bindgen to a bare `i8`.

## litert crate changes already made (this branch)

`Model::from_file`/`Model::from_bytes` now take `&Environment` as their
first parameter (mirrors `CompiledModel::new`, `TensorBuffer::managed_host`,
which already threaded environment through in 2.1.4). Updated every call
site: `litert/src/{model,signature,compiled_model,lib}.rs` doc examples,
`litert/examples/{add_cpu,add_gpu,add_wasm,image_classification}.rs`,
`litert/tests/{inference,inference_gpu,lifecycle}.rs`. All of these already
constructed `Environment` before the `Model::from_file`/`from_bytes` call,
so it was a mechanical `&env` insertion, not a reordering.

---

# 2.1.6 → 2.2.0 (branch `litert-2.2.0-support`)

Much smaller than the previous bump. **No function was removed and no
existing signature changed**, so the argument-shift class of failure that
made 2.1.4 → 2.1.6 a segfault does not apply here. All 329 functions the
crate binds still exist verbatim, and `LITERT_API_VERSION_{MAJOR,MINOR,PATCH}`
is unchanged at 0/1/0.

## The one breaking change: `LiteRtLayout` on MSVC

`litert/c/litert_layout.h` changed `bool has_strides : 1` to
`unsigned int has_strides : 1` (upstream issue 7459) and deleted the
`_MSC_VER` branch of its `static_assert`s:

| | 2.1.6 MSVC | 2.1.6 Itanium | 2.2.0 (both) |
|---|---|---|---|
| `sizeof(LiteRtLayout)` | 72 | 68 | 68 |
| `offsetof(dimensions)` | 8 | 4 | 4 |
| `offsetof(strides)` | 40 | 36 | 36 |
| `sizeof(LiteRtRankedTensorType)` | 76 | 72 | 72 |

So this bump **silently invalidates any Windows `libLiteRt.dll` older than
2.2.0**. Verified concretely on this branch: with the regenerated 2.2.0
bindings against the 2.1.6 Windows DLL, `litert/tests/inference.rs` fails
with `left: [0, 10], right: [10, 10]` — the shape read one `i32` early —
while `smoke.rs`, `lifecycle.rs`'s non-shape tests, and everything on
Android still pass. That is the *same* corruption signature as the 2.1.6
regen bug documented above, just with the ABIs swapped.

Consequences already applied:

- All six 64-bit binding files regenerated; `x86_64-pc-windows-msvc.rs` lost
  its `__bindgen_padding_0: u32`.
- `has_strides()`/`set_has_strides()` now take/return `c_uint`, not `bool`.
  `litert/src/tensor_buffer.rs` passes `0` instead of `false`.
- New `litert-sys/tests/layout_abi.rs` asserts the sizes and offsets in the
  table above, so a future regen that disagrees with the vendored header
  fails a test instead of corrupting shapes at runtime. **This is the check
  the "before trusting a regen again" note above asked for — it now exists;
  don't delete it.**

## `LITERT_SYS_BINDGEN_ABI` is still needed

The two passes no longer differ on `LiteRtLayout`, but they still differ on:

- enums without an explicit underlying type (`c_int` under MSVC vs `c_uint`
  under Itanium) — `LiteRtOpCode`, `LiteRtEventType`, `LiteRtEnvOptionTag`,
  `LiteRtCpuKernelMode`, `LiteRtQuantizationTypeId`, …
- trailing flexible-array members (`[T; 1]` vs `__IncompleteArrayField<T>`)
  in `LiteRtMagicNumberConfigs` / `LiteRtMagicNumberVerifications`
- the `LITERT_HAS_*_SUPPORT*` platform constants

So keep generating twice, exactly as documented above.

## Everything else is additive

- New: `LiteRtGetCompiledModelEnvironment`.
- New enum values: `kLiteRtDelegatePrecisionFp16WithFp32Accum` (3),
  `kLiteRtEnvOptionTagContext` (28),
  `kLiteRtEnvOptionTagWebGpuFlushCallback` (29). No renumbering.
- `kLiteRtCpuKernelModeDelegate` is the new spelling of
  `kLiteRtCpuKernelModeXnnpack`; upstream keeps the old name as an alias and
  both are still 0.
- `Lrt{Set,Get}CpuOptionsEnableYNNPack` and
  `Lrt{Set,Get}GpuOptionsMetalResidencySet` were added upstream but do **not**
  appear in our bindings — `wrapper.h`'s allowlist is `LiteRt.*`, which has
  never matched the `Lrt*`-prefixed options functions. Pre-existing, not a
  2.2.0 regression; widen the allowlist if those are ever wanted.
- `LrtSetRuntimeOptionsSelectedSignatures` is `#ifdef __cplusplus`-only, so
  bindgen skips it. `litert_runtime_options.h`'s new `<string>`/`<vector>`
  includes are behind the same guard and add no bindgen dependency.

## Vendoring notes

- File list is byte-for-byte the same 46 paths as `litert-v2.1.6/`; the
  include closure pulls in no new `internal/` header. (2.2.0 adds
  `internal/litert_abi_header.h`, `litert_runtime_api_export.h` and
  `litert_runtime_builtin.h`, none of which `wrapper.h` reaches.)
- `build_common/build_config.h.in` is **unchanged** between 2.1.6 and 2.2.0,
  so the hand-written `build_config.h` was carried over again — the caveat
  at the top of this doc was re-checked and still holds.
- `LICENSE` is unchanged upstream.
- The linker version script only gained `kLiteRtRuntimeBuiltin`; nothing was
  un-exported.

## Still not done

- **No 2.2.0 binaries exist locally yet.** `cam_poc_materials` has
  `lite_rt_bin`, `_2.1.5` and `_2.1.6` only. Everything above was validated
  against 2.1.6 libraries, which is why the shape-carrying Windows tests
  fail (correctly). `cam_poc`'s `Cargo.toml` is deliberately *not* pointed at
  this branch yet.
- `LITERT_MAVEN_VERSION` / `ANDROID_AAR_SHA256` / `ANDROID_AAR_SIZE` still
  pin the 2.1.4 AAR, and `LITERT_LM_TAG` still pins `v0.10.2` desktop
  prebuilts. Both need a network fetch to re-pin.
- Pre-existing and untouched by this branch: `cargo clippy -- -D warnings`
  fails on a `collapsible_if` in `build.rs`'s `cache_root()`, `cargo fmt
  --check` wants to reflow `WASM32_EMSCRIPTEN_TARBALL_URL`, the
  `gpu_options_cache` / `inference_gpu` tests fail on this host for lack of
  `dxil.dll`, and the `GpuOptions` doctest fails to compile. All four
  reproduce identically on the 2.1.6 branch.
