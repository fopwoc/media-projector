use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::PreflightConfig;
use crate::logging;
use crate::matcher::{ProjectionPlan, build_plan, episode_identity, normalized_stem};
use crate::media::{MediaFile, MediaKind, Scan, SourceEntry};
use crate::{Result, invalid_input};

#[derive(Serialize)]
struct PreflightReport {
    schema_version: u32,
    generator: GeneratorReport,
    generated_at_unix_seconds: u64,
    source: String,
    configuration: ConfigurationReport,
    summary: SummaryReport,
    ignored_extensions: BTreeMap<String, usize>,
    entries: Vec<EntryReport>,
    planned_operations: Vec<OperationReport>,
    skipped_sidecars: Vec<SkippedReport>,
}

#[derive(Serialize)]
struct GeneratorReport {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct ConfigurationReport {
    settle_seconds: u64,
    mode: &'static str,
    content_reads: bool,
    target_mutations: bool,
}

#[derive(Serialize)]
struct SummaryReport {
    top_level_entries: usize,
    directories_visited: usize,
    regular_files_visited: usize,
    recognized_files: usize,
    eligible_files: usize,
    unsettled_files: usize,
    ignored_files: usize,
    skipped_symlinks: usize,
    video_files: usize,
    audio_files: usize,
    subtitle_files: usize,
    source_logical_bytes: u64,
    planned_links: usize,
    planned_logical_bytes: u64,
    matched_sidecars: usize,
    skipped_sidecars: usize,
    lossy_paths: usize,
}

#[derive(Serialize)]
struct EntryReport {
    source_entry: String,
    target_directory: Option<String>,
    files: Vec<FileReport>,
}

#[derive(Serialize)]
struct FileReport {
    path: String,
    kind: &'static str,
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
    eligible: bool,
    normalized_stem: String,
    episode: Option<EpisodeReport>,
}

#[derive(Serialize)]
struct EpisodeReport {
    season: Option<u16>,
    episode: u16,
}

#[derive(Serialize)]
struct OperationReport {
    source: String,
    target: String,
    kind: &'static str,
    match_strategy: Option<&'static str>,
}

#[derive(Serialize)]
struct SkippedReport {
    source: String,
    reason: &'static str,
}

pub fn run_preflight(config: PreflightConfig) -> Result<()> {
    let source = fs::canonicalize(&config.source).map_err(|error| {
        invalid_input(format!(
            "cannot resolve source directory {}: {error}",
            config.source.display()
        ))
    })?;
    logging::info(
        "preflight",
        format_args!(
            "scanning {} without reading file contents",
            source.display()
        ),
    );

    let scan = crate::media::scan(&source, config.settle)?;
    let plan = build_plan(&scan)?;
    let report = build_report(&source, config.settle.as_secs(), &scan, &plan);
    write_report(&config.output, &report)?;
    logging::info(
        "preflight",
        format_args!(
            "report contains {} recognized files and {} planned links",
            report.summary.recognized_files, report.summary.planned_links
        ),
    );
    Ok(())
}

fn build_report(
    source: &Path,
    settle_seconds: u64,
    scan: &Scan,
    plan: &ProjectionPlan,
) -> PreflightReport {
    let files = scan.entries.iter().flat_map(|entry| &entry.files);
    let mut video_files = 0;
    let mut audio_files = 0;
    let mut subtitle_files = 0;
    let mut eligible_files = 0;
    let mut source_logical_bytes = 0_u64;
    let mut path_sizes = BTreeMap::new();
    let mut lossy_paths = 0;

    for file in files {
        match file.kind {
            MediaKind::Video => video_files += 1,
            MediaKind::Audio => audio_files += 1,
            MediaKind::Subtitle => subtitle_files += 1,
        }
        eligible_files += usize::from(file.eligible);
        source_logical_bytes = source_logical_bytes.saturating_add(file.size_bytes);
        path_sizes.insert(file.source.clone(), file.size_bytes);
        lossy_paths += usize::from(file.source.to_str().is_none());
    }

    let planned_logical_bytes = plan.operations.iter().fold(0_u64, |total, operation| {
        total.saturating_add(
            path_sizes
                .get(&operation.source)
                .copied()
                .unwrap_or_default(),
        )
    });
    let entries = scan.entries.iter().map(entry_report).collect();
    let planned_operations = plan
        .operations
        .iter()
        .map(|operation| OperationReport {
            source: relative_string(source, &operation.source),
            target: path_string(&operation.target),
            kind: kind_name(operation.kind),
            match_strategy: operation.match_strategy,
        })
        .collect();
    let skipped_sidecars = plan
        .skipped
        .iter()
        .map(|skipped| SkippedReport {
            source: relative_string(source, &skipped.source),
            reason: skipped.reason,
        })
        .collect();

    PreflightReport {
        schema_version: 1,
        generator: GeneratorReport {
            name: "media-projector",
            version: env!("CARGO_PKG_VERSION"),
        },
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        source: path_string(source),
        configuration: ConfigurationReport {
            settle_seconds,
            mode: "preflight-debug",
            content_reads: false,
            target_mutations: false,
        },
        summary: SummaryReport {
            top_level_entries: scan.entries.len(),
            directories_visited: scan.stats.directories,
            regular_files_visited: scan.stats.regular_files,
            recognized_files: scan.stats.recognized,
            eligible_files,
            unsettled_files: scan.stats.unsettled,
            ignored_files: scan.stats.ignored_files,
            skipped_symlinks: scan.stats.skipped_symlinks,
            video_files,
            audio_files,
            subtitle_files,
            source_logical_bytes,
            planned_links: plan.links.len(),
            planned_logical_bytes,
            matched_sidecars: plan.stats.matched_sidecars,
            skipped_sidecars: plan.stats.skipped_sidecars,
            lossy_paths,
        },
        ignored_extensions: scan.stats.ignored_extensions.clone(),
        entries,
        planned_operations,
        skipped_sidecars,
    }
}

fn entry_report(entry: &SourceEntry) -> EntryReport {
    let target_directory = entry.target_directory.as_deref().map(path_string_os);
    EntryReport {
        source_entry: target_directory.clone().unwrap_or_else(|| ".".to_owned()),
        target_directory,
        files: entry.files.iter().map(file_report).collect(),
    }
}

fn file_report(file: &MediaFile) -> FileReport {
    FileReport {
        path: path_string(&file.relative),
        kind: kind_name(file.kind),
        size_bytes: file.size_bytes,
        modified_unix_seconds: file.modified_unix_seconds,
        eligible: file.eligible,
        normalized_stem: normalized_stem(&file.relative),
        episode: episode_identity(&file.relative).map(|identity| EpisodeReport {
            season: identity.season,
            episode: identity.episode,
        }),
    }
}

fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Subtitle => "subtitle",
    }
}

