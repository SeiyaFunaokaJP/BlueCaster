//! Signs BlueCaster's installer for the in-app update, as `tauri signer sign
//! --app-version <version> <file>` did, and checks such a signature as
//! bluecaster.exe does (installer/build-release.ps1, installer/README.md).
//!
//! The formats are the Tauri CLI's (tauri-cli 2.12, helpers/updater_signature.rs),
//! so the keys made by `tauri signer generate` keep working, and the copies
//! still on the Tauri window (3.0.4 and older, tauri-plugin-updater) accept
//! what this tool signs:
//!
//! - the key files hold a minisign key box, base64-wrapped on one line: the
//!   secret key encrypted with its password (scrypt), the public key in
//!   `<key>.pub`;
//! - the signature is minisign's (prehashed, BLAKE2b-512 of the file), with
//!   the untrusted comment "signature from tauri secret key" and the trusted
//!   comment `timestamp:<unix seconds>\tfile:<file name>\tversion:<version>`;
//! - `<file>.sig` holds the signature box, base64-wrapped, without a line
//!   break.
//!
//! The environment variables are the Tauri CLI's: TAURI_SIGNING_PRIVATE_KEY
//! (the key's content; here also its path), TAURI_SIGNING_PRIVATE_KEY_PATH
//! and TAURI_SIGNING_PRIVATE_KEY_PASSWORD.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

const KEY_VAR: &str = "TAURI_SIGNING_PRIVATE_KEY";
const KEY_PATH_VAR: &str = "TAURI_SIGNING_PRIVATE_KEY_PATH";
const PASSWORD_VAR: &str = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD";
/// The key bluecaster.exe is built with (app/src/crates/ui-host/build.rs)
const PUBKEY_VAR: &str = "BLUECASTER_UPDATER_PUBKEY";
/// The Tauri CLI's untrusted comment
const UNTRUSTED_COMMENT: &str = "signature from tauri secret key";

const USAGE: &str = "\
Usage:
  sign-update sign --app-version <version> <file>
      Writes <file>.sig. The key comes from TAURI_SIGNING_PRIVATE_KEY (content or path)
      or TAURI_SIGNING_PRIVATE_KEY_PATH, its password from TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
  sign-update verify --app-version <version> [--pubkey <path or content>] <file> [<sig file>]
      Checks <file> as bluecaster.exe checks an update. The public key defaults to
      BLUECASTER_UPDATER_PUBKEY, the signature to <file>.sig.
  sign-update generate --write-keys <path> [--force]
      Makes a new key pair: <path> (secret) and <path>.pub. The password comes from
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD, or is asked for.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sign-update: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The command line: positional arguments and `--name value` options
struct Args {
    positional: Vec<String>,
    options: Vec<(String, Option<String>)>,
}

