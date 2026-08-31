use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Result, invalid_data, invalid_input};

pub const MARKER_NAME: &str = ".media-projector";
const MARKER_CONTENT: &str = "media-projector projection\n";
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Eq, PartialEq)]
pub struct PreparedPaths {
    pub source: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ReconcileReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed_files: usize,
    pub removed_directories: usize,
}

pub fn prepare(source: &Path, target: &Path) -> Result<PreparedPaths> {
    let source = fs::canonicalize(source).map_err(|error| {
        invalid_input(format!(
            "cannot resolve source directory {}: {error}",
            source.display()
        ))
    })?;
    if !fs::metadata(&source)?.is_dir() {
        return Err(invalid_input(format!(
            "source path is not a directory: {}",
            source.display()
        ))
        .into());
    }

    let created_target = !target.exists();
    if created_target {
        fs::create_dir_all(target).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "cannot create target directory {}: {error}",
                    target.display()
                ),
            )
        })?;
    }
    let target = fs::canonicalize(target).map_err(|error| {
        invalid_input(format!(
            "cannot resolve target directory {}: {error}",
            target.display()
        ))
    })?;
    if !fs::metadata(&target)?.is_dir() {
        return Err(invalid_input(format!(
            "target path is not a directory: {}",
            target.display()
        ))
        .into());
    }

    if source.starts_with(&target) || target.starts_with(&source) {
        if created_target {
            let _ = fs::remove_dir(&target);
        }
        return Err(invalid_input(format!(
            "source and target must not overlap: {} and {}",
            source.display(),
            target.display()
        ))
        .into());
    }

    let source_device = fs::metadata(&source)?.dev();
    let target_device = fs::metadata(&target)?.dev();
    if source_device != target_device {
        return Err(invalid_input(format!(
            "source and target are on different filesystems; hardlinks require one filesystem ({} and {})",
            source.display(),
            target.display()
        ))
        .into());
    }

    claim_target(&target)?;
    Ok(PreparedPaths { source, target })
}

fn claim_target(target: &Path) -> Result<()> {
    let marker = target.join(MARKER_NAME);
    if let Ok(metadata) = marker.symlink_metadata() {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid_data(format!(
                "target marker {} is not a regular media-projector marker",
                marker.display()
            ))
            .into());
        }
        let content = fs::read_to_string(&marker).map_err(|error| {
            invalid_data(format!(
                "cannot read projection marker {}: {error}",
                marker.display()
            ))
        })?;
        if content != MARKER_CONTENT {
            return Err(invalid_data(format!(
                "target marker {} is not owned by media-projector",
                marker.display()
            ))
            .into());
        }
        return Ok(());
    }

    if fs::read_dir(target)?.next().is_some() {
        return Err(invalid_input(format!(
            "target directory is not empty and has no {MARKER_NAME} ownership marker: {}",
            target.display()
        ))
        .into());
    }

    fs::write(&marker, MARKER_CONTENT).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "cannot write projection marker {}: {error}",
                marker.display()
            ),
        )
    })?;
    Ok(())
}

pub fn reconcile(target: &Path, desired: &BTreeMap<PathBuf, PathBuf>) -> Result<ReconcileReport> {
    validate_desired_paths(desired)?;
    let mut report = ReconcileReport::default();

    for (relative, source) in desired {
        let destination = target.join(relative);
        ensure_parent_directory(target, &destination)?;
        if destination
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir())
        {
            fs::remove_dir_all(&destination)?;
        }

        if same_file(source, &destination)? {
            report.unchanged += 1;
            continue;
        }

        let existed = destination.symlink_metadata().is_ok();
        replace_with_hardlink(source, &destination)?;
        if existed {
            report.updated += 1;
        } else {
            report.created += 1;
        }
    }

    clean_directory(target, target, desired, &mut report)?;
    Ok(report)
}

fn validate_desired_paths(desired: &BTreeMap<PathBuf, PathBuf>) -> Result<()> {
    for relative in desired.keys() {
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(invalid_data(format!(
                "projection path is not a safe relative path: {}",
                relative.display()
            ))
            .into());
        }
    }
    Ok(())
}

fn ensure_parent_directory(target: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .expect("a projection destination always has a parent");
    let relative = parent
        .strip_prefix(target)
        .expect("projection destinations remain below the target");
    let mut current = target.to_owned();
    for component in relative.components() {
        current.push(component);
        match current.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                fs::remove_file(&current)?;
                fs::create_dir(&current)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn same_file(source: &Path, destination: &Path) -> Result<bool> {
    let source = fs::metadata(source).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("cannot read source file {}: {error}", source.display()),
        )
    })?;
    let destination = match fs::metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    Ok(source.dev() == destination.dev() && source.ino() == destination.ino())
}

fn replace_with_hardlink(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .expect("a projection destination always has a parent");
    let temporary = loop {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".media-projector.tmp.{}.{}",
            std::process::id(),
            sequence
        ));
        if candidate.symlink_metadata().is_err() {
            break candidate;
        }
    };

    fs::hard_link(source, &temporary).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "cannot hardlink {} into {}: {error}",
                source.display(),
                parent.display()
            ),
        )
    })?;
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(std::io::Error::new(
            error.kind(),
            format!(
                "cannot install projection file {}: {error}",
                destination.display()
            ),
        )
        .into());
    }
    Ok(())
}

fn clean_directory(
    root: &Path,
    directory: &Path,
    desired: &BTreeMap<PathBuf, PathBuf>,
    report: &mut ReconcileReport,
) -> Result<()> {
    let entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let path = entry.path();
        let metadata = path.symlink_metadata()?;
        if metadata.is_dir() && !entry.file_type()?.is_symlink() {
            clean_directory(root, &path, desired, report)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)?;
                report.removed_directories += 1;
            }
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .expect("walked target paths remain below the target");
        if relative == Path::new(MARKER_NAME) || desired.contains_key(relative) {
            continue;
        }
        fs::remove_file(&path)?;
        report.removed_files += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn refuses_to_claim_a_nonempty_unowned_target() {
        let root = test_directory("ownership");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("user-file"), b"keep").unwrap();

        let error = prepare(&source, &target).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("has no .media-projector ownership marker")
        );
        assert!(target.join("user-file").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reconciliation_creates_hardlinks_and_removes_stale_files() {
        let root = test_directory("reconcile");
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("episode.mkv"), b"video").unwrap();
        let paths = prepare(&source, &target).unwrap();
        fs::write(target.join("stale"), b"stale").unwrap();
        let desired = BTreeMap::from([(
            PathBuf::from("Show/episode.mkv"),
            source.join("episode.mkv"),
        )]);

        let first = reconcile(&paths.target, &desired).unwrap();
        let second = reconcile(&paths.target, &desired).unwrap();

        assert_eq!(first.created, 1);
        assert_eq!(first.removed_files, 1);
        assert_eq!(second.unchanged, 1);
        let original = fs::metadata(source.join("episode.mkv")).unwrap();
        let projected = fs::metadata(target.join("Show/episode.mkv")).unwrap();
        assert_eq!(original.ino(), projected.ino());
        fs::remove_dir_all(root).unwrap();
    }

    fn test_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "media-projector-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
}
