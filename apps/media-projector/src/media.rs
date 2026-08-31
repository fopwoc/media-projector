use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::logging;
use crate::{ProjectionLayout, Result, invalid_input};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Video,
    Audio,
    Subtitle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaFile {
    pub source: PathBuf,
    pub relative: PathBuf,
    pub kind: MediaKind,
    pub size_bytes: u64,
    pub modified_unix_seconds: Option<u64>,
    pub eligible: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SourceEntry {
    pub target_directory: Option<OsString>,
    pub files: Vec<MediaFile>,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ScanStats {
    pub directories: usize,
    pub regular_files: usize,
    pub recognized: usize,
    pub unsettled: usize,
    pub skipped_symlinks: usize,
    pub ignored_files: usize,
    pub ignored_extensions: BTreeMap<String, usize>,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Scan {
    pub entries: Vec<SourceEntry>,
    pub stats: ScanStats,
}

pub fn scan(source: &Path, settle: Duration, layout: ProjectionLayout) -> Result<Scan> {
    let metadata = fs::metadata(source).map_err(|error| {
        invalid_input(format!(
            "cannot read source directory {}: {error}",
            source.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(invalid_input(format!(
            "source path is not a directory: {}",
            source.display()
        ))
        .into());
    }

    let now = SystemTime::now();
    let mut result = Scan::default();
    match layout {
        ProjectionLayout::Flat => scan_flat(source, settle, now, &mut result)?,
        ProjectionLayout::GroupByDirectory => scan_grouped(source, settle, now, &mut result)?,
    }
    Ok(result)
}

fn scan_flat(source: &Path, settle: Duration, now: SystemTime, result: &mut Scan) -> Result<()> {
    let mut files = Vec::new();
    collect_files(source, source, settle, now, &mut files, &mut result.stats)?;
    if !files.is_empty() {
        result.entries.push(SourceEntry {
            target_directory: None,
            files,
        });
    }
    Ok(())
}

fn scan_grouped(source: &Path, settle: Duration, now: SystemTime, result: &mut Scan) -> Result<()> {
    let mut root_files = Vec::new();
    let mut children = read_directory(source)?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        let path = child.path();
        let file_type = match child.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };

        if file_type.is_symlink() {
            result.stats.skipped_symlinks += 1;
            continue;
        }

        if file_type.is_dir() {
            result.stats.directories += 1;
            let mut files = Vec::new();
            collect_files(&path, &path, settle, now, &mut files, &mut result.stats)?;
            if !files.is_empty() {
                result.entries.push(SourceEntry {
                    target_directory: Some(child.file_name()),
                    files,
                });
            }
        } else if file_type.is_file() {
            result.stats.regular_files += 1;
            collect_file(
                &path,
                Path::new(&child.file_name()),
                settle,
                now,
                &mut root_files,
                &mut result.stats,
            )?;
        }
    }

    if !root_files.is_empty() {
        result.entries.push(SourceEntry {
            target_directory: None,
            files: root_files,
        });
    }

    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    settle: Duration,
    now: SystemTime,
    files: &mut Vec<MediaFile>,
    stats: &mut ScanStats,
) -> Result<()> {
    let mut children = match read_directory(directory) {
        Ok(children) => children,
        Err(error) if is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        let path = child.path();
        let file_type = match child.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };

        if file_type.is_symlink() {
            stats.skipped_symlinks += 1;
        } else if file_type.is_dir() {
            stats.directories += 1;
            collect_files(root, &path, settle, now, files, stats)?;
        } else if file_type.is_file() {
            stats.regular_files += 1;
            let relative = path
                .strip_prefix(root)
                .expect("walked paths remain below their root");
            collect_file(&path, relative, settle, now, files, stats)?;
        }
    }

    Ok(())
}

fn collect_file(
    source: &Path,
    relative: &Path,
    settle: Duration,
    now: SystemTime,
    files: &mut Vec<MediaFile>,
    stats: &mut ScanStats,
) -> Result<()> {
    let Some(kind) = classify(source) else {
        stats.ignored_files += 1;
        *stats
            .ignored_extensions
            .entry(extension_label(source))
            .or_default() += 1;
        return Ok(());
    };

    let metadata = match fs::metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let modified = metadata.modified().ok();
    let stable = settle.is_zero()
        || modified
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= settle);

    if !stable {
        stats.unsettled += 1;
    }

    files.push(MediaFile {
        source: source.to_owned(),
        relative: relative.to_owned(),
        kind,
        size_bytes: metadata.len(),
        modified_unix_seconds: modified.and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs())
        }),
        eligible: stable,
    });
    stats.recognized += 1;
    Ok(())
}