/// Options that take no value
const FLAGS: &[&str] = &["--force"];

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let mut positional = Vec::new();
        let mut options = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if let Some(name) = arg.strip_prefix("--") {
                if let Some((name, value)) = name.split_once('=') {
                    options.push((format!("--{name}"), Some(value.to_string())));
                } else if FLAGS.contains(&arg.as_str()) {
                    options.push((arg.clone(), None));
                } else {
                    let value = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
                    options.push((arg.clone(), Some(value.clone())));
                }
            } else {
                positional.push(arg.clone());
            }
        }
        Ok(Args { positional, options })
    }

    fn take(&mut self, name: &str) -> Option<String> {
        let i = self.options.iter().position(|(n, _)| n == name)?;
        self.options.remove(i).1
    }

    fn flag(&mut self, name: &str) -> bool {
        let i = self.options.iter().position(|(n, _)| n == name);
        i.map(|i| self.options.remove(i)).is_some()
    }

    fn finish(self, max_positional: usize) -> Result<Vec<String>, String> {
        if let Some((name, _)) = self.options.first() {
            return Err(format!("unknown option {name}\n\n{USAGE}"));
        }
        if self.positional.len() > max_positional {
            return Err(format!("too many arguments\n\n{USAGE}"));
        }
        Ok(self.positional)
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(USAGE.into());
    };
    let mut args = Args::parse(rest)?;
    match command.as_str() {
        "sign" => {
            let version =
                args.take("--app-version").ok_or("sign needs --app-version (the app accepts only signed versions)")?;
            let files = args.finish(1)?;
            let file = PathBuf::from(files.first().ok_or_else(|| format!("sign needs a file\n\n{USAGE}"))?);
            let key = private_key_from_env()?;
            let password = password_from_env();
            let secret = secret_key(&key, password)?;
            let (sig_path, encoded) = sign_file(&secret, &file, &version)?;
            println!("{}", sig_path.display());
            println!("{encoded}");
            Ok(())
        }
        "verify" => {
            let version = args.take("--app-version").ok_or("verify needs --app-version")?;
            let pubkey = match args.take("--pubkey") {
                Some(k) => k,
                None => std::env::var(PUBKEY_VAR).map_err(|_| format!("no --pubkey and no {PUBKEY_VAR}"))?,
            };
            let files = args.finish(2)?;
            let file = PathBuf::from(files.first().ok_or_else(|| format!("verify needs a file\n\n{USAGE}"))?);
            let sig_path = files.get(1).map(PathBuf::from).unwrap_or_else(|| signature_path(&file));
            let pubkey = read_value_or_file(&pubkey)?;
            let signature = std::fs::read_to_string(&sig_path).map_err(|e| format!("{}: {e}", sig_path.display()))?;
            let data = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            verify(&data, &signature, &pubkey, &version)?;
            println!("{}: valid, signed for version {version}", file.display());
            Ok(())
        }
        "generate" => {
            let path =
                args.take("--write-keys").ok_or_else(|| format!("generate needs --write-keys <path>\n\n{USAGE}"))?;
            let force = args.flag("--force");
            args.finish(0)?;
            let password = std::env::var(PASSWORD_VAR).ok();
            let (sk, pk) = generate_key(password)?;
            let (sk_path, pk_path) = save_keypair(Path::new(&path), &sk, &pk, force)?;
            println!("Secret key: {} (keep it secret, with its password)", sk_path.display());
            println!("Public key: {}", pk_path.display());
            Ok(())
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

/// A value that may name a file holding it (the variables of
/// build-release.ps1 take either), trimmed
fn read_value_or_file(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let path = Path::new(trimmed);
    // A base64 key is no file name: only an existing file is read
    if !trimmed.is_empty() && path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(text.trim().to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// The private key (base64-wrapped key box) as the Tauri CLI takes it:
/// TAURI_SIGNING_PRIVATE_KEY, or the file TAURI_SIGNING_PRIVATE_KEY_PATH
fn private_key_from_env() -> Result<String, String> {
    if let Some(key) = std::env::var(KEY_VAR).ok().filter(|k| !k.trim().is_empty()) {
        return read_value_or_file(&key);
    }
    if let Some(path) = std::env::var_os(KEY_PATH_VAR) {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{KEY_PATH_VAR}: {e}"))?;
        return Ok(text.trim().to_string());
    }
    Err(format!("no private key: set {KEY_VAR} (or {KEY_PATH_VAR})"))
}

/// As the Tauri CLI: an unset password is asked for at a terminal, and
/// taken as empty otherwise
fn password_from_env() -> Option<String> {
    match std::env::var(PASSWORD_VAR) {
        Ok(p) => Some(p),
        Err(_) if std::io::stdin().is_terminal() => None,
        Err(_) => Some(String::new()),
    }
}

fn decode_base64(text: &str, what: &str) -> Result<String, String> {
    let bytes = STANDARD.decode(text.trim()).map_err(|e| format!("{what}: not base64: {e}"))?;
    String::from_utf8(bytes).map_err(|_| format!("{what}: not UTF-8 inside the base64"))
}

/// The secret key from Tauri's key file content and its password (None:
/// minisign asks for it)
fn secret_key(private_key: &str, password: Option<String>) -> Result<minisign::SecretKey, String> {
    let decoded = decode_base64(private_key, "the private key")?;
    let sk_box = minisign::SecretKeyBox::from_string(&decoded).map_err(|e| format!("the private key: {e}"))?;
    sk_box.into_secret_key(password).map_err(|e| format!("the private key (wrong password?): {e}"))
}

/// Where the Tauri CLI writes the signature: `.sig` appended to the
/// extension (`setup.exe.sig`), or as the extension
fn signature_path(file: &Path) -> PathBuf {
    match file.extension() {
        Some(ext) => {
            let mut ext = ext.to_os_string();
            ext.push(".sig");
            file.with_extension(ext)
        }
        None => file.with_extension("sig"),
    }
}

/// The trusted comment, as the Tauri CLI writes it with `--app-version`.
/// It is one line of tab-separated fields: a tab or a line break in a value
/// would make another field.
fn trusted_comment(timestamp: u64, file_name: &str, version: &str) -> Result<String, String> {
    if file_name.contains(['\t', '\r', '\n']) {
        return Err(format!("the file name {file_name:?} contains a tab or a line break"));
    }
    if version.is_empty() || version.contains(['\t', '\r', '\n']) {
        return Err(format!("the version {version:?} is empty or contains a tab or a line break"));
    }
    Ok(format!("timestamp:{timestamp}\tfile:{file_name}\tversion:{version}"))
}

/// Signs `data` (named `file_name`) for `version`: Tauri's signature, the
/// base64-wrapped signature box
fn sign_bytes(
    secret: &minisign::SecretKey,
    data: impl std::io::Read,
    file_name: &str,
    version: &str,
    timestamp: u64,
) -> Result<String, String> {
    let comment = trusted_comment(timestamp, file_name, version)?;
    let signature = minisign::sign(None, secret, data, Some(&comment), Some(UNTRUSTED_COMMENT))
        .map_err(|e| format!("signing failed: {e}"))?;
    Ok(STANDARD.encode(signature.to_string()))
}

/// Signs the file and writes `<file>.sig` -> (its path, its content)
fn sign_file(secret: &minisign::SecretKey, file: &Path, version: &str) -> Result<(PathBuf, String), String> {
    let file_name = file
        .file_name()
        .ok_or_else(|| format!("{} is not a file path", file.display()))?
        .to_string_lossy()
        .into_owned();
    let data = std::fs::File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "the system clock is before 1970")?.as_secs();
    let encoded = sign_bytes(secret, std::io::BufReader::new(data), &file_name, version, timestamp)?;
    let sig_path = signature_path(file);
    std::fs::write(&sig_path, encoded.as_bytes()).map_err(|e| format!("{}: {e}", sig_path.display()))?;
    Ok((sig_path, encoded))
}

/// bluecaster.exe's own check of a downloaded installer
/// (ui_host::update::verify_signature, the signed version required)
fn verify(data: &[u8], signature: &str, pubkey: &str, version: &str) -> Result<(), String> {
    ui_host::update::verify_signature(data, signature.trim(), pubkey.trim(), version, true)
}

/// A new key pair as `tauri signer generate` makes it -> (secret, public),
/// both base64-wrapped key boxes. An empty password leaves the secret key
/// unencrypted; None asks for one.
fn generate_key(password: Option<String>) -> Result<(String, String), String> {
    let minisign::KeyPair { pk, sk } =
        minisign::KeyPair::generate_encrypted_keypair(password).map_err(|e| format!("key generation failed: {e}"))?;
    let pk_box = pk.to_box().map_err(|e| e.to_string())?.to_string();
    let sk_box = sk.to_box(None).map_err(|e| e.to_string())?.to_string();
    Ok((STANDARD.encode(sk_box), STANDARD.encode(pk_box)))
}

/// Writes the key pair as `tauri signer generate -w <path>` does: `<path>`
/// and `<path>.pub`, without a line break; never over an existing key
/// unless `force`
fn save_keypair(sk_path: &Path, sk: &str, pk: &str, force: bool) -> Result<(PathBuf, PathBuf), String> {
    let pk_path = PathBuf::from(format!("{}.pub", sk_path.display()));
    if !force {
        for path in [sk_path, pk_path.as_path()] {
            if path.exists() {
                return Err(format!("{} exists already (--force replaces it)", path.display()));
            }
        }
    }
    if let Some(parent) = sk_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(sk_path, sk).map_err(|e| format!("{}: {e}", sk_path.display()))?;
    std::fs::write(&pk_path, pk).map_err(|e| format!("{}: {e}", pk_path.display()))?;
    Ok((sk_path.to_path_buf(), pk_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Tauri CLI's own test key (tauri-cli 2.12,
    /// helpers/updater_signature.rs), encrypted with an empty password
    const TAURI_TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V5ClJXUlRZMEl5dkpDN09RZm5GeVAzc2RuYlNzWVVJelJRQnNIV2JUcGVXZUplWXZXYXpqUUFBQkFBQUFBQUFBQUFBQUlBQUFBQTZrN2RnWGh5dURxSzZiL1ZQSDdNcktiaHRxczQwMXdQelRHbjRNcGVlY1BLMTBxR2dpa3I3dDE1UTVDRDE4MXR4WlQwa1BQaXdxKy9UU2J2QmVSNXhOQWFDeG1GSVllbUNpTGJQRkhhTnROR3I5RmdUZi90OGtvaGhJS1ZTcjdZU0NyYzhQWlQ5cGM9Cg==";

    // ui-host's fixed vector (tauri-plugin-updater 2.14's own): a key made
    // with `tauri signer generate`, and the bytes "test" signed with `tauri
    // signer sign --app-version 2.0.0`
    const TAURI_PUBLIC_KEY: &str = "untrusted comment: minisign public key: A49EF4746FA6E6D7\nRWTX5qZvdPSepDGWg2bz8n6ArQnBMwgQK1geVosorcbLzcunD+KJ+cYe";
    const TAURI_SIGNATURE: &str = "untrusted comment: signature from tauri secret key\nRUTX5qZvdPSepPF2zsjvP/L/TQtBLRnWRvaq+4omK1UEugtQp/USrGiI1v762sDy3id3aor7bRC9ETyeot6lMHHrWoZ5hjUM1Qc=\ntrusted comment: timestamp:1791649051\tfile:payload\tversion:2.0.0\nDDq2CLOEl5cQmC2eJRHlnZPVViOZsu1CjpPKQoei4A/XBfkDqGO0B+QMrcrHgEQ5qV3mne75tdFWhJpqUXh9AA==";

    fn throwaway(password: &str) -> (String, String) {
        generate_key(Some(password.into())).unwrap()
    }

    fn decoded(encoded: &str) -> String {
        decode_base64(encoded, "test").unwrap()
    }

    #[test]
    fn a_signed_file_passes_the_apps_check() {
        let (sk, pk) = throwaway("correct horse");
        let secret = secret_key(&sk, Some("correct horse".into())).unwrap();
        let data = b"MZ an installer".as_slice();
        let sig = sign_bytes(&secret, data, "BlueCaster-3.0.5-win64-setup.exe", "3.0.5", 1_800_000_000).unwrap();
        assert_eq!(verify(data, &sig, &pk, "3.0.5"), Ok(()));
        assert_eq!(verify(data, &sig, &pk, "v3.0.5"), Ok(()));
        // Changed bytes, another version, another key
        assert!(verify(b"MZ an installeR", &sig, &pk, "3.0.5").is_err());
        assert!(verify(data, &sig, &pk, "3.0.6").is_err());
        let (_, other_pk) = throwaway("");
        assert!(verify(data, &sig, &other_pk, "3.0.5").is_err());
    }

    #[test]
    fn the_signature_has_the_tauri_clis_layout() {
        let (sk, _) = throwaway("pw");
        let secret = secret_key(&sk, Some("pw".into())).unwrap();
        let sig = sign_bytes(&secret, b"x".as_slice(), "setup.exe", "3.0.5-beta.1", 1_700_000_000).unwrap();
        // One base64 line, no line break (written as is to .sig and latest.json)
        assert!(!sig.contains(['\n', '\r', ' ']));
        let text = decoded(&sig);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert_eq!(lines[0], "untrusted comment: signature from tauri secret key");
        // "ED": minisign's prehashed algorithm, which the updater requires
        let raw = STANDARD.decode(lines[1]).unwrap();
        assert_eq!(&raw[..2], b"ED");
        assert_eq!(lines[2], "trusted comment: timestamp:1700000000\tfile:setup.exe\tversion:3.0.5-beta.1");
        // Same layout as the Tauri CLI's signature in ui-host's test vector
        let tauri: Vec<&str> = TAURI_SIGNATURE.lines().collect();
        assert_eq!(lines[0], tauri[0]);
        assert_eq!(&STANDARD.decode(tauri[1]).unwrap()[..2], b"ED");
        assert_eq!(raw.len(), STANDARD.decode(tauri[1]).unwrap().len());
    }

    #[test]
    fn the_key_files_have_the_tauri_clis_layout() {
        let (sk, pk) = throwaway("pw");
        assert!(!sk.contains(['\n', '\r']) && !pk.contains(['\n', '\r']));
        let pk_text = decoded(&pk);
        assert!(pk_text.starts_with("untrusted comment: minisign public key: "), "{pk_text}");
        let sk_text = decoded(&sk);
        assert!(sk_text.starts_with("untrusted comment: "), "{sk_text}");
        // ui-host's build.rs takes the .pub content as one line
        let tauri_pk = STANDARD.encode(TAURI_PUBLIC_KEY);
        assert_eq!(decoded(&tauri_pk).lines().count(), pk_text.lines().count());
    }

    #[test]
    fn a_key_made_by_the_tauri_cli_signs() {
        // Encrypted with an empty password; a wrong one is refused
        let secret = secret_key(TAURI_TEST_KEY, Some(String::new())).unwrap();
        assert!(secret_key(TAURI_TEST_KEY, Some("wrong".into())).is_err());
        let sig = sign_bytes(&secret, b"TAURI".as_slice(), "payload", "1.2.3", 1).unwrap();
        assert!(decoded(&sig).contains("trusted comment: timestamp:1\tfile:payload\tversion:1.2.3\n"));
    }

    #[test]
    fn ui_hosts_fixed_vector_passes_the_same_check() {
        let (pk, sig) = (STANDARD.encode(TAURI_PUBLIC_KEY), STANDARD.encode(TAURI_SIGNATURE));
        assert_eq!(verify(b"test", &sig, &pk, "2.0.0"), Ok(()));
        assert!(verify(b"test", &sig, &pk, "2.0.1").is_err());
    }

    #[test]
    fn files_round_trip_through_sign_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let (sk, pk) = throwaway("pw");
        let (sk_path, pk_path) = save_keypair(&dir.path().join("keys").join("updater.key"), &sk, &pk, false).unwrap();
        assert_eq!(pk_path.file_name().unwrap(), "updater.key.pub");
        // No second key over the first one without --force
        assert!(save_keypair(&sk_path, "x", "y", false).is_err());
        assert_eq!(std::fs::read_to_string(&sk_path).unwrap(), sk);

        let setup = dir.path().join("BlueCaster-3.0.5-win64-setup.exe");
        std::fs::write(&setup, b"MZ setup").unwrap();
        let key = read_value_or_file(sk_path.to_str().unwrap()).unwrap();
        let secret = secret_key(&key, Some("pw".into())).unwrap();
        let (sig_path, encoded) = sign_file(&secret, &setup, "3.0.5").unwrap();
        assert_eq!(sig_path, dir.path().join("BlueCaster-3.0.5-win64-setup.exe.sig"));
        let written = std::fs::read_to_string(&sig_path).unwrap();
        assert_eq!(written, encoded);
        assert!(decoded(&written).contains("\tfile:BlueCaster-3.0.5-win64-setup.exe\tversion:3.0.5\n"));
        let pubkey = read_value_or_file(pk_path.to_str().unwrap()).unwrap();
        assert_eq!(verify(&std::fs::read(&setup).unwrap(), &written, &pubkey, "3.0.5"), Ok(()));

        // The command line, end to end
        let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let setup_arg = setup.to_str().unwrap();
        let pk_arg = pk_path.to_str().unwrap();
        run(&s(&["verify", "--app-version", "3.0.5", "--pubkey", pk_arg, setup_arg])).unwrap();
        assert!(run(&s(&["verify", "--app-version", "3.0.4", "--pubkey", pk_arg, setup_arg])).is_err());
        assert!(run(&s(&["sign", setup_arg])).is_err(), "sign without --app-version");
    }

    #[test]
    fn where_the_signature_goes() {
        assert_eq!(signature_path(Path::new("a/setup.exe")), Path::new("a/setup.exe.sig"));
        assert_eq!(signature_path(Path::new("a/app.tar.gz")), Path::new("a/app.tar.gz.sig"));
        assert_eq!(signature_path(Path::new("a/payload")), Path::new("a/payload.sig"));
    }

    #[test]
    fn a_tab_or_line_break_cannot_add_a_field() {
        assert!(trusted_comment(1, "a\tversion:9.9.9", "1.0.0").is_err());
        assert!(trusted_comment(1, "setup.exe", "1.0.0\tx").is_err());
        assert!(trusted_comment(1, "setup.exe", "1.0.0\n").is_err());
        assert!(trusted_comment(1, "setup.exe", "").is_err());
    }
}
