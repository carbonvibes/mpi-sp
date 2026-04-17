use std::process::Command;

fn main() {
    // Build the C control plane (vfs + delta + control_plane → libcontrol_plane.a).
    // The Makefile is idempotent; if nothing changed it is a no-op.
    let status = Command::new("make")
        .arg("libcontrol_plane.a")
        .current_dir("../control_plane")
        .status()
        .expect("failed to invoke make in ../control_plane — is make installed?");

    assert!(
        status.success(),
        "control_plane make failed; run `make` in control_plane/ to see the error"
    );

    // Tell Cargo where to find the archive and what to link.
    println!("cargo:rustc-link-search=native=../control_plane");
    println!("cargo:rustc-link-lib=static=control_plane");

    // Rebuild automatically when C sources change.
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
}