fn extension_label(path: &Path) -> String {
    match path.extension() {
        Some(extension) => extension
            .to_str()
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "<non-utf8>".to_owned()),
        None => "<none>".to_owned(),
    }
}

fn classify(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "mkv" | "mp4" | "m4v" | "avi" | "webm" | "mov" | "mpg" | "mpeg" | "ts" | "m2ts" => {
            Some(MediaKind::Video)
        }
        "mka" | "aac" | "ac3" | "eac3" | "dts" | "dtshd" | "flac" | "mp3" | "ogg" | "opus"
        | "wav" => Some(MediaKind::Audio),
        "ass" | "ssa" | "srt" | "sub" | "idx" | "vtt" | "ttml" => Some(MediaKind::Subtitle),
        _ => None,
    }
}

fn read_directory(path: &Path) -> Result<Vec<fs::DirEntry>> {
    fs::read_dir(path)
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot read directory {}: {error}", path.display()),
            )
        })?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn is_not_found(error: &crate::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

pub fn log_scan_stats(stats: &ScanStats) {
    logging::debug(
        "scanner",
        format_args!(
            "visited {} directories and {} files; recognized {}, deferred {}, ignored {}, skipped {} symlinks",
            stats.directories,
            stats.regular_files,
            stats.recognized,
            stats.unsettled,
            stats.ignored_files,
            stats.skipped_symlinks
        ),
    );
    if stats.skipped_symlinks > 0 {
        logging::warn(
            "scanner",
            format_args!(
                "ignored {} source symlinks; source symlinks are never followed",
                stats.skipped_symlinks
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::UNIX_EPOCH;

    use super::*;

    #[test]
    fn scans_recognized_files_without_following_symlinks() {
        let root = test_directory("scan");
        let entry = root.join("Show");
        fs::create_dir_all(entry.join("Subs")).unwrap();
        fs::write(entry.join("Episode 01.mkv"), b"video").unwrap();
        fs::write(entry.join("Subs/01.ass"), b"subtitle").unwrap();
        fs::write(entry.join("poster.jpg"), b"image").unwrap();
        std::os::unix::fs::symlink(&entry, root.join("Show link")).unwrap();

        let scan = scan(&root, Duration::ZERO, ProjectionLayout::GroupByDirectory).unwrap();

        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].files.len(), 2);
        assert_eq!(scan.stats.recognized, 2);
        assert_eq!(scan.stats.ignored_files, 1);
        assert_eq!(scan.stats.skipped_symlinks, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flat_layout_combines_all_nested_files_into_one_entry() {
        let root = test_directory("flat-scan");
        fs::create_dir_all(root.join("Movie One/Extras")).unwrap();
        fs::create_dir_all(root.join("Movie Two")).unwrap();
        fs::write(root.join("Movie One/movie.mkv"), b"one").unwrap();
        fs::write(root.join("Movie One/Extras/movie.ass"), b"subtitle").unwrap();
        fs::write(root.join("Movie Two/movie.mkv"), b"two").unwrap();

        let scan = scan(&root, Duration::ZERO, ProjectionLayout::Flat).unwrap();

        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].target_directory, None);
        assert_eq!(scan.entries[0].files.len(), 3);
        assert!(
            scan.entries[0]
                .files
                .iter()
                .any(|file| file.relative == Path::new("Movie One/movie.mkv"))
        );
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
