//! Build-time check, for the -sys crates' build scripts, that a C library
//! under extern/ is the commit listed in extern/versions.txt.
//!
//! A pull that moves a submodule leaves its checkout behind until
//! `git submodule update`. The build then compiled the old library against
//! bindings written for the new one: with BTstack, event codes shifted by one
//! and connections failed with nothing in the build to say why (issue #86).
//!
//! The commit is read with `git`; a source folder that is not a git checkout
//! (copied sources) or a machine without git gets a warning, not an error.

use std::fs;
use std::path::Path;
use std::process::Command;

/// What extern/versions.txt asks for one library
#[derive(Debug, PartialEq, Eq)]
enum Expected {
    /// Commit (full or abbreviated) and the version name for messages
    Commit { commit: String, version: String },
    /// `*`: any version
    Any,
}

/// Stops the build when `dir`, the source folder of extern library `name`
/// (its line in extern/versions.txt), is not at the listed commit.
/// `manifest` is the calling crate's CARGO_MANIFEST_DIR.
pub fn check(manifest: &Path, name: &str, dir: &Path) {
    let root = manifest.join("../../../..");
    let list = std::path::absolute(root.join("extern/versions.txt")).unwrap();
    println!("cargo:rerun-if-changed={}", list.display());
    let text = fs::read_to_string(&list).unwrap_or_else(|e| panic!("cannot read {}: {e}", list.display()));
    let expected = parse(&text, name).unwrap_or_else(|| panic!("{name} has no line in {}", list.display()));
    let Expected::Commit { commit, version } = expected else {
        return;
    };

    // A folder default-placed under extern/ is named by its path there; one
    // from an environment variable by its own path
    let label = if same_dir(dir, &root.join("extern").join(name)) {
        format!("extern/{name}")
    } else {
        dir.display().to_string()
    };
    let Some(found) = head(dir) else {
        println!("cargo:warning={label}: not a git checkout or git not found, its version is not checked");
        return;
    };
    if is_commit(&found, &commit) {
        return;
    }
    let described = git(dir, &["describe", "--tags", "--always"]).unwrap_or_else(|| short(&found).to_string());
    panic!(
        "\n\n{label} is at {} ({described}), but this source tree is written for {} ({version}).\n\
         After a pull, update the submodules:\n\n    \
         git submodule update --init --recursive\n\n\
         To build with this version anyway, change the {name} line in\n\n    {}\n\n\
         to `{name}  {found}`, or `{name}  *` to skip the check.\n\n",
        short(&found),
        short(&commit),
        list.display(),
    );
}

/// The line for `name`: `name commit [version]` or `name *`; `#` starts a comment
fn parse(text: &str, name: &str) -> Option<Expected> {
    text.lines().map(|l| l.split('#').next().unwrap_or("")).find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != name {
            return None;
        }
        let commit = words.next()?;
        if commit == "*" {
            return Some(Expected::Any);
        }
        let version = words.collect::<Vec<_>>().join(" ");
        Some(Expected::Commit { commit: commit.to_ascii_lowercase(), version })
    })
}

/// Whether HEAD `found` is the listed `commit`, which may be abbreviated
/// (7 characters at least)
fn is_commit(found: &str, commit: &str) -> bool {
    commit.len() >= 7 && found.to_ascii_lowercase().starts_with(commit)
}

fn short(commit: &str) -> &str {
    commit.get(..8).unwrap_or(commit)
}

/// HEAD of `dir` when it is the top of a git checkout of its own: an empty
/// or copied folder inside BlueCaster's own checkout would otherwise answer
/// with BlueCaster's commit
fn head(dir: &Path) -> Option<String> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    if !same_dir(Path::new(&top), dir) {
        return None;
    }
    git(dir, &["rev-parse", "HEAD"])
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !text.trim().is_empty()).then(|| text.trim().to_string())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| fs::canonicalize(p).ok();
    matches!((canon(a), canon(b)), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "# comment\n\
        btstack   51e11ce3692abfe31b4ee3d66cffdeb1ae1626f2  v1.8.3\n\
        opus      *   # any\n\
        libldac   EEEE1A3F  android-15.0.0_r36-4-geeee1a3\n";

    #[test]
    fn parses_lines() {
        assert_eq!(
            parse(LIST, "btstack"),
            Some(Expected::Commit {
                commit: "51e11ce3692abfe31b4ee3d66cffdeb1ae1626f2".into(),
                version: "v1.8.3".into()
            })
        );
        assert_eq!(parse(LIST, "opus"), Some(Expected::Any));
        assert_eq!(
            parse(LIST, "libldac"),
            Some(Expected::Commit { commit: "eeee1a3f".into(), version: "android-15.0.0_r36-4-geeee1a3".into() })
        );
        assert_eq!(parse(LIST, "fdk-aac"), None);
        assert_eq!(parse(LIST, "comment"), None);
    }

    #[test]
    fn matches_full_and_abbreviated() {
        let head = "51e11ce3692abfe31b4ee3d66cffdeb1ae1626f2";
        assert!(is_commit(head, head));
        assert!(is_commit(head, "51e11ce"));
        assert!(!is_commit(head, "51e11c"));
        assert!(!is_commit(head, "5bc5cbdbeec33be1fdbd0d50e04c0f6deab99d2d"));
        assert!(is_commit(&head.to_ascii_uppercase(), "51e11ce3"));
    }
}
