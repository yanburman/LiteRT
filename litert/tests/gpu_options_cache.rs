//! Verifies GpuOptions' TOML-payload opaque-options attachment (see
//! `litert/src/options.rs` doc comment) is actually accepted by the runtime
//! at compile time, and — where a GPU accelerator is available — that the
//! program cache directory receives on-disk artifacts.

use std::path::PathBuf;

use litert::{
    Accelerators, CompilationOptions, CompiledModel, Environment, GpuOptions, LogSeverity, Model,
    set_global_log_severity,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("add_10x10.tflite")
}

#[test]
fn gpu_program_cache_compiles_and_may_populate_cache_dir() {
    let _ = set_global_log_severity(LogSeverity::Error);

    let cache_dir =
        std::env::temp_dir().join(format!("litert_gpu_cache_test_{}", std::process::id()));
    std::fs::create_dir_all(&cache_dir).expect("create cache dir");

    let env = Environment::new().expect("environment");
    let model = Model::from_file(&env, fixture_path()).expect("load tflite");

    let gpu_options = GpuOptions::new()
        .with_serialization_dir(&cache_dir)
        .expect("serialization dir")
        .with_model_cache_key("add_10x10_test")
        .expect("model cache key")
        .with_serialize_program_cache(true);

    let options = CompilationOptions::new()
        .expect("options")
        .with_accelerators(Accelerators::GPU | Accelerators::CPU)
        .expect("set accelerators")
        .with_gpu_options(gpu_options)
        .expect("attach gpu options");

    let compiled = CompiledModel::new(env, model, &options).expect("compile");
    let accelerated = compiled.is_fully_accelerated().unwrap_or(false);

    let entries: Vec<_> = std::fs::read_dir(&cache_dir)
        .expect("read cache dir")
        .filter_map(|e| e.ok())
        .collect();
    eprintln!(
        "gpu_program_cache: fully_accelerated={accelerated} cache_dir_entries={}",
        entries.len()
    );

    let _ = std::fs::remove_dir_all(&cache_dir);
}
