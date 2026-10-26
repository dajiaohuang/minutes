//! Make the plugin able to find its own sherpa libraries wherever it is put.
//!
//! On Linux, sherpa-rs links `libsherpa-onnx-c-api.so` and `libonnxruntime.so`
//! dynamically, and those land in this crate's `target/` directory. Without a
//! runpath the plugin only resolves them while it sits there, so copying it
//! anywhere else, which is exactly what installing and packaging both do,
//! produces a library that fails to load with a missing-dependency error.
//!
//! `$ORIGIN` points the loader at the plugin's own directory rather than the
//! build tree, so the plugin works anywhere its sibling libraries travel with
//! it. That is the shape the release archive already ships and the shape
//! docs/architecture/sherpa-engine.md tells people to install by hand.
//!
//! macOS does the same with `@loader_path`, and additionally fetches the
//! pinned sherpa-onnx archive itself (see `macos` below). Windows resolves a
//! DLL's dependencies from the loading process's search path rather than the
//! DLL's own directory, so it needs different handling entirely; that is part
//! of #645.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN"),
        #[cfg(target_os = "macos")]
        Ok("macos") if std::env::var_os("CARGO_FEATURE_LEGACY_STATIC").is_none() => {
            macos::link_sherpa()
        }
        _ => {}
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};

    const VERSION: &str = "v1.13.8";
    const ARCHIVE: &str = "sherpa-onnx-v1.13.8-osx-universal2-shared";
    // From the release's published asset digest. A mismatch aborts the build.
    const SHA256: &str = "2249f97f10df7d828af1b0d10e3d1eeaa2198b60b9a999a7ed853c150181ccfd";
    const DYLIBS: [&str; 2] = ["libsherpa-onnx-c-api.dylib", "libonnxruntime.dylib"];

    /// Download and verify the archive, generate bindings from its headers, and
    /// link the shared libraries with a runpath that finds them beside the plugin.
    ///
    /// `MINUTES_SHERPA_ARCHIVE` points at an already downloaded `.tar.bz2` for
    /// offline builds; it is held to the same checksum.
    pub fn link_sherpa() {
        println!("cargo:rerun-if-env-changed=MINUTES_SHERPA_ARCHIVE");
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let root = out_dir.join(ARCHIVE);
        if !root.join("lib").join(DYLIBS[0]).exists() {
            let bytes = fetch_archive();
            let digest = format!("{:x}", Sha256::digest(&bytes));
            assert_eq!(
                digest, SHA256,
                "sherpa-onnx archive checksum mismatch; refusing to build against it"
            );
            let decoder = bzip2::read::BzDecoder::new(bytes.as_slice());
            tar::Archive::new(decoder)
                .unpack(&out_dir)
                .expect("unpack sherpa-onnx archive");
        }

        let lib_dir = root.join("lib");
        let header = root.join("include/sherpa-onnx/c-api/c-api.h");
        println!("cargo:rerun-if-changed={}", header.display());
        bindgen::Builder::default()
            .header(header.to_string_lossy())
            .allowlist_function("SherpaOnnx.*")
            .allowlist_type("SherpaOnnx.*")
            .layout_tests(false)
            .generate()
            .expect("generate sherpa-onnx bindings")
            .write_to_file(out_dir.join("sherpa_bindings.rs"))
            .expect("write sherpa-onnx bindings");

        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
        copy_beside_plugin(&out_dir, &lib_dir);
    }

    fn fetch_archive() -> Vec<u8> {
        if let Ok(path) = std::env::var("MINUTES_SHERPA_ARCHIVE") {
            return std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        }
        let url = format!(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/{VERSION}/{ARCHIVE}.tar.bz2"
        );
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut ureq::get(&url)
                .call()
                .unwrap_or_else(|e| panic!("download {url}: {e}"))
                .into_reader(),
            &mut bytes,
        )
        .expect("read sherpa-onnx download");
        bytes
    }

    /// `target/<profile>/` is where cargo writes the plugin, and the runpath is
    /// relative to it, so a plugin built in place loads without an install step.
    fn copy_beside_plugin(out_dir: &Path, lib_dir: &Path) {
        let Some(profile_dir) = out_dir.ancestors().nth(3) else {
            return;
        };
        for name in DYLIBS {
            std::fs::copy(lib_dir.join(name), profile_dir.join(name))
                .unwrap_or_else(|error| panic!("stage required {name} beside plugin: {error}"));
        }
    }
}
