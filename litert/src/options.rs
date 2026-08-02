//! Compilation options controlling how a model is compiled for execution.

use std::ffi::CString;
use std::path::Path;
use std::ptr::NonNull;

use litert_sys as sys;

use crate::{check, Result};

/// Hardware accelerator selection, represented as a bitset so a model can be
/// compiled to target multiple backends simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Accelerators(sys::LiteRtHwAcceleratorSet);

impl Accelerators {
    /// No accelerator. The compiled model will fail to run.
    pub const NONE: Self = Self(sys::kLiteRtHwAcceleratorNone as _);
    /// CPU reference backend. Always available.
    pub const CPU: Self = Self(sys::kLiteRtHwAcceleratorCpu as _);
    /// GPU backend (Metal on Apple, WebGPU / OpenCL elsewhere).
    pub const GPU: Self = Self(sys::kLiteRtHwAcceleratorGpu as _);
    /// NPU backend. Available on a narrower set of platforms.
    pub const NPU: Self = Self(sys::kLiteRtHwAcceleratorNpu as _);

    /// Combine with another accelerator bit.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Raw bitset value, for passing to the C API.
    #[must_use]
    pub const fn bits(self) -> sys::LiteRtHwAcceleratorSet {
        self.0
    }

    /// `true` if any of `other`'s bits are set in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0 && other.0 != 0
    }
}

impl std::ops::BitOr for Accelerators {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl Default for Accelerators {
    fn default() -> Self {
        Self::CPU
    }
}

/// Options passed to [`CompiledModel::new`](crate::CompiledModel::new).
pub struct CompilationOptions {
    ptr: NonNull<sys::LiteRtOptionsT>,
}

impl std::fmt::Debug for CompilationOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilationOptions")
            .field("ptr", &self.ptr.as_ptr())
            .finish()
    }
}

impl CompilationOptions {
    /// Creates a new options object with default settings (CPU-only).
    ///
    /// # Errors
    ///
    /// Returns [`Error::NullPointer`](crate::Error::NullPointer) if the C API
    /// refused to allocate, or [`Error::Status`](crate::Error::Status) if the
    /// runtime reported a failure.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use litert::{Accelerators, CompilationOptions};
    ///
    /// let options = CompilationOptions::new()?
    ///     .with_accelerators(Accelerators::GPU | Accelerators::CPU)?;
    /// # Ok::<(), litert::Error>(())
    /// ```
    pub fn new() -> Result<Self> {
        let mut raw: sys::LiteRtOptions = std::ptr::null_mut();
        check(unsafe { sys::LiteRtCreateOptions(&mut raw) })?;
        let ptr = NonNull::new(raw).ok_or(crate::Error::NullPointer)?;
        let mut this = Self { ptr };
        this.set_accelerators(Accelerators::CPU)?;
        Ok(this)
    }

    /// Selects which hardware backends the compiler may use.
    ///
    /// # Errors
    ///
    /// Returns an error if the LiteRT runtime rejects the combination.
    pub fn set_accelerators(&mut self, accelerators: Accelerators) -> Result<()> {
        check(unsafe {
            sys::LiteRtSetOptionsHardwareAccelerators(self.ptr.as_ptr(), accelerators.bits())
        })
    }

    /// Builder-style accelerator setter.
    ///
    /// # Errors
    ///
    /// See [`Self::set_accelerators`].
    pub fn with_accelerators(mut self, accelerators: Accelerators) -> Result<Self> {
        self.set_accelerators(accelerators)?;
        Ok(self)
    }

    /// Attaches GPU-accelerator options (program-cache serialization,
    /// precision, buffer strategy, etc.) built with [`GpuOptions`] to this
    /// compilation options object.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Status`](crate::Error::Status) if the runtime rejects
    /// the opaque-options payload.
    pub fn add_gpu_options(&mut self, gpu_options: GpuOptions) -> Result<()> {
        // `Lrt*` (the GPU options builder C API) is deliberately not part of
        // libLiteRt's exported ABI — see GpuOptions' doc comment. The wire
        // format the GPU delegate actually consumes is a plain TOML string
        // attached under the "gpu_options" opaque-options identifier, built
        // directly here instead of via the unexported builder API.
        let payload = CString::new(gpu_options.toml).map_err(|_| {
            crate::Error::Unsupported("GPU options TOML contains an interior NUL byte")
        })?;
        let payload_ptr = payload.into_raw();

        let mut opaque: sys::LiteRtOpaqueOptions = std::ptr::null_mut();
        check(unsafe {
            sys::LiteRtCreateOpaqueOptions(
                GPU_OPTIONS_IDENTIFIER.as_ptr(),
                payload_ptr.cast(),
                Some(free_cstring_payload),
                &mut opaque,
            )
        })?;
        check(unsafe { sys::LiteRtAddOpaqueOptions(self.ptr.as_ptr(), opaque) })
    }

    /// Builder-style [`Self::add_gpu_options`].
    ///
    /// # Errors
    ///
    /// See [`Self::add_gpu_options`].
    pub fn with_gpu_options(mut self, gpu_options: GpuOptions) -> Result<Self> {
        self.add_gpu_options(gpu_options)?;
        Ok(self)
    }

    pub(crate) fn as_raw(&self) -> sys::LiteRtOptions {
        self.ptr.as_ptr()
    }
}

impl Drop for CompilationOptions {
    fn drop(&mut self) {
        unsafe { sys::LiteRtDestroyOptions(self.ptr.as_ptr()) }
    }
}

// Safety: a LiteRtOptions handle carries no thread-local state and is only
// mutated via exclusive borrow in the safe API.
unsafe impl Send for CompilationOptions {}

