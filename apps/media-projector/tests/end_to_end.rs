use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use media_projector::{Config, LogLevel, ProjectionLayout};

#[test]
fn one_shot_projects_a_nested_release() {
    let root = test_directory("end-to-end");
    let source = root.join("source");
    let target = root.join("target");
    let release = source.join("Frieren");
    fs::create_dir_all(release.join("Audio/ru")).unwrap();
    fs::create_dir_all(release.join("Subs/signs")).unwrap();
    let video = release.join("Frieren - 01.mkv");
    let audio = release.join("Audio/ru/01.mka");
    fs::write(&video, b"video").unwrap();
    fs::write(&audio, b"audio").unwrap();
    fs::write(release.join("Subs/signs/01.ass"), b"subtitle").unwrap();
    fs::create_dir_all(source.join("Dungeon Meshi")).unwrap();
    fs::write(
        source.join("Dungeon Meshi/Dungeon Meshi - 01.mkv"),
        b"another video",
    )
    .unwrap();

    media_projector::run(Config {
        source,
        target: target.clone(),
        debounce: Duration::from_secs(3),
        scan_interval: Duration::from_secs(300),
        settle: Duration::ZERO,
        layout: ProjectionLayout::GroupByDirectory,
        log_level: LogLevel::Error,
        once: true,
    })
    .unwrap();

    let projected_video = target.join("Frieren/Frieren - 01.mkv");
    let projected_audio = target.join("Frieren/Frieren - 01.ru.mka");
    let projected_subtitle = target.join("Frieren/Frieren - 01.signs.ass");
    assert!(projected_video.exists());
    assert!(projected_audio.exists());
    assert!(projected_subtitle.exists());
    assert!(target.join("Dungeon Meshi/Dungeon Meshi - 01.mkv").exists());
    assert_eq!(
        fs::metadata(video).unwrap().ino(),
        fs::metadata(projected_video).unwrap().ino()
    );
    assert_eq!(
        fs::metadata(audio).unwrap().ino(),
        fs::metadata(projected_audio).unwrap().ino()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn one_shot_flattens_multiple_movie_directories_into_target_root() {
    let root = test_directory("flat-end-to-end");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("Movie One")).unwrap();
    fs::create_dir_all(source.join("Movie Two")).unwrap();
    let first = source.join("Movie One/Movie One.mkv");
    let second = source.join("Movie Two/Movie Two.mkv");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();

    media_projector::run(Config {
        source,
        target: target.clone(),
        debounce: Duration::from_secs(3),
        scan_interval: Duration::from_secs(300),
        settle: Duration::ZERO,
        layout: ProjectionLayout::Flat,
        log_level: LogLevel::Error,
        once: true,
    })
    .unwrap();

    let projected_first = target.join("Movie One.mkv");
    let projected_second = target.join("Movie Two.mkv");
    assert!(projected_first.exists());
    assert!(projected_second.exists());
    assert_eq!(
        fs::metadata(first).unwrap().ino(),
        fs::metadata(projected_first).unwrap().ino()
    );
    assert_eq!(
        fs::metadata(second).unwrap().ino(),
        fs::metadata(projected_second).unwrap().ino()
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
