use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::media::{EntryTarget, MediaFile, MediaKind, Scan, SourceEntry};
use crate::projection::MARKER_NAME;
use crate::{Result, invalid_data};

#[derive(Debug, Default, Eq, PartialEq)]
pub struct PlanStats {
    pub videos: usize,
    pub matched_sidecars: usize,
    pub skipped_sidecars: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SkippedSidecar {
    pub source: PathBuf,
    pub reason: &'static str,
}

#[derive(Debug, Eq, PartialEq)]
pub struct PlannedOperation {
    pub source: PathBuf,
    pub target: PathBuf,
    pub kind: MediaKind,
    pub match_strategy: Option<&'static str>,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ProjectionPlan {
    pub links: BTreeMap<PathBuf, PathBuf>,
    pub operations: Vec<PlannedOperation>,
    pub skipped: Vec<SkippedSidecar>,
    pub stats: PlanStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EpisodeIdentity {
    pub season: Option<u16>,
    pub episode: u16,
}

struct PlannedVideo<'a> {
    file: &'a MediaFile,
    target_directory: Option<OsString>,
    target_name: OsString,
    normalized_stem: String,
    identity: Option<EpisodeIdentity>,
    tokens: BTreeSet<String>,
}

pub fn build_plan(scan: &Scan) -> Result<ProjectionPlan> {
    let mut plan = ProjectionPlan::default();
    for entry in &scan.entries {
        plan_entry(entry, &mut plan)?;
    }
    Ok(plan)
}

fn plan_entry(entry: &SourceEntry, plan: &mut ProjectionPlan) -> Result<()> {
    if matches!(
        &entry.target,
        EntryTarget::Directory(directory) if directory == OsStr::new(MARKER_NAME)
    ) {
        return Err(invalid_data(format!(
            "source top-level directory {MARKER_NAME:?} conflicts with the projection marker"
        ))
        .into());
    }

    let mut used_names = HashSet::new();
    let mut used_directories = HashSet::from([OsString::from(MARKER_NAME)]);
    let mut videos = entry
        .files
        .iter()
        .filter(|file| file.eligible && file.kind == MediaKind::Video)
        .map(|file| {
            let preferred = file
                .relative
                .file_name()
                .expect("scanned files always have a file name");
            let target_name = allocate_name(preferred, &file.relative, &mut used_names);
            let target_directory = match &entry.target {
                EntryTarget::Root => None,
                EntryTarget::Directory(directory) => Some(directory.clone()),
                EntryTarget::PerVideoDirectory => Some(allocate_directory_name(
                    file.relative.file_stem().unwrap_or(preferred),
                    &file.relative,
                    &mut used_directories,
                )),
            };
            PlannedVideo {
                file,
                target_directory,
                target_name,
                normalized_stem: normalized_stem(&file.relative),
                identity: episode_identity(&file.relative),
                tokens: meaningful_tokens(file.relative.file_stem().unwrap_or_default()),
            }
        })
        .collect::<Vec<_>>();
    videos.sort_by(|left, right| left.file.relative.cmp(&right.file.relative));

    for video in &videos {
        let target = target_path(video.target_directory.as_deref(), &video.target_name);
        plan.links.insert(target.clone(), video.file.source.clone());
        plan.operations.push(PlannedOperation {
            source: video.file.source.clone(),
            target,
            kind: MediaKind::Video,
            match_strategy: None,
        });
        plan.stats.videos += 1;
    }

    for sidecar in entry
        .files
        .iter()
        .filter(|file| file.eligible && file.kind != MediaKind::Video)
    {
        let (video, match_strategy) = match match_video(sidecar, &videos) {
            Ok(matched) => matched,
            Err(reason) => {
                plan.skipped.push(SkippedSidecar {
                    source: sidecar.source.clone(),
                    reason,
                });
                plan.stats.skipped_sidecars += 1;
                continue;
            }
        };

        let target_name = sidecar_name(sidecar, video, &mut used_names);
        let target = target_path(video.target_directory.as_deref(), &target_name);
        plan.links.insert(target.clone(), sidecar.source.clone());
        plan.operations.push(PlannedOperation {
            source: sidecar.source.clone(),
            target,
            kind: sidecar.kind,
            match_strategy: Some(match_strategy),
        });
        plan.stats.matched_sidecars += 1;
    }

    Ok(())
}

fn match_video<'a>(
    sidecar: &MediaFile,
    videos: &'a [PlannedVideo<'a>],
) -> std::result::Result<(&'a PlannedVideo<'a>, &'static str), &'static str> {
    let normalized = normalized_stem(&sidecar.relative);
    let mut exact = Vec::new();
    if !normalized.is_empty() {
        exact = videos
            .iter()
            .filter(|video| video.normalized_stem == normalized)
            .collect::<Vec<_>>();
        if let [video] = exact.as_slice() {
            return Ok((*video, "normalized-basename"));
        }
    }

    let mut prefix_candidates = videos
        .iter()
        .filter(|video| {
            !video.normalized_stem.is_empty() && normalized.starts_with(&video.normalized_stem)
        })
        .collect::<Vec<_>>();
    if let Some(longest) = prefix_candidates
        .iter()
        .map(|video| video.normalized_stem.len())
        .max()
    {
        prefix_candidates.retain(|video| video.normalized_stem.len() == longest);
        if let [video] = prefix_candidates.as_slice() {
            return Ok((*video, "video-basename-prefix"));
        }
    }

    let identity_candidates =
        episode_identity(&sidecar.relative).map_or_else(Vec::new, |identity| {
            videos
                .iter()
                .filter(|video| {
                    video.identity.is_some_and(|video_identity| {
                        video_identity.episode == identity.episode
                            && match (video_identity.season, identity.season) {
                                (Some(video_season), Some(sidecar_season)) => {
                                    video_season == sidecar_season
                                }
                                _ => true,
                            }
                    })
                })
                .collect::<Vec<_>>()
        });
    if let [video] = identity_candidates.as_slice() {
        return Ok((*video, "episode-identity"));
    }
    if let [video] = videos {
        return Ok((video, "single-video-fallback"));
    }
    if exact.len() > 1 || prefix_candidates.len() > 1 || identity_candidates.len() > 1 {
        Err("ambiguous video match")
    } else {
        Err("no matching video")
    }
}

fn target_path(directory: Option<&OsStr>, name: &OsStr) -> PathBuf {
    directory.map_or_else(
        || PathBuf::from(name),
        |directory| PathBuf::from(directory).join(name),
    )
}

fn allocate_directory_name(
    preferred: &OsStr,
    relative: &Path,
    used: &mut HashSet<OsString>,
) -> OsString {
    if used.insert(preferred.to_owned()) {
        return preferred.to_owned();
    }

    let mut candidate = preferred.to_owned();
    candidate.push(format!(
        ".{:08x}",
        stable_hash(relative.as_os_str().as_bytes())
    ));
    if used.insert(candidate.clone()) {
        return candidate;
    }

    for sequence in 2_u32.. {
        let mut candidate = preferred.to_owned();
        candidate.push(format!(
            ".{:08x}.{sequence}",
            stable_hash(relative.as_os_str().as_bytes())
        ));
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

fn allocate_name(preferred: &OsStr, relative: &Path, used: &mut HashSet<OsString>) -> OsString {
    if used.insert(preferred.to_owned()) {
        return preferred.to_owned();
    }

    let path = Path::new(preferred);
    let stem = path.file_stem().unwrap_or(preferred).to_string_lossy();
    let extension = path.extension().map(|value| value.to_string_lossy());
    let parent_label = relative
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| sanitize_label(&component.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("_");

    if !parent_label.is_empty() {
        let candidate = file_name(&stem, &[parent_label], extension.as_deref());
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }

    let hash = stable_hash(relative.as_os_str().as_bytes());
    let candidate = file_name(&stem, &[format!("{hash:08x}")], extension.as_deref());
    used.insert(candidate.clone());
    candidate
}

fn sidecar_name(
    sidecar: &MediaFile,
    video: &PlannedVideo<'_>,
    used: &mut HashSet<OsString>,
) -> OsString {
    let video_path = Path::new(&video.target_name);
    let video_stem = video_path
        .file_stem()
        .unwrap_or(&video.target_name)
        .to_string_lossy();
    let extension = sidecar
        .relative
        .extension()
        .map(|value| value.to_string_lossy());
    let mut labels = sidecar_labels(sidecar, video);
    if labels.is_empty() {
        labels.push(match sidecar.kind {
            MediaKind::Audio => "audio".to_owned(),
            MediaKind::Subtitle => "subtitle".to_owned(),
            MediaKind::Video => unreachable!(),
        });
    }

    let candidate = file_name(&video_stem, &labels, extension.as_deref());
    if used.insert(candidate.clone()) {
        return candidate;
    }

    labels.push(format!(
        "{:08x}",
        stable_hash(sidecar.relative.as_os_str().as_bytes())
    ));
    let candidate = file_name(&video_stem, &labels, extension.as_deref());
    used.insert(candidate.clone());
    candidate
}

fn sidecar_labels(sidecar: &MediaFile, video: &PlannedVideo<'_>) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = BTreeSet::new();
    let mut languages = BTreeSet::new();

    if let Some(parent) = sidecar.relative.parent() {
        for component in parent.components() {
            let value = component.as_os_str().to_string_lossy();
            if is_generic_directory(&value)
                || episode_identity(Path::new(component.as_os_str())).is_some()
            {
                continue;
            }
            if let Some(label) = sanitize_label(&value) {
                insert_label(&mut labels, &mut seen, label);
            }
            for token in tokens(component.as_os_str()) {
                if let Some(language) = language_code(&token) {
                    languages.insert(language);
                }
            }
        }
    }

    for token in tokens(sidecar.relative.file_stem().unwrap_or_default()) {
        if video.tokens.contains(&token)
            || token.chars().all(|character| character.is_ascii_digit())
            || is_episode_token(&token)
            || is_noise_token(&token)
        {
            continue;
        }
        if let Some(language) = language_code(&token) {
            languages.insert(language);
            if token.len() > 3
                && let Some(label) = sanitize_label(&token)
            {
                insert_label(&mut labels, &mut seen, label);
            }
        } else if let Some(label) = sanitize_label(&token) {
            insert_label(&mut labels, &mut seen, label);
        }
    }

    for language in languages {
        insert_label(&mut labels, &mut seen, language.to_owned());
    }

    labels
}

fn insert_label(labels: &mut Vec<String>, seen: &mut BTreeSet<String>, label: String) {
    if seen.insert(label.to_ascii_lowercase()) {
        labels.push(label);
    }
}

fn file_name(stem: &str, labels: &[String], extension: Option<&str>) -> OsString {
    let mut name = stem.to_owned();
    for label in labels {
        name.push('.');
        name.push_str(label);
    }
    if let Some(extension) = extension {
        name.push('.');
        name.push_str(extension);
    }
    name.into()
}

pub(crate) fn normalized_stem(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    tokens(OsStr::new(&strip_bracketed(&stem))).join("")
}

fn meaningful_tokens(value: &OsStr) -> BTreeSet<String> {
    tokens(value)
        .into_iter()
        .filter(|token| !is_noise_token(token))
        .collect()
}

fn tokens(value: &OsStr) -> Vec<String> {
    value
        .to_string_lossy()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn strip_bracketed(value: &str) -> String {
    let mut depth = 0_u32;
    value
        .chars()
        .filter(|character| match character {
            '[' | '(' | '{' => {
                depth += 1;
                false
            }
            ']' | ')' | '}' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect()
}

pub(crate) fn episode_identity(path: &Path) -> Option<EpisodeIdentity> {
    let text = path.to_string_lossy().to_ascii_lowercase();
    let values = text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if let Some(identity) = compact_identity(text.as_bytes()) {
        return Some(identity);
    }

    let season = values.windows(2).find_map(|window| {
        (window[0] == "season")
            .then(|| window[1].parse::<u16>().ok())
            .flatten()
    });
    let explicit_episode = values.windows(2).find_map(|window| {
        matches!(window[0], "episode" | "ep" | "e")
            .then(|| window[1].parse::<u16>().ok())
            .flatten()
    });
    if let Some(episode) = explicit_episode {
        return Some(EpisodeIdentity { season, episode });
    }

    for value in values.iter().rev() {
        if let Some(episode) = versioned_episode(value) {
            return Some(EpisodeIdentity { season, episode });
        }
        if let Some(episode) = value
            .strip_prefix("ep")
            .or_else(|| value.strip_prefix('e'))
            .and_then(|value| value.parse().ok())
        {
            return Some(EpisodeIdentity { season, episode });
        }
    }

    let episode = values.iter().enumerate().rev().find_map(|(index, value)| {
        let number = value.parse::<u16>().ok()?;
        let is_season = index > 0 && values[index - 1] == "season";
        (!is_season && !is_noise_number(number)).then_some(number)
    })?;
    Some(EpisodeIdentity { season, episode })
}

fn compact_identity(bytes: &[u8]) -> Option<EpisodeIdentity> {
    for index in 0..bytes.len() {
        if bytes[index] == b's'
            && let Some((season, after_season)) = ascii_number(bytes, index + 1)
            && bytes.get(after_season) == Some(&b'e')
            && let Some((episode, _)) = ascii_number(bytes, after_season + 1)
        {
            return Some(EpisodeIdentity {
                season: Some(season),
                episode,
            });
        }

        if bytes[index].is_ascii_digit()
            && let Some((season, after_season)) = ascii_number(bytes, index)
            && bytes.get(after_season) == Some(&b'x')
            && let Some((episode, _)) = ascii_number(bytes, after_season + 1)
        {
            return Some(EpisodeIdentity {
                season: Some(season),
                episode,
            });
        }
    }
    None
}

fn ascii_number(bytes: &[u8], start: usize) -> Option<(u16, usize)> {
    let mut end = start;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    (end > start)
        .then(|| {
            std::str::from_utf8(&bytes[start..end])
                .ok()?
                .parse()
                .ok()
                .map(|n| (n, end))
        })
        .flatten()
}

fn versioned_episode(value: &str) -> Option<u16> {
    let separator = value.find('v')?;
    let episode = &value[..separator];
    let version = &value[separator + 1..];
    (!episode.is_empty()
        && episode.chars().all(|character| character.is_ascii_digit())
        && !version.is_empty()
        && version.chars().all(|character| character.is_ascii_digit()))
    .then(|| episode.parse().ok())
    .flatten()
}

fn is_noise_number(number: u16) -> bool {
    matches!(number, 264 | 265 | 480 | 576 | 720 | 1080 | 2160) || (1900..=2099).contains(&number)
}

fn is_episode_token(token: &str) -> bool {
    token == "episode"
        || token == "ep"
        || compact_identity(token.as_bytes()).is_some()
        || versioned_episode(token).is_some()
        || token
            .strip_prefix("ep")
            .or_else(|| token.strip_prefix('e'))
            .is_some_and(|value| value.chars().all(|character| character.is_ascii_digit()))
}

fn is_noise_token(token: &str) -> bool {
    matches!(
        token,
        "audio"
            | "audios"
            | "dub"
            | "dubs"
            | "sub"
            | "subs"
            | "subtitle"
            | "subtitles"
            | "track"
            | "tracks"
            | "voice"
            | "voices"
            | "webrip"
            | "webdl"
            | "bluray"
            | "bdrip"
            | "remux"
            | "x264"
            | "x265"
            | "h264"
            | "h265"
            | "hevc"
            | "av1"
            | "10bit"
            | "8bit"
            | "1080p"
            | "720p"
            | "2160p"
    )
}

fn language_code(token: &str) -> Option<&'static str> {
    match token {
        "en" | "eng" | "english" => Some("en"),
        "ru" | "rus" | "russian" => Some("ru"),
        "ja" | "jp" | "jpn" | "japanese" => Some("ja"),
        "de" | "deu" | "ger" | "german" => Some("de"),
        "fr" | "fra" | "fre" | "french" => Some("fr"),
        "es" | "spa" | "spanish" => Some("es"),
        "it" | "ita" | "italian" => Some("it"),
        "ko" | "kor" | "korean" => Some("ko"),
        "pl" | "pol" | "polish" => Some("pl"),
        "pt" | "por" | "portuguese" => Some("pt"),
        "uk" | "ukr" | "ukrainian" => Some("uk"),
        "zh" | "zho" | "chi" | "chinese" => Some("zh"),
        _ => None,
    }
}

fn is_generic_directory(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "audio"
            | "audios"
            | "dub"
            | "dubs"
            | "sound"
            | "sounds"
            | "sub"
            | "subs"
            | "subtitle"
            | "subtitles"
            | "track"
            | "tracks"
            | "voice"
            | "voices"
    )
}

fn sanitize_label(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut separator = false;
    for character in value.trim().chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_') {
            if separator && !result.is_empty() {
                result.push('_');
            }
            separator = false;
            result.push(character);
        } else {
            separator = true;
        }
    }
    (!result.is_empty()).then_some(result)
}

fn stable_hash(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_common_episode_identities() {
        assert_eq!(
            episode_identity(Path::new("Show S02E03.mkv")),
            Some(EpisodeIdentity {
                season: Some(2),
                episode: 3
            })
        );
        assert_eq!(
            episode_identity(Path::new("Season 2/Subs/03.ass")),
            Some(EpisodeIdentity {
                season: Some(2),
                episode: 3
            })
        );
        assert_eq!(
            episode_identity(Path::new("Show - 03v2 [1080p].mkv")),
            Some(EpisodeIdentity {
                season: None,
                episode: 3
            })
        );
    }

    #[test]
    fn matches_nested_sidecars_and_builds_jellyfin_names() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::Directory("Frieren".into()),
                files: vec![
                    file("[SubsPlease] Frieren - 01 (1080p).mkv", MediaKind::Video),
                    file("Subs/signs/Frieren 01.ass", MediaKind::Subtitle),
                    file("Audio/ru/01.mka", MediaKind::Audio),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert!(
            plan.links
                .contains_key(Path::new("Frieren/[SubsPlease] Frieren - 01 (1080p).mkv"))
        );
        assert!(plan.links.contains_key(Path::new(
            "Frieren/[SubsPlease] Frieren - 01 (1080p).signs.ass"
        )));
        assert!(plan.links.contains_key(Path::new(
            "Frieren/[SubsPlease] Frieren - 01 (1080p).ru.mka"
        )));
        assert_eq!(plan.stats.matched_sidecars, 2);
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn normalizes_language_aliases_from_real_sidecar_layouts() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::Directory("Show".into()),
                files: vec![
                    file("Show S01E01.mkv", MediaKind::Video),
                    file("JP Subs/Show S01E01.jp_netflix.srt", MediaKind::Subtitle),
                    file("RUS Sound [AniLiberty]/Show S01E01.mka", MediaKind::Audio),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert!(
            plan.links
                .contains_key(Path::new("Show/Show S01E01.JP_Subs.netflix.ja.srt"))
        );
        assert!(
            plan.links
                .contains_key(Path::new("Show/Show S01E01.RUS_Sound_AniLiberty.ru.mka"))
        );
    }

    #[test]
    fn refuses_ambiguous_episode_matches() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::Directory("Show".into()),
                files: vec![
                    file("Season 1/01.mkv", MediaKind::Video),
                    file("Season 2/01.mkv", MediaKind::Video),
                    file("Subs/01.ass", MediaKind::Subtitle),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert_eq!(plan.stats.skipped_sidecars, 1);
        assert_eq!(plan.skipped[0].reason, "ambiguous video match");
    }

    #[test]
    fn a_single_video_is_the_safe_fallback_for_a_movie() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::Directory("Movie".into()),
                files: vec![
                    file("Movie (2024).mkv", MediaKind::Video),
                    file("Subs/English Commentary.ass", MediaKind::Subtitle),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert!(
            plan.links
                .contains_key(Path::new("Movie/Movie (2024).english.commentary.en.ass"))
        );
    }

    #[test]
    fn standalone_videos_and_sidecars_are_wrapped_by_video_stem() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::PerVideoDirectory,
                files: vec![
                    file("Paprika.mkv", MediaKind::Video),
                    file("Paprika.en.srt", MediaKind::Subtitle),
                    file("Perfect Blue.mkv", MediaKind::Video),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert!(plan.links.contains_key(Path::new("Paprika/Paprika.mkv")));
        assert!(plan.links.contains_key(Path::new("Paprika/Paprika.en.srt")));
        assert!(
            plan.links
                .contains_key(Path::new("Perfect Blue/Perfect Blue.mkv"))
        );
    }

    #[test]
    fn sidecar_prefix_matching_prefers_the_longest_video_stem() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::PerVideoDirectory,
                files: vec![
                    file("Movie.mkv", MediaKind::Video),
                    file("Movie Extended.mkv", MediaKind::Video),
                    file("Movie Extended.en.srt", MediaKind::Subtitle),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert!(
            plan.links
                .contains_key(Path::new("Movie Extended/Movie Extended.en.srt"))
        );
        assert_eq!(
            plan.operations.last().unwrap().match_strategy,
            Some("video-basename-prefix")
        );
    }

    #[test]
    fn duplicate_flattened_video_names_are_disambiguated() {
        let scan = Scan {
            entries: vec![SourceEntry {
                target: EntryTarget::Directory("Show".into()),
                files: vec![
                    file("Season 1/01.mkv", MediaKind::Video),
                    file("Season 2/01.mkv", MediaKind::Video),
                ],
            }],
            ..Scan::default()
        };

        let plan = build_plan(&scan).unwrap();

        assert!(plan.links.contains_key(Path::new("Show/01.mkv")));
        assert!(plan.links.contains_key(Path::new("Show/01.Season_2.mkv")));
    }

    fn file(relative: &str, kind: MediaKind) -> MediaFile {
        MediaFile {
            source: PathBuf::from("/source").join(relative),
            relative: relative.into(),
            kind,
            size_bytes: 0,
            modified_unix_seconds: None,
            eligible: true,
        }
    }
}
