use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::invalid_input;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

impl FromStr for LogLevel {
    type Err = std::io::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(invalid_input(format!(
                "invalid log level {value:?}; expected debug, info, warn, or error"
            ))),
        }
    }
}

static LEVEL: OnceLock<LogLevel> = OnceLock::new();

pub fn init_logging(level: LogLevel) {
    let _ = LEVEL.set(level);
}

pub fn debug(category: &str, message: impl fmt::Display) {
    write(LogLevel::Debug, category, message);
}

pub fn info(category: &str, message: impl fmt::Display) {
    write(LogLevel::Info, category, message);
}

pub fn warn(category: &str, message: impl fmt::Display) {
    write(LogLevel::Warn, category, message);
}

pub fn error(category: &str, message: impl fmt::Display) {
    write(LogLevel::Error, category, message);
}

fn write(level: LogLevel, category: &str, message: impl fmt::Display) {
    if level < *LEVEL.get_or_init(|| LogLevel::Error) {
        return;
    }

    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    eprintln!(
        "{}.{:03} {:<5} [{}] {}",
        elapsed.as_secs(),
        elapsed.subsec_millis(),
        level.label(),
        category,
        message
    );
}
