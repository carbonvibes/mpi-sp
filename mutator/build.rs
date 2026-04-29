use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_libarchive)");
    println!("cargo::rustc-check-cfg=cfg(has_fuse3)");

    // ── Control plane (VFS + delta + control_plane → libcontrol_plane.a) ────
    let status = Command::new("make")
        .arg("libcontrol_plane.a")
        .current_dir("../control_plane")
        .status()
        .expect("failed to invoke make in ../control_plane — is make installed?");

    assert!(
        status.success(),
        "control_plane make failed; run `make` in control_plane/ to see the error"
    );

    println!("cargo:rustc-link-search=native=../control_plane");
    println!("cargo:rustc-link-lib=static=control_plane");

    for path in &[
        "../vfs/vfs.c",
        "../vfs/vfs.h",
        "../control_plane/delta.c",
        "../control_plane/delta.h",
        "../control_plane/control_plane.c",
        "../control_plane/control_plane.h",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    // ── foobar demo target ────────────────────────────────────────────────────
    // Must use clang: GCC does not support -fsanitize-coverage=trace-pc-guard.
    // SanCov callbacks (__sanitizer_cov_trace_pc_guard*) are provided by
    // libafl_targets at link time — no separate ASan runtime needed.
    cc::Build::new()
        .compiler("clang")
        .file("../demo/foobar_target.c")
        .flag("-fsanitize-coverage=trace-pc-guard,trace-cmp")
        .opt_level(0)
        .compile("foobar_target");

    println!("cargo:rerun-if-changed=../demo/foobar_target.c");

    // ── libarchive harness (optional) ─────────────────────────────────────────
    // Probe for libarchive-dev; skip gracefully if absent.
    // The fuzz_libafl binary guards the libarchive campaign behind a runtime
    // check, so the binary still builds without libarchive installed.
    let has_archive = Command::new("pkg-config")
        .args(["--exists", "libarchive"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if has_archive {
        // Prefer the SanCov-instrumented static build over the system .so.
        // Build it once with: bash scripts/build_libarchive_sancov.sh
        let sancov_dir = std::path::PathBuf::from("../vendor/libarchive-sancov");
        let sancov_lib = sancov_dir.join("lib/libarchive.a");

        let mut build = cc::Build::new();
        build.compiler("clang")
             .file("../demo/libarchive_harness.c")
             .flag("-fsanitize-coverage=trace-pc-guard,trace-cmp")
             .opt_level(1);

        if sancov_lib.exists() {
            // Static SanCov build: edges inside libarchive's parsers are visible
            // to HitcountsMapObserver.  Link the static archive plus its deps.
            build.include(sancov_dir.join("include"));
            build.compile("libarchive_harness");

            println!(
                "cargo:rustc-link-search=native={}",
                sancov_dir.join("lib").display()
            );
            println!("cargo:rustc-link-lib=static=archive");
            // Transitive deps that libarchive.a requires at link time
            println!("cargo:rustc-link-lib=z");
            println!("cargo:rustc-link-lib=bz2");
            println!("cargo:rustc-link-lib=lzma");
            println!("cargo:rustc-link-lib=lz4");
            println!("cargo:rustc-link-lib=zstd");
            println!("cargo:rustc-link-lib=acl");
            println!("cargo:warning=libarchive: using SanCov-instrumented static build");
        } else {
            // System .so fallback: only the 3 edges in the harness wrapper are
            // visible — corpus will stall.  Run scripts/build_libarchive_sancov.sh
            // once to get real coverage inside libarchive's format parsers.
            build.compile("libarchive_harness");
            println!("cargo:rustc-link-lib=archive");
            println!("cargo:warning=libarchive: using system .so (no SanCov — run scripts/build_libarchive_sancov.sh for real coverage)");
        }

        println!("cargo:rustc-cfg=has_libarchive");
    } else {
        println!("cargo:warning=libarchive-dev not found — libarchive campaign disabled");
        println!("cargo:warning=Install with: apt install libarchive-dev");
    }

    println!("cargo:rerun-if-changed=../demo/libarchive_harness.c");

    // ── fuse_vfs library (FUSE mount for the harness loop) ───────────────────
    // Compiled with -DFUSE_VFS_LIBRARY so the standalone main() is excluded.
    // Provides: fuse_vfs_lib_init, fuse_vfs_lib_run, fuse_vfs_lib_is_mounted,
    //           fuse_vfs_lib_stop.
    let has_fuse3 = Command::new("pkg-config")
        .args(["--exists", "fuse3"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if has_fuse3 {
        let fuse_cflags = Command::new("pkg-config")
            .args(["--cflags", "fuse3"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let fuse_libs = Command::new("pkg-config")
            .args(["--libs", "fuse3"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let mut build = cc::Build::new();
        build.compiler("clang")
             .file("../fuse_vfs/fuse_vfs.c")
             .define("FUSE_VFS_LIBRARY", None)
             .define("FUSE_USE_VERSION", "31")
             // fuse_vfs.c is harness infrastructure, NOT a fuzzing target.
             // Its callbacks (fvfs_getattr, fvfs_open, fvfs_read, fvfs_readdir)
             // execute the same code path on every single iteration — their SanCov
             // slots saturate after iteration 1 and never produce new signal.
             // Instrumenting harness code wastes map slots and dilutes the feedback
             // signal from the actual targets (foobar_target.c, libarchive, runc).
             .opt_level(1);

        // Pass -I/usr/include/fuse3 and any other cflags from pkg-config.
        for flag in fuse_cflags.split_whitespace() {
            build.flag(flag);
        }

        build.compile("fuse_vfs_lib");

        // Link fuse3 (and pthread, which it needs).
        for token in fuse_libs.split_whitespace() {
            if let Some(lib) = token.strip_prefix("-l") {
                println!("cargo:rustc-link-lib={lib}");
            } else if let Some(path) = token.strip_prefix("-L") {
                println!("cargo:rustc-link-search=native={path}");
            }
        }

        println!("cargo:rustc-cfg=has_fuse3");
        println!("cargo:rerun-if-changed=../fuse_vfs/fuse_vfs.c");
    } else {
        println!("cargo:warning=libfuse3-dev not found — FUSE harness loop disabled");
        println!("cargo:warning=Install with: apt install libfuse3-dev");
    }
}
