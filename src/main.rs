use std::process::ExitCode;

use media_projector::{Command, init_logging, parse_args};

fn main() -> ExitCode {
    let environment = std::env::vars_os().collect::<Vec<_>>();
    let command = match parse_args(std::env::args_os().skip(1), &environment) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}\n\nRun media-projector --help for usage.");
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Help => {
            print!("{}", media_projector::HELP);
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("media-projector {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Preflight(config) => {
            init_logging(config.log_level);
            let output = config.output.clone();
            if let Err(error) = media_projector::run_preflight(config) {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            } else {
                println!("wrote preflight report to {}", output.display());
                ExitCode::SUCCESS
            }
        }
        Command::Run(config) => {
            init_logging(config.log_level);
            if let Err(error) = media_projector::run(config) {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
