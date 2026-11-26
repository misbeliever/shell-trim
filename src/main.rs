mod audit;
mod cargo_cmd;
mod cli;
mod config;
mod doctor;
mod export;
mod filter;
mod fs_cmd;
mod git_cmd;
mod install;
mod lang;
mod outline;
mod read;
mod search;
mod style;
mod swift;
mod telemetry;
mod tui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Ok(cli::Command::Help) => {
            print!("{}", cli::HELP_TEXT);
            ExitCode::SUCCESS
        }
        Ok(cli::Command::Version) => {
            println!("trm {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(cli::Command::Read(opts)) => match read::run(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm read: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Search(opts)) => match search::run(&opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("trm rg: {e}");
                ExitCode::from(2)
            }
        },
        Ok(cli::Command::Swift(bin, opts)) => match swift::run(bin, &opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("trm {bin}: {e}");
                ExitCode::from(2)
            }
        },
        Ok(cli::Command::Cargo(opts)) => match cargo_cmd::run(&opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("trm cargo: {e}");
                ExitCode::from(2)
            }
        },
        Ok(cli::Command::Git(opts)) => match git_cmd::run(&opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("trm git: {e}");
                ExitCode::from(2)
            }
        },
        Ok(cli::Command::Ls(opts)) => match fs_cmd::run_ls(&opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("trm ls: {e}");
                ExitCode::from(2)
            }
        },
        Ok(cli::Command::Find(opts)) => match fs_cmd::run_find(&opts) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("trm find: {e}");
                ExitCode::from(2)
            }
        },
        Ok(cli::Command::Ui(opts)) => match tui::run(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm ui: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Stats(opts)) => match telemetry::run_stats(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm stats: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Audit(opts)) => match audit::run_audit(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm audit: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Export(opts)) => match export::run(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm export: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Install(opts)) => match install::run_install(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm install: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Uninstall(opts)) => match install::run_uninstall(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm uninstall: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::Doctor) => match doctor::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("trm doctor: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("trm: {e}\nRun 'trm --help' for usage.");
            ExitCode::from(2)
        }
    }
}
