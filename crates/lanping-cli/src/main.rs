//! LanPing on the command line, for scripts and cron.
//!
//! Output is a table for people or, with `--json`, JSON for scripts. The exit code says how
//! it went: 0 fine, 1 the command could not run to the end, 2 the command line was wrong,
//! and 3 the check ran and found something to act on.

mod args;
mod commands;
mod output;

use std::process::ExitCode;

use commands::{COMMANDS, Exit, Failure};

const EXIT_FAILED: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_PROBLEM: u8 = 3;

fn help() -> String {
    let mut text = String::from(
        "lanping-cli: the checks of LanPing for scripts and cron\n\nUsage: lanping-cli <command> [options]\n\nCommands:\n",
    );
    for info in &COMMANDS {
        text.push_str(&format!("  {:<10} {}\n", info.name, info.summary));
    }
    text.push_str("\nOptions of each command are shown with `lanping-cli <command> --help`.\n");
    text.push_str("\nExit codes: 0 fine, 1 could not run to the end, 2 wrong command line, 3 a check found a problem.\n");
    text
}

fn command_help(name: &str) -> Option<String> {
    COMMANDS
        .iter()
        .find(|info| info.name == name)
        .map(|info| format!("Usage: lanping-cli {}\n\n{}\n", info.usage, info.summary))
}

#[tokio::main]
async fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = arguments.split_first() else {
        print!("{}", help());
        return ExitCode::from(EXIT_USAGE);
    };
    match command.as_str() {
        "-h" | "--help" | "help" => {
            print!("{}", help());
            return ExitCode::SUCCESS;
        }
        "-V" | "--version" => {
            println!("lanping-cli {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    if rest.iter().any(|word| word == "-h" || word == "--help")
        && let Some(text) = command_help(command)
    {
        print!("{text}");
        return ExitCode::SUCCESS;
    }
    match commands::run(command, rest).await {
        Ok(Exit::Fine) => ExitCode::SUCCESS,
        Ok(Exit::Problem) => ExitCode::from(EXIT_PROBLEM),
        Err(Failure { message, usage }) => {
            eprintln!("lanping-cli: {message}");
            if usage {
                eprintln!("Try `lanping-cli --help`.");
                ExitCode::from(EXIT_USAGE)
            } else {
                ExitCode::from(EXIT_FAILED)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_lists_every_command_and_the_exit_codes() {
        let text = help();
        for info in &COMMANDS {
            assert!(text.contains(info.name), "{}", info.name);
        }
        assert!(text.contains("Exit codes"));
    }

    #[test]
    fn each_command_has_its_own_help_that_starts_with_its_name() {
        for info in &COMMANDS {
            let text = command_help(info.name).unwrap();
            assert!(
                text.starts_with(&format!("Usage: lanping-cli {}", info.name)),
                "{text}"
            );
        }
        assert!(command_help("nope").is_none());
    }
}
