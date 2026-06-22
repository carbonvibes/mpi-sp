use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_libarchive)");
    println!("cargo::rustc-check-cfg=cfg(has_fuse3)");

    // control plane → libcontrol_plane.a
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

    // foobar demo target. clang only: GCC lacks -fsanitize-coverage=trace-pc-guard.
    // SanCov callbacks come from libafl_targets at link time.
    cc::Build::new()
        .compiler("clang")
        .file("../demo/foobar_target.c")
        .flag("-fsanitize-coverage=trace-pc-guard,trace-cmp")
        .opt_level(0)
        .compile("foobar_target");

    println!("cargo:rerun-if-changed=../demo/foobar_target.c");

    // libarchive harness (optional). fuzz_libafl runtime-guards the campaign,
    // so it still builds without libarchive.
    let has_archive = Command::new("pkg-config")
        .args(["--exists", "libarchive"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if has_archive {
        // Prefer SanCov static build over system .so.
        // Build once with: bash scripts/build_libarchive_sancov.sh
        let sancov_dir = std::path::PathBuf::from("../vendor/libarchive-sancov");
        let sancov_lib = sancov_dir.join("lib/libarchive.a");

        let mut build = cc::Build::new();
        build
            .compiler("clang")
            .file("../demo/libarchive_harness.c")
            .flag("-fsanitize-coverage=trace-pc-guard,trace-cmp")
            .opt_level(1);

        if sancov_lib.exists() {
            // edges inside libarchive's parsers are visible. link archive + deps.
            build.include(sancov_dir.join("include"));
            build.compile("libarchive_harness");

            println!(
                "cargo:rustc-link-search=native={}",
                sancov_dir.join("lib").display()
            );
            println!("cargo:rustc-link-lib=static=archive");
            // transitive deps for libarchive.a
            println!("cargo:rustc-link-lib=z");
            println!("cargo:rustc-link-lib=bz2");
            println!("cargo:rustc-link-lib=lzma");
            println!("cargo:rustc-link-lib=lz4");
            println!("cargo:rustc-link-lib=zstd");
            println!("cargo:rustc-link-lib=acl");
            println!("cargo:warning=libarchive: using SanCov-instrumented static build");
        } else {
            // system .so fallback: only harness-wrapper edges visible, corpus stalls.
            // run scripts/build_libarchive_sancov.sh for real parser coverage.
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

    // fuse_vfs library. -DFUSE_VFS_LIBRARY excludes the standalone main().
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
        build
            .compiler("clang")
            .file("../fuse_vfs/fuse_vfs.c")
            .define("FUSE_VFS_LIBRARY", None)
            .define("FUSE_USE_VERSION", "31")
            // harness code, not a target: no SanCov or it dilutes the targets' signal.
            .opt_level(1);

        for flag in fuse_cflags.split_whitespace() {
            build.flag(flag);
        }

        build.compile("fuse_vfs_lib");

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

    // libcrun (SanCov, in-process crun fuzzing). Build crun once with:
    //   cd vendor/crun && ./autogen.sh
    //   CC=clang CFLAGS="-fsanitize-coverage=trace-pc-guard,trace-cmp -O1 -g" \
    //     ./configure --disable-shared --enable-static && make -j$(nproc)
    //
    // Only compile crun_harness.c and set link-search paths here. The actual
    // link-lib directives are #[link] attrs in fuzz_crun.rs so they stay
    // per-binary — package-wide would pull libcrun.a (needs SanCov symbols from
    // libafl_targets) into vfs_bench/fuzz and break their link.
    println!("cargo::rustc-check-cfg=cfg(has_libcrun)");
    println!("cargo::rustc-check-cfg=cfg(has_bundled_yajl)");

    let crun_dir = std::path::PathBuf::from("../vendor/crun");
    let libcrun_a = crun_dir.join(".libs/libcrun.a");

    if libcrun_a.exists() {
        // crun_harness.c is a thin FFI wrapper, not the target: no SanCov.
        cc::Build::new()
            .compiler("clang")
            .file("../demo/crun_harness.c")
            .opt_level(1)
            .include(&crun_dir)
            .include(crun_dir.join("src"))
            .include(crun_dir.join("libocispec/src"))
            .compile("crun_harness");

        // search paths for fuzz_crun.rs's #[link] attrs
        println!(
            "cargo:rustc-link-search=native={}/.libs",
            crun_dir.display()
        );

        let libocispec = crun_dir.join("libocispec/.libs/libocispec.a");
        if libocispec.exists() {
            println!(
                "cargo:rustc-link-search=native={}",
                crun_dir.join("libocispec/.libs").display()
            );
        }

        let libyajl = crun_dir.join("libocispec/yajl/.libs/libyajl.a");
        if libyajl.exists() {
            println!(
                "cargo:rustc-link-search=native={}",
                crun_dir.join("libocispec/yajl/.libs").display()
            );
            println!("cargo:rustc-cfg=has_bundled_yajl");
        }

        println!("cargo:rustc-cfg=has_libcrun");
        println!("cargo:warning=libcrun: using SanCov-instrumented static build from vendor/crun");
        println!("cargo:rerun-if-changed=../demo/crun_harness.c");
        println!("cargo:rerun-if-changed=../vendor/crun/.libs/libcrun.a");
    } else {
        println!("cargo:warning=libcrun not found — crun in-process campaign disabled");
        println!("cargo:warning=Build with: cd vendor/crun && ./autogen.sh && CC=clang CFLAGS=\"-fsanitize-coverage=trace-pc-guard,trace-cmp -O1 -g\" ./configure --disable-shared --enable-static && make -j$(nproc)");
    }
}
