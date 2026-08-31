# media-projector

`media-projector` turns torrent-friendly directory trees into a flat or grouped,
[Jellyfin-compatible](https://jellyfin.org/docs/general/server/media/shows/#external-subtitles-and-audio-tracks)
media tree without copying media data or changing the torrent.

The supported deployment target is Linux, normally as the included Docker or
Podman container. macOS and Windows are not supported targets. This grouped
example uses `--group-by-directory`:

```text
/storage/torrents/media/Frieren/
├── [SubsPlease] Frieren - 01 (1080p).mkv
├── Audio/ru/01.mka
└── Subs/signs/Frieren 01.ass

                         hardlinks
                            │
                            ▼

/storage/jellyfin/Frieren/
├── [SubsPlease] Frieren - 01 (1080p).mkv
├── [SubsPlease] Frieren - 01 (1080p).ru.mka
└── [SubsPlease] Frieren - 01 (1080p).signs.ass
```

The projection uses additional directory entries for the same inodes. It does
not consume the combined size of the video, audio, and subtitle files again.

> [!NOTE]
> This project contains AI-generated code. See [AI_USAGE.md](AI_USAGE.md) for details.

## Contract

- The source is read-only authoritative data. It is never renamed or modified.
- By default, all recursively discovered media is flattened into the target
  root. This is intended for a movies instance.
- With `--group-by-directory`, each source top-level directory becomes one flat
  target directory. This is intended for a shows instance.
- Root-level source files remain at the target root in either layout.
- Videos and matched external audio/subtitle files are hardlinked; unrelated
  files are ignored.
- The target is disposable derived state and must be dedicated to this process.
- Source and target must be on the same Unix filesystem.
- Source symlinks are ignored and target symlinks are never followed.

On first use, the target must be absent or empty. The process writes a
`.media-projector` ownership marker. Later reconciliations remove target files
which are not in the desired projection. A non-empty unmarked directory is
refused rather than cleaned.

See [DESIGN.md](DESIGN.md) for matching, reconciliation, and safety details.

## Run

One movies reconciliation:

```bash
cargo run --release --package media-projector -- \
  --source /storage/torrents/movies \
  --target /storage/jellyfin/movies \
  --settle 0 \
  --once
```

One shows reconciliation:

```bash
cargo run --release --package media-projector -- \
  --source /storage/torrents/shows \
  --target /storage/jellyfin/shows \
  --group-by-directory \
  --settle 0 \
  --once
```

Daemon mode:

```bash
cargo run --release --package media-projector -- \
  --source /storage/torrents/shows \
  --target /storage/jellyfin/shows \
  --group-by-directory \
  --log-level info
```

The daemon performs an initial full reconciliation, watches the source
recursively using Linux `inotify`, ignores non-mutating access events, debounces
mutation bursts for three seconds, and performs a safety reconciliation every
five minutes. Files changed within the last 30 seconds are deferred so an active
download is not exposed to Jellyfin immediately.

All options have unprefixed environment equivalents. Run `media-projector
--help` for the complete list.

## Preflight diagnostics

To inspect a real library without creating or changing a projection, run the
binary from the source directory:

```bash
./media-projector --preflight-debug=./media-projector-preflight.json
```

Add `--group-by-directory` to preview the shows layout. The selected projection
layout is recorded in the report.

`--source` defaults to the current directory only in this mode. The report is
written atomically and the process exits. It contains:

- the recognized file hierarchy, logical sizes, modification times, and settle
  eligibility;
- normalized basenames and extracted season/episode identities;
- every proposed hardlink target and the sidecar match strategy used;
- every ambiguous or unmatched sidecar;
- ignored extension counts, symlink counts, and aggregate scan/plan statistics.

The scan reads directory entries and file metadata only. It does not open,
decode, or hash media contents, and it never creates a target or hardlinks. The
JSON contains real filenames, so treat it as potentially private when sharing
it.

## Docker

Build and run with Docker or compatible Podman commands:

```bash
./scripts/build-image.sh media-projector
docker run --rm \
  -v /storage:/storage \
  -e LOG_LEVEL=info \
  media-projector \
  --source /storage/torrents/movies \
  --target /storage/jellyfin/movies
```

Set `CONTAINER_ENGINE=podman` when building with Podman.

The shared parent mount is intentional: hardlinks cannot cross bind-mounted
filesystems. Jellyfin only needs the generated directory:

```yaml
services:
  movies-projector:
    image: ghcr.io/fopwoc/media-projector:latest
    restart: unless-stopped
    volumes:
      - /storage:/storage
    command:
      - --source
      - /storage/torrents/movies
      - --target
      - /storage/jellyfin/movies
    environment:
      LOG_LEVEL: info

  shows-projector:
    image: ghcr.io/fopwoc/media-projector:latest
    restart: unless-stopped
    volumes:
      - /storage:/storage
    command:
      - --source
      - /storage/torrents/shows
      - --target
      - /storage/jellyfin/shows
      - --group-by-directory
    environment:
      LOG_LEVEL: info

  jellyfin:
    image: jellyfin/jellyfin
    volumes:
      - /storage/jellyfin:/media:ro
```

For the cleanest completion boundary, configure the torrent client to download
incomplete data outside the watched source and move completed torrents into it.
The settle window remains useful for rename bursts and clients which download
directly into the source.

## Releases

Push a stable semantic-version tag such as `0.2.0` from the default branch. A
clean tagged commit uses the exact tag as its version. Untagged development
builds use `YYYYMMDD-short-commit-hash`. The identity is resolved once during
the build and embedded in binaries, containers, logs, and preflight reports.

## Recognized files

- Video: `mkv`, `mp4`, `m4v`, `avi`, `webm`, `mov`, `mpg`, `mpeg`, `ts`, `m2ts`
- Audio: `mka`, `aac`, `ac3`, `eac3`, `dts`, `dtshd`, `flac`, `mp3`, `ogg`, `opus`, `wav`
- Subtitles: `ass`, `ssa`, `srt`, `sub`, `idx`, `vtt`, `ttml`

## License and authorship

Licensed under the [WTFNMFPL](LICENSE). See [COPYRIGHT](COPYRIGHT).
