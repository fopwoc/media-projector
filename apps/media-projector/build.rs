use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=VERSION");

    let workspace = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .parent()
        .and_then(Path::parent)
        .expect("application belongs to the repository workspace")
        .to_owned();
    let resolver = workspace.join("scripts/version.sh");
    println!("cargo:rerun-if-changed={}", resolver.display());
    watch_git_state(&workspace);

    let output = Command::new(&resolver)
        .current_dir(&workspace)
        .output()
        .expect("run version resolver");
    if !output.status.success() {
        panic!(
            "version resolver failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let version = String::from_utf8(output.stdout).expect("version is UTF-8");
    let version = version.trim();
    assert!(!version.is_empty(), "version resolver returned no version");

    println!("cargo:rustc-env=VERSION={version}");
}

fn watch_git_state(workspace: &Path) {
    let Some(git_directory) = git_output(workspace, &["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    let git_directory = PathBuf::from(git_directory);
    let common_directory = git_output(workspace, &["rev-parse", "--git-common-dir"])
        .map(PathBuf::from)
        .map(|path| absolute_path(workspace, path))
        .unwrap_or_else(|| git_directory.clone());

    for path in [
        git_directory.join("HEAD"),
        git_directory.join("index"),
        common_directory.join("packed-refs"),
        common_directory.join("refs/tags"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    if let Some(reference) = git_output(workspace, &["symbolic-ref", "--quiet", "HEAD"]) {
        println!(
            "cargo:rerun-if-changed={}",
            common_directory.join(reference).display()
        );
    }
}

fn git_output(workspace: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(workspace)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn absolute_path(workspace: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}
