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
        [command, rest @ ..] if command == "paginate" => {
            let paginate = parse_paginate_args(rest)?;
            match paginate.format {
                PaginateFormat::Json => {
                    let bytes = std::fs::read(&paginate.path).map_err(|error| error.to_string())?;
                    let json = pagelet::cli::paginate_bytes_json_with_options(
                        bytes,
                        pagelet::epub::OpenOptions::default(),
                        pagelet::cli::layout_options_from_px(paginate.width, paginate.height),
                    )
                    .map_err(|error| error.to_string())?;
                    print!("{json}");
                }
                PaginateFormat::Svg => {
                    let bytes = std::fs::read(&paginate.path).map_err(|error| error.to_string())?;
                    let svg = pagelet::cli::paginate_bytes_debug_svg_with_options(
                        bytes,
                        pagelet::epub::OpenOptions::default(),
                        pagelet::cli::layout_options_from_px(paginate.width, paginate.height),
                    )
                    .map_err(|error| error.to_string())?;
                    print!("{svg}");
                }
            }
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PaginateFormat {
    Json,
    Svg,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PaginateArgs {
    path: String,
    format: PaginateFormat,
    width: i64,
    height: i64,
}

fn parse_paginate_args(args: &[String]) -> Result<PaginateArgs, String> {
    let mut format = PaginateFormat::Json;
    let mut width = 360_i64;
    let mut height = 640_i64;
    let mut path = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--format requires json or svg".to_owned())?;
                format = match value.as_str() {
                    "json" => PaginateFormat::Json,
                    "svg" => PaginateFormat::Svg,
                    _ => return Err(format!("unsupported paginate format: {value}")),
                };
            }
            "--width" => {
                index += 1;
                width = args
                    .get(index)
                    .ok_or_else(|| "--width requires a value".to_owned())?
                    .parse()
                    .map_err(|_| "--width must be a whole pixel value".to_owned())?;
            }
            "--height" => {
                index += 1;
                height = args
                    .get(index)
                    .ok_or_else(|| "--height requires a value".to_owned())?
                    .parse()
                    .map_err(|_| "--height must be a whole pixel value".to_owned())?;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown paginate option: {value}"));
            }
            value => {
                if path.replace(value.to_owned()).is_some() {
                    return Err("paginate accepts exactly one EPUB path".to_owned());
                }
            }
        }
        index += 1;
    }

    let path = path.ok_or_else(|| "paginate requires an EPUB path".to_owned())?;
    if width <= 0 || height <= 0 {
        return Err("paginate width and height must be positive".to_owned());
    }
    Ok(PaginateArgs {
        path,
        format,
        width,
        height,
    })
}

fn print_help() {
    println!("pagelet");
    println!();
    println!("Usage:");
    println!("  pagelet inspect <book.epub>");
    println!("  pagelet paginate [--format json|svg] [--width px] [--height px] <book.epub>");
}