/// Opaque-options identifier the GPU delegate looks up at compile time
/// (`LrtGetGpuOptionsIdentifier()` in `litert/c/options/litert_gpu_options.cc`,
/// consumed by `GetGpuOptionsPayload()` in
/// `litert/runtime/accelerators/gpu/ml_drift_delegate_create.cc`).
const GPU_OPTIONS_IDENTIFIER: &std::ffi::CStr = c"gpu_options";

/// `payload_destructor` for a payload previously produced by
/// `CString::into_raw`, matching the signature `LiteRtCreateOpaqueOptions`
/// expects.
unsafe extern "C" fn free_cstring_payload(payload: *mut std::os::raw::c_void) {
    drop(unsafe { CString::from_raw(payload.cast()) });
}

/// GPU-accelerator-specific compilation options: program-cache serialization,
/// cache namespacing, etc.
///
/// Upstream deliberately keeps the GPU-options *builder* C API (`Lrt*` in
/// `litert_gpu_options.h`) out of libLiteRt's exported ABI — see
/// `g3doc/apis/opaque_options_toml.md`: "The `Lrt` prefix indicates that
/// these functions are not part of the core LiteRT C APIs, but belong to a
/// totally independent library. Exported tools should be removed from
/// scripts like `windows_exported_symbols.def`." The actual wire format the
/// GPU delegate consumes is a TOML string attached via the stable, exported
/// opaque-options primitives (`LiteRtCreateOpaqueOptions`/
/// `LiteRtAddOpaqueOptions`) under the `"gpu_options"` identifier — traced
/// directly in `GetGpuOptionsPayload()`
/// (`litert/runtime/accelerators/gpu/ml_drift_delegate_create.cc`), which
/// looks up that identifier and re-parses the payload bytes as TOML via
/// `LrtCreateGpuOptionsFromToml`. This type builds that TOML text directly
/// instead of linking the unexported `Lrt*` builder API.
///
/// # Example
///
/// ```no_run
/// use litert::{CompilationOptions, GpuOptions};
/// use std::path::Path;
///
/// let gpu_options = GpuOptions::new()
///     .with_serialization_dir(Path::new("/data/local/tmp/litert_cache"))?
///     .and_then(|g| g.with_model_cache_key("yolo26n_w8a32"))?;
/// let gpu_options = gpu_options.with_serialize_program_cache(true);
/// let options = CompilationOptions::new()?.with_gpu_options(gpu_options)?;
/// # Ok::<(), litert::Error>(())
/// ```
#[derive(Debug, Default)]
pub struct GpuOptions {
    toml: String,
}

impl GpuOptions {
    /// Creates a new GPU options builder with no fields set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a `key = "value"` TOML line. Rejects characters the
    /// runtime's minimal TOML parser (`litert/core/litert_toml_parser.cc`)
    /// can't round-trip — it only strips one layer of surrounding quotes,
    /// with no escape handling.
    fn set_string_field(&mut self, key: &str, value: &str) -> Result<()> {
        if value.contains(['"', '\n', '\r']) {
            return Err(crate::Error::Unsupported(
                "GPU option string value contains an unsupported character (\", \\n, or \\r)",
            ));
        }
        self.toml.push_str(key);
        self.toml.push_str(" = \"");
        self.toml.push_str(value);
        self.toml.push_str("\"\n");
        Ok(())
    }

    /// Sets the on-disk directory the GPU delegate uses for program-cache
    /// serialization. Should be a private, writable app directory (e.g.
    /// Android's `Context.getCodeCacheDir()`). Whether serialization actually
    /// happens depends on the backend and directory validity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidPath`](crate::Error::InvalidPath) if `dir`
    /// contains non-UTF-8 bytes, or
    /// [`Error::Unsupported`](crate::Error::Unsupported) if it contains a
    /// character the TOML payload can't carry (`"`, `\n`, `\r`).
    pub fn set_serialization_dir(&mut self, dir: &Path) -> Result<()> {
        let s = dir
            .to_str()
            .ok_or_else(|| crate::Error::InvalidPath(dir.to_path_buf()))?;
        self.set_string_field("serialization_dir", s)
    }

    /// Builder-style [`Self::set_serialization_dir`].
    ///
    /// # Errors
    ///
    /// See [`Self::set_serialization_dir`].
    pub fn with_serialization_dir(mut self, dir: &Path) -> Result<Self> {
        self.set_serialization_dir(dir)?;
        Ok(self)
    }

    /// Sets the cache-namespace key: a token unique to a particular model
    /// (graph and constants) so a stale cache from a different model is
    /// never reused.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`](crate::Error::Unsupported) if `key`
    /// contains a character the TOML payload can't carry (`"`, `\n`, `\r`).
    pub fn set_model_cache_key(&mut self, key: &str) -> Result<()> {
        self.set_string_field("model_cache_key", key)
    }

    /// Builder-style [`Self::set_model_cache_key`].
    ///
    /// # Errors
    ///
    /// See [`Self::set_model_cache_key`].
    pub fn with_model_cache_key(mut self, key: &str) -> Result<Self> {
        self.set_model_cache_key(key)?;
        Ok(self)
    }

    /// When `true` (and `serialization_dir`/`model_cache_key` are also set),
    /// the delegate serializes the compiled GPU program so future compiles
    /// against the same cache can skip recompilation.
    pub fn set_serialize_program_cache(&mut self, enable: bool) {
        self.toml.push_str("serialize_program_cache = ");
        self.toml.push_str(if enable { "true" } else { "false" });
        self.toml.push('\n');
    }

    /// Builder-style [`Self::set_serialize_program_cache`].
    #[must_use]
    pub fn with_serialize_program_cache(mut self, enable: bool) -> Self {
        self.set_serialize_program_cache(enable);
        self
    }
}
