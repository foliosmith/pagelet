#![forbid(unsafe_code)]

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return ExitCode::SUCCESS;
    };

    match command.as_str() {
        "-h" | "--help" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        "golden" => print_command_help("golden", "check and update normalized golden files"),
        "corpus" => print_command_help("corpus", "run configured EPUB corpus profiles"),
        "bench" => print_command_help("bench", "run benchmark profiles and reports"),
        "release" => print_command_help("release", "verify and publish the pagelet crate"),
        "external" => print_command_help("external", "sync and verify external test tools"),
        other => {
            eprintln!("unknown xtask command: {other}");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_command_help(name: &str, summary: &str) -> ExitCode {
    println!("cargo xtask {name}");
    println!();
    println!("{summary}.");
    println!();
    println!("This command group is reserved for upcoming pagelet automation tasks.");
    ExitCode::SUCCESS
}

fn print_help() {
    println!("pagelet xtask");
    println!();
    println!("Usage:");
    println!("  cargo xtask <command> [options]");
    println!();
    println!("Commands:");
    println!("  golden    Check or update normalized golden files");
    println!("  corpus    Run configured EPUB corpus profiles");
    println!("  bench     Run benchmark profiles and reports");
    println!("  release   Verify and publish the pagelet crate");
    println!("  external  Sync and verify external test tools");
    println!("  help      Print this help text");
}
