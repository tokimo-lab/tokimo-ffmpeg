use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));

    // Search order for FFmpeg install:
    // 1. packages/tokimo-ffmpeg/install/ (own build)
    // 2. Workspace root bin/ffmpeg/current/ (main project's FFmpeg build)
    let candidates = [
        manifest_dir.join("install"),
        manifest_dir.join("../../bin/ffmpeg/current"),
    ];

    // Always watch build.rs itself — this is the baseline rerun trigger.
    println!("cargo:rerun-if-changed=build.rs");

    for install_dir in &candidates {
        let lib_dir = install_dir.join("lib");
        if lib_dir.exists() {
            let lib_dir = lib_dir.canonicalize().unwrap_or(lib_dir.clone());
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());

            // Watch lib dir + every libav* version.h so we re-run if FFmpeg is rebuilt.
            // This ensures rusty_ffmpeg's bindgen regenerates FFI bindings when any
            // library's ABI changes (e.g. after `make ffmpeg`), preventing SIGSEGV.
            // Only emit rerun-if-changed for paths that actually exist — Cargo re-runs
            // the build script on every build if a watched path does not exist.
            println!("cargo:rerun-if-changed={}", lib_dir.display());
            let include_dir_watch = install_dir.join("include");
            if include_dir_watch.exists() {
                let include_dir_watch = include_dir_watch.canonicalize().unwrap_or(include_dir_watch);
                // Watch version.h for every libav* library — these change on every
                // FFmpeg rebuild and drive the ABI versioning for all bindings.
                for lib in &[
                    "libavcodec",
                    "libavdevice",
                    "libavfilter",
                    "libavformat",
                    "libavutil",
                    "libpostproc",
                    "libswresample",
                    "libswscale",
                ] {
                    let version_header = include_dir_watch.join(lib).join("version.h");
                    if version_header.exists() {
                        println!("cargo:rerun-if-changed={}", version_header.display());
                    }
                }
            }

            if std::env::var("FFMPEG_DYN_DIR").is_err() {
                println!("cargo:rustc-env=FFMPEG_DYN_DIR={}", lib_dir.display());
            }
            let include_dir = install_dir.join("include");
            if include_dir.exists() && std::env::var("FFMPEG_INCLUDE_DIR").is_err() {
                let include_dir = include_dir.canonicalize().unwrap_or(include_dir);
                println!("cargo:rustc-env=FFMPEG_INCLUDE_DIR={}", include_dir.display());
            }
            let pkgconfig_dir = lib_dir.join("pkgconfig");
            if pkgconfig_dir.exists() && std::env::var("FFMPEG_PKG_CONFIG_PATH").is_err() {
                let pkgconfig_dir = pkgconfig_dir.canonicalize().unwrap_or(pkgconfig_dir);
                println!("cargo:rustc-env=FFMPEG_PKG_CONFIG_PATH={}", pkgconfig_dir.display());
            }

            break;
        }
    }
}
