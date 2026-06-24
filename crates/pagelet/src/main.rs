#![forbid(unsafe_code)]

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [command, path] if command == "inspect" => {
            let json = pagelet::cli::inspect_path_json(path).map_err(|error| error.to_string())?;
            print!("{json}");
            Ok(())
        }
        [command] if matches!(command.as_str(), "-h" | "--help" | "help") => {
            print_help();
            Ok(())
        }
        [] => {
            print_help();
            Ok(())
        }
        [command, ..] => Err(format!("unknown pagelet command: {command}")),
    }
}

fn print_help() {
    println!("pagelet");
    println!();
    println!("Usage:");
    println!("  pagelet inspect <book.epub>");
}
