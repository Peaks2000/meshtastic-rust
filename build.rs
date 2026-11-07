use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets");
    let resources = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("meshtastic.gresource");
    let status = Command::new(
        env::var_os("GLIB_COMPILE_RESOURCES").unwrap_or_else(|| "glib-compile-resources".into()),
    )
    .arg("--sourcedir=assets")
    .arg(format!("--target={}", resources.display()))
    .arg("assets/resources.gresource.xml")
    .status()
    .expect("Could not run glib-compile-resources (install GTK development tools)");
    assert!(status.success(), "Logo resource compilation failed");
    println!("cargo:rerun-if-changed=vendor/protobufs");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("meshtastic.bin");
    let status = Command::new(protoc_bin_vendored::protoc_bin_path().unwrap())
        .arg("-Ivendor/protobufs")
        .arg(format!(
            "-I{}",
            protoc_bin_vendored::include_path().unwrap().display()
        ))
        .arg(format!("--descriptor_set_out={}", out.display()))
        .args([
            "--include_imports",
            "--include_source_info",
            "meshtastic/admin.proto",
        ])
        .status()
        .expect("Could not run bundled protoc");
    assert!(status.success(), "Meshtastic protocol compilation failed");
}
