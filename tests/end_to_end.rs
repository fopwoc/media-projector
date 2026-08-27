use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use media_projector::{Config, LogLevel};

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

    media_projector::run(Config {
        source,
        target: target.clone(),
        debounce: Duration::from_secs(3),
        scan_interval: Duration::from_secs(300),
        settle: Duration::ZERO,
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
