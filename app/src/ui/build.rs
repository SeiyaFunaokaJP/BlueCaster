//! Embeds the app icon (ID 101; built by app/resources/make_icon.py) and
//! the version information: the same product name and version as
//! bluecaster-core.exe. Code signing checks these fields.

include!("../../resources/version_rc.rs");

fn main() {
    embed_resource::compile("resources/ui.rc", embed_resource::NONE).manifest_optional().unwrap();
    let rc = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("version.rc");
    std::fs::write(&rc, version_rc("bluecaster.exe", "BlueCaster")).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().unwrap();
    println!("cargo:rerun-if-changed=resources/ui.rc");
    println!("cargo:rerun-if-changed=../../resources/app.ico");
    println!("cargo:rerun-if-changed=../../resources/version_rc.rs");
}