fn relative_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| path_string(path), path_string)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn path_string_os(path: &std::ffi::OsStr) -> String {
    path.to_string_lossy().into_owned()
}

fn write_report(output: &Path, report: &PreflightReport) -> Result<()> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = temporary_path(output);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let mut writer = BufWriter::new(file);
    if let Err(error) = serde_json::to_writer_pretty(&mut writer, report) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    if let Err(error) = fs::rename(&temporary, output) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn temporary_path(output: &Path) -> PathBuf {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = output.file_name().unwrap_or_default().to_string_lossy();
    for sequence in 0_u32.. {
        let candidate = parent.join(format!(
            ".{name}.media-projector.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use serde_json::Value;

    use super::*;
    use crate::LogLevel;

    #[test]
    fn report_contains_hierarchy_plan_and_match_strategy_without_a_target() {
        let root = std::env::temp_dir().join(format!(
            "media-projector-preflight-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let output = root.join("report.json");
        fs::create_dir_all(source.join("Show/Audio/ru")).unwrap();
        fs::write(source.join("Show/Show - 01.mkv"), b"video").unwrap();
        fs::write(source.join("Show/Audio/ru/01.mka"), b"audio").unwrap();
        fs::write(source.join("Show/poster.jpg"), b"ignored").unwrap();

        run_preflight(PreflightConfig {
            source,
            output: output.clone(),
            settle: Duration::ZERO,
            log_level: LogLevel::Error,
        })
        .unwrap();

        let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["summary"]["recognized_files"], 2);
        assert_eq!(report["summary"]["ignored_files"], 1);
        assert_eq!(
            report["planned_operations"][1]["match_strategy"],
            "episode-identity"
        );
        assert_eq!(report["configuration"]["target_mutations"], false);
        assert!(!root.join("target").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
