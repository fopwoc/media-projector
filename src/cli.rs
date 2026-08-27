use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::logging::LogLevel;
use crate::{Result, invalid_input};

pub const HELP: &str = r#"media-projector 0.1.0

Project a recursive media tree into a flat, Jellyfin-friendly hardlink tree.

USAGE:
    media-projector --source PATH --target PATH [OPTIONS]
    media-projector [--source PATH] --preflight-debug PATH

OPTIONS:
    --source PATH          Source media directory (env: MEDIA_PROJECTOR_SOURCE)
    --target PATH          Owned projection directory (env: MEDIA_PROJECTOR_TARGET)
    --debounce SECONDS     Quiet time before an event-triggered scan [default: 3]
    --scan-interval SEC    Periodic full scan interval [default: 300]
    --settle SECONDS       Ignore files modified more recently than this [default: 30]
    --log-level LEVEL      debug, info, warn, or error [default: error]
    --preflight-debug PATH Write a non-mutating JSON scan/plan report and exit
    --once                 Reconcile once and exit
    -h, --help             Print help
    -V, --version          Print version

Environment variables use the same names prefixed with MEDIA_PROJECTOR_, for
example MEDIA_PROJECTOR_SCAN_INTERVAL and MEDIA_PROJECTOR_LOG_LEVEL.
"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub source: PathBuf,
    pub target: PathBuf,
    pub debounce: Duration,
    pub scan_interval: Duration,
    pub settle: Duration,
    pub log_level: LogLevel,
    pub once: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreflightConfig {
    pub source: PathBuf,
    pub output: PathBuf,
    pub settle: Duration,
    pub log_level: LogLevel,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Version,
    Preflight(PreflightConfig),
    Run(Config),
}

pub fn parse_args(
    args: impl IntoIterator<Item = OsString>,
    environment: &[(OsString, OsString)],
) -> Result<Command> {
    let environment: HashMap<&OsStr, &OsStr> = environment
        .iter()
        .map(|(name, value)| (name.as_os_str(), value.as_os_str()))
        .collect();

    let mut source = env_path(&environment, "MEDIA_PROJECTOR_SOURCE");
    let mut target = env_path(&environment, "MEDIA_PROJECTOR_TARGET");
    let mut debounce = env_seconds(&environment, "MEDIA_PROJECTOR_DEBOUNCE")?.unwrap_or(3);
    let mut scan_interval =
        env_seconds(&environment, "MEDIA_PROJECTOR_SCAN_INTERVAL")?.unwrap_or(300);
    let mut settle = env_seconds(&environment, "MEDIA_PROJECTOR_SETTLE")?.unwrap_or(30);
    let mut log_level = environment
        .get(OsStr::new("MEDIA_PROJECTOR_LOG_LEVEL"))
        .map(|value| parse_utf8("MEDIA_PROJECTOR_LOG_LEVEL", value))
        .transpose()?
        .map(LogLevel::from_str)
        .transpose()?
        .unwrap_or(LogLevel::Error);
    let mut once = false;
    let mut preflight_output = None;

    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if let Some(value) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("--preflight-debug="))
        {
            if value.is_empty() {
                return Err(invalid_input("--preflight-debug requires a path").into());
            }
            preflight_output = Some(PathBuf::from(value));
            continue;
        }
        match argument.to_str() {
            Some("-h" | "--help") => return Ok(Command::Help),
            Some("-V" | "--version") => return Ok(Command::Version),
            Some("--source") => source = Some(PathBuf::from(next_value(&mut args, "--source")?)),
            Some("--target") => target = Some(PathBuf::from(next_value(&mut args, "--target")?)),
            Some("--debounce") => {
                debounce = parse_seconds("--debounce", &next_value(&mut args, "--debounce")?)?
            }
            Some("--scan-interval") => {
                scan_interval = parse_seconds(
                    "--scan-interval",
                    &next_value(&mut args, "--scan-interval")?,
                )?
            }
            Some("--settle") => {
                settle = parse_seconds("--settle", &next_value(&mut args, "--settle")?)?
            }
            Some("--log-level") => {
                let value = next_value(&mut args, "--log-level")?;
                log_level = LogLevel::from_str(parse_utf8("--log-level", &value)?)?;
            }
            Some("--preflight-debug") => {
                preflight_output = Some(PathBuf::from(next_value(&mut args, "--preflight-debug")?));
            }
            Some("--once") => once = true,
            Some(value) if value.starts_with('-') => {
                return Err(invalid_input(format!("unknown option {value:?}")).into());
            }
            Some(value) => {
                return Err(
                    invalid_input(format!("unexpected positional argument {value:?}")).into(),
                );
            }
            None => return Err(invalid_input("arguments must be valid UTF-8 option names").into()),
        }
    }

    if scan_interval == 0 {
        return Err(invalid_input("--scan-interval must be greater than zero").into());
    }

    if let Some(output) = preflight_output {
        return Ok(Command::Preflight(PreflightConfig {
            source: source.unwrap_or(std::env::current_dir()?),
            output,
            settle: Duration::from_secs(settle),
            log_level,
        }));
    }

    Ok(Command::Run(Config {
        source: source.ok_or_else(|| invalid_input("--source is required"))?,
        target: target.ok_or_else(|| invalid_input("--target is required"))?,
        debounce: Duration::from_secs(debounce),
        scan_interval: Duration::from_secs(scan_interval),
        settle: Duration::from_secs(settle),
        log_level,
        once,
    }))
}

