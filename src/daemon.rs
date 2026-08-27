use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use notify::event::{MetadataKind, ModifyKind};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::Result;
use crate::cli::Config;
use crate::logging;
use crate::matcher::build_plan;
use crate::media;
use crate::projection::{self, PreparedPaths};

pub fn run(config: Config) -> Result<()> {
    logging::info(
        "app",
        format_args!("media-projector {} starting", env!("CARGO_PKG_VERSION")),
    );
    logging::info(
        "app",
        format_args!(
            "source={}, target={}, settle={}s",
            config.source.display(),
            config.target.display(),
            config.settle.as_secs()
        ),
    );
    logging::info("projection", "validating source and target");
    let paths = projection::prepare(&config.source, &config.target)?;
    reconcile_once(&paths, config.settle)?;

    if config.once {
        logging::info("app", "one-shot reconciliation complete");
        return Ok(());
    }

    let (sender, receiver) = mpsc::channel();
    let watcher = create_watcher(&paths, sender)
        .map_err(|error| {
            logging::warn(
                "watcher",
                format_args!(
                    "native file watching is unavailable ({error}); periodic scans remain active"
                ),
            );
            error
        })
        .ok();
    if watcher.is_some() {
        logging::info(
            "watcher",
            format_args!("watching {} recursively", paths.source.display()),
        );
    }
    logging::info(
        "app",
        format_args!(
            "ready; debounce={}s, periodic scan={}s",
            config.debounce.as_secs(),
            config.scan_interval.as_secs()
        ),
    );

    let mut pending: Option<Instant> = None;
    let mut next_periodic = Instant::now() + config.scan_interval;
    let mut receiver_connected = watcher.is_some();

    loop {
        let now = Instant::now();
        let next_deadline = pending.map_or(next_periodic, |deadline| deadline.min(next_periodic));
        let timeout = next_deadline.saturating_duration_since(now);

        if receiver_connected {
            match receiver.recv_timeout(timeout) {
                Ok(Ok(event)) => {
                    if requires_reconciliation(&event) {
                        log_event(&event);
                        pending = Some(Instant::now() + config.debounce);
                    }
                }
                Ok(Err(error)) => {
                    logging::warn("watcher", format_args!("filesystem watcher error: {error}"));
                    pending = Some(Instant::now() + config.debounce);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    logging::warn(
                        "watcher",
                        "filesystem watcher stopped; periodic scans remain active",
                    );
                    receiver_connected = false;
                }
            }
        } else if !timeout.is_zero() {
            thread::sleep(timeout.min(Duration::from_secs(1)));
        }

        let now = Instant::now();
        let periodic_due = now >= next_periodic;
        let debounce_due = pending.is_some_and(|deadline| now >= deadline);
        if periodic_due || debounce_due {
            if let Err(error) = reconcile_once(&paths, config.settle) {
                logging::error("reconcile", format_args!("reconciliation failed: {error}"));
            }
            pending = None;
            next_periodic = Instant::now() + config.scan_interval;
        }
    }
}

fn create_watcher(
    paths: &PreparedPaths,
    sender: mpsc::Sender<notify::Result<Event>>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })?;
    watcher.watch(&paths.source, RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn reconcile_once(paths: &PreparedPaths, settle: Duration) -> Result<()> {
    logging::debug("reconcile", "starting full reconciliation");
    let scan = media::scan(&paths.source, settle)?;
    media::log_scan_stats(&scan.stats);
    let plan = build_plan(&scan)?;
    for skipped in &plan.skipped {
        logging::warn(
            "matcher",
            format_args!("skipped {}: {}", skipped.source.display(), skipped.reason),
        );
    }
    logging::debug(
        "matcher",
        format_args!(
            "planned {} videos and {} sidecars; skipped {} sidecars",
            plan.stats.videos, plan.stats.matched_sidecars, plan.stats.skipped_sidecars
        ),
    );

    let report = projection::reconcile(&paths.target, &plan.links)?;
    logging::info(
        "reconcile",
        format_args!(
            "complete: {} created, {} updated, {} unchanged, {} stale files and {} empty directories removed",
            report.created,
            report.updated,
            report.unchanged,
            report.removed_files,
            report.removed_directories
        ),
    );
    Ok(())
}

fn log_event(event: &Event) {
    logging::debug(
        "watcher",
        format_args!("event {:?}: {:?}", event.kind, event.paths),
    );
}

fn requires_reconciliation(event: &Event) -> bool {
    !matches!(
        event.kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, CreateKind};

    use super::*;

    #[test]
    fn scanner_access_events_do_not_trigger_feedback_reconciliation() {
        assert!(!requires_reconciliation(&Event::new(EventKind::Access(
            AccessKind::Any
        ))));
        assert!(!requires_reconciliation(&Event::new(EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::AccessTime)
        ))));
        assert!(requires_reconciliation(&Event::new(EventKind::Create(
            CreateKind::File
        ))));
    }
}
