//! Generates bindings for the part of libobs this plugin calls.
//!
//! On Linux libobs comes from pkg-config. For Windows, which has no pkg-config
//! for OBS, `OBS_SRC` is an OBS source tree of the version the plugin will load
//! into, for the headers, and `OBS_BIN` is that release's `bin/64bit`, whose
//! `obs.dll` and `obs-frontend-api.dll` GNU ld links against directly.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let includes = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows(&out)
    } else {
        linux()
    };
    let mut b = bindgen::Builder::default().header("wrapper.h");
    for dir in &includes {
        b = b.clang_arg(format!("-I{}", dir.display()));
    }
    let bindings = b
        .allowlist_function("obs_register_output_s")
        .allowlist_function("obs_output_.*")
        .allowlist_function("obs_get_video_info")
        .allowlist_function("obs_queue_task")
        .allowlist_function("obs_frontend_(add|remove)_event_callback")
        .allowlist_function("blog")
        .allowlist_function("os_gettime_ns")
        .allowlist_type("obs_output_info")
        .allowlist_type("video_data")
        .allowlist_type("video_scale_info")
        .allowlist_type("obs_frontend_event")
        .allowlist_var("OBS_OUTPUT_.*")
        .allowlist_var("LOG_.*")
        .allowlist_var("LIBOBS_API_.*")
        .allowlist_var("MAX_AV_PLANES")
        .default_enum_style(bindgen::EnumVariation::Consts)
        .derive_default(true)
        .layout_tests(false)
        .generate()
        .expect("bindgen on libobs headers");
    bindings.write_to_file(out.join("obs.rs")).expect("write bindings");
}

fn linux() -> Vec<PathBuf> {
    let obs = pkg_config::Config::new().probe("libobs").expect("libobs from pkg-config");
    // libobs.pc gives <prefix>/include; the headers include each other as <obs.h>.
    let mut includes = Vec::new();
    for dir in &obs.include_paths {
        includes.push(dir.clone());
        includes.push(dir.join("obs"));
    }
    // libobs-frontend-api has no .pc file; it sits next to libobs.
    for dir in &obs.link_paths {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    println!("cargo:rustc-link-lib=dylib=obs-frontend-api");
    includes
}

fn windows(out: &std::path::Path) -> Vec<PathBuf> {
    println!("cargo:rerun-if-env-changed=OBS_SRC");
    println!("cargo:rerun-if-env-changed=OBS_BIN");
    let src = PathBuf::from(std::env::var("OBS_SRC").expect("OBS_SRC: an OBS source tree"));
    let bin = PathBuf::from(std::env::var("OBS_BIN").expect("OBS_BIN: OBS's bin/64bit"));
    // CMake generates obsconfig.h; the paths in it are for OBS's own build.
    std::fs::write(out.join("obsconfig.h"), "#pragma once\n#define OBS_RELEASE_CANDIDATE 0\n#define OBS_BETA 0\n")
        .expect("write obsconfig.h");
    println!("cargo:rustc-link-search=native={}", bin.display());
    println!("cargo:rustc-link-lib=dylib=obs");
    println!("cargo:rustc-link-lib=dylib=obs-frontend-api");
    vec![out.to_path_buf(), src.join("libobs"), src.join("frontend/api")]
}