fn env_path(environment: &HashMap<&OsStr, &OsStr>, name: &str) -> Option<PathBuf> {
    environment.get(OsStr::new(name)).map(PathBuf::from)
}

fn env_seconds(environment: &HashMap<&OsStr, &OsStr>, name: &str) -> Result<Option<u64>> {
    environment
        .get(OsStr::new(name))
        .map(|value| parse_seconds(name, value))
        .transpose()
}

fn next_value(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<OsString> {
    args.next()
        .ok_or_else(|| invalid_input(format!("{option} requires a value")).into())
}

fn parse_seconds(name: &str, value: &OsStr) -> Result<u64> {
    let value = parse_utf8(name, value)?;
    value
        .parse()
        .map_err(|_| invalid_input(format!("{name} must be a non-negative integer")))
        .map_err(Into::into)
}

fn parse_utf8<'a>(name: &str, value: &'a OsStr) -> Result<&'a str> {
    value
        .to_str()
        .ok_or_else(|| invalid_input(format!("{name} must be valid UTF-8")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_values_override_environment() {
        let environment = vec![
            ("MEDIA_PROJECTOR_SOURCE".into(), "/env/source".into()),
            ("MEDIA_PROJECTOR_TARGET".into(), "/env/target".into()),
            ("MEDIA_PROJECTOR_LOG_LEVEL".into(), "warn".into()),
        ];
        let command = parse_args(
            [
                "--source".into(),
                "/cli/source".into(),
                "--settle".into(),
                "0".into(),
                "--once".into(),
            ],
            &environment,
        )
        .unwrap();

        let Command::Run(config) = command else {
            panic!("expected run command");
        };
        assert_eq!(config.source, PathBuf::from("/cli/source"));
        assert_eq!(config.target, PathBuf::from("/env/target"));
        assert_eq!(config.settle, Duration::ZERO);
        assert_eq!(config.log_level, LogLevel::Warn);
        assert!(config.once);
    }

    #[test]
    fn source_and_target_are_required() {
        let error = parse_args(Vec::<OsString>::new(), &[]).unwrap_err();
        assert_eq!(error.to_string(), "--source is required");
    }

    #[test]
    fn scan_interval_cannot_be_disabled() {
        let error = parse_args(
            [
                "--source".into(),
                "source".into(),
                "--target".into(),
                "target".into(),
                "--scan-interval".into(),
                "0".into(),
            ],
            &[],
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "--scan-interval must be greater than zero"
        );
    }

    #[test]
    fn preflight_defaults_source_to_current_directory_and_needs_no_target() {
        let command = parse_args(["--preflight-debug=out.json".into()], &[]).unwrap();
        let Command::Preflight(config) = command else {
            panic!("expected preflight command");
        };
        assert_eq!(config.source, std::env::current_dir().unwrap());
        assert_eq!(config.output, PathBuf::from("out.json"));
    }
}
