//! The updater's public key, built into the program (update.rs): from
//! BLUECASTER_UPDATER_PUBKEY, the .pub file's path or its content, as
//! installer/build-release.ps1 sets it for a signed release. Without it the
//! in-app update stays off ("unsigned").

fn main() {
    const VAR: &str = "BLUECASTER_UPDATER_PUBKEY";
    println!("cargo:rerun-if-env-changed={VAR}");
    let value = std::env::var(VAR).unwrap_or_default();
    let value = value.trim();
    let path = std::path::Path::new(value);
    let key = if !value.is_empty() && path.is_file() {
        println!("cargo:rerun-if-changed={value}");
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{VAR}: cannot read the key file: {e}"))
    } else {
        value.to_string()
    };
    let key = key.trim();
    // Tauri's key files hold the minisign key base64-wrapped on one line;
    // a line break would also end the variable below
    if key.contains(['\r', '\n']) {
        panic!("{VAR}: not a Tauri updater public key (one base64 line)");
    }
    println!("cargo:rustc-env=BLUECASTER_UPDATER_PUBKEY_BUILT={key}");
}
