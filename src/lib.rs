mod cli;
mod daemon;
mod logging;
mod matcher;
mod media;
mod preflight;
mod projection;

pub use cli::{Command, Config, HELP, PreflightConfig, parse_args};
pub use daemon::run;
pub use logging::{LogLevel, init_logging};
pub use preflight::run_preflight;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

pub(crate) fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}
