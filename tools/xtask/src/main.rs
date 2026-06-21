#![forbid(unsafe_code)]

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use pagelet_testkit::{FixtureKind, GoldenDocument, GoldenSectionName, ValidEpubBuilder};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), XtaskError> {
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(());
    };

    match command {
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        "golden" => run_golden(&args[1..]),
        "corpus" => run_corpus(&args[1..]),
        "manifests" => run_manifests(&args[1..]),
        "bench" => print_command_help("bench", "run benchmark profiles and reports"),
        "release" => print_command_help("release", "verify and publish the pagelet crate"),
        "external" => print_command_help("external", "sync and verify external test tools"),
        other => Err(XtaskError::Usage(format!("unknown xtask command: {other}"))),
    }
}

fn run_golden(args: &[String]) -> Result<(), XtaskError> {
    let Some(action) = args.first().map(String::as_str) else {
        print_golden_help();
        return Ok(());
    };

    match action {
        "check" => golden_check(&parse_golden_selection(&args[1..])?),
        "update" => golden_update(&parse_golden_update(&args[1..])?),
        "-h" | "--help" | "help" => {
            print_golden_help();
            Ok(())
        }
        other => Err(XtaskError::Usage(format!(
            "unknown golden command: {other}"
        ))),
    }
}

fn parse_golden_selection(args: &[String]) -> Result<GoldenSelection, XtaskError> {
    let mut selected = GoldenSelection::All;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => selected = GoldenSelection::All,
            "--case" => {
                index += 1;
                let Some(case) = args.get(index) else {
                    return Err(XtaskError::Usage("--case requires a value".into()));
                };
                selected = GoldenSelection::One(case.clone());
            }
            other => {
                return Err(XtaskError::Usage(format!(
                    "unknown golden selection option: {other}"
                )));
            }
        }
        index += 1;
    }
    Ok(selected)
}

fn parse_golden_update(args: &[String]) -> Result<GoldenUpdate, XtaskError> {
    let mut selection = GoldenSelection::All;
    let mut reason = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--all" => selection = GoldenSelection::All,
            "--case" => {
                index += 1;
                let Some(case) = args.get(index) else {
                    return Err(XtaskError::Usage("--case requires a value".into()));
                };
                selection = GoldenSelection::One(case.clone());
            }
            "--reason" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(XtaskError::Usage("--reason requires a value".into()));
                };
                reason = Some(value.clone());
            }
            other => {
                return Err(XtaskError::Usage(format!(
                    "unknown golden update option: {other}"
                )));
            }
        }
        index += 1;
    }

    if selection == GoldenSelection::All && reason.as_deref().unwrap_or("").is_empty() {
        return Err(XtaskError::Usage(
            "golden update --all requires --reason <msg>".into(),
        ));
    }

    Ok(GoldenUpdate { selection, reason })
}

fn golden_check(selection: &GoldenSelection) -> Result<(), XtaskError> {
    if env::var_os("PAGELET_GOLDEN_UPDATE").is_some() {
        return Err(XtaskError::Command(
            "PAGELET_GOLDEN_UPDATE is not allowed during golden check".into(),
        ));
    }

    let cases = selected_golden_cases(selection)?;
    let mut failures = Vec::new();
    for case in cases {
        let path = golden_path(case.name);
        let actual = fs::read_to_string(&path).unwrap_or_default();
        if actual != case.contents {
            failures.push(format!(
                "{}\n{}",
                path.display(),
                first_diff(&actual, &case.contents)
            ));
        }
    }

    if failures.is_empty() {
        println!("golden check passed");
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "golden check failed:\n{}",
            failures.join("\n\n")
        )))
    }
}

fn golden_update(update: &GoldenUpdate) -> Result<(), XtaskError> {
    let cases = selected_golden_cases(&update.selection)?;
    let reason = update.reason.as_deref().unwrap_or("single-case update");
    for case in cases {
        let path = golden_path(case.name);
        atomic_write(&path, case.contents.as_bytes())?;
        println!("updated {} ({reason})", path.display());
    }
    Ok(())
}

fn selected_golden_cases(selection: &GoldenSelection) -> Result<Vec<GoldenCase>, XtaskError> {
    let all = built_in_golden_cases();
    match selection {
        GoldenSelection::All => Ok(all),
        GoldenSelection::One(name) => all
            .into_iter()
            .find(|case| case.name == name)
            .map(|case| vec![case])
            .ok_or_else(|| XtaskError::Usage(format!("unknown golden case: {name}"))),
    }
}

fn built_in_golden_cases() -> Vec<GoldenCase> {
    let fixture = ValidEpubBuilder::preset(FixtureKind::MinimalEpub3).build();
    let contents = GoldenDocument::empty()
        .entry(GoldenSectionName::BookSummary, "id", fixture.id.clone())
        .entry(
            GoldenSectionName::Manifest,
            "entry_count",
            fixture.entries.len().to_string(),
        )
        .entry(GoldenSectionName::Manifest, "package", "EPUB/package.opf")
        .entry(GoldenSectionName::Navigation, "nav", "EPUB/nav.xhtml")
        .to_json();

    vec![GoldenCase {
        name: "minimal-epub3",
        contents,
    }]
}

fn golden_path(name: &str) -> PathBuf {
    Path::new("tests")
        .join("golden")
        .join(format!("{name}.golden.json"))
}

fn first_diff(actual: &str, expected: &str) -> String {
    if actual.is_empty() {
        return "actual file is missing or empty".into();
    }
    for (index, (left, right)) in actual.lines().zip(expected.lines()).enumerate() {
        if left != right {
            return format!(
                "line {} differs\nactual:   {left}\nexpected: {right}",
                index + 1
            );
        }
    }
    format!(
        "line count differs: actual {}, expected {}",
        actual.lines().count(),
        expected.lines().count()
    )
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), XtaskError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn run_corpus(args: &[String]) -> Result<(), XtaskError> {
    let mut profile = "smoke".to_owned();
    let mut required = env_flag("PAGELET_CORPUS_REQUIRED");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(XtaskError::Usage("--profile requires a value".into()));
                };
                validate_corpus_profile(value)?;
                profile = value.clone();
            }
            "--required" => required = true,
            "-h" | "--help" | "help" => {
                print_corpus_help();
                return Ok(());
            }
            other => {
                return Err(XtaskError::Usage(format!("unknown corpus option: {other}")));
            }
        }
        index += 1;
    }

    validate_corpus_profile(&profile)?;
    let root = env::var_os("PAGELET_CORPUS_ROOT").map(PathBuf::from);
    let manifest = env::var_os("PAGELET_CORPUS_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/corpus-manifest.toml"));

    let Some(root) = root else {
        if required {
            return Err(XtaskError::Command(
                "PAGELET_CORPUS_ROOT is required but not set".into(),
            ));
        }
        println!("corpus profile={profile} skipped: PAGELET_CORPUS_ROOT not set");
        return Ok(());
    };

    if !root.exists() {
        return Err(XtaskError::Command(format!(
            "corpus root does not exist: {}",
            root.display()
        )));
    }
    let manifest_text = fs::read_to_string(&manifest)?;
    let case_count = manifest_text.matches("[[books]]").count();
    println!(
        "corpus profile={profile} root={} manifest={} cases={case_count}",
        root.display(),
        manifest.display()
    );
    Ok(())
}

fn validate_corpus_profile(profile: &str) -> Result<(), XtaskError> {
    match profile {
        "smoke" | "full" | "robustness" | "locale" | "regression" => Ok(()),
        other => Err(XtaskError::Usage(format!(
            "unknown corpus profile: {other}"
        ))),
    }
}

fn run_manifests(args: &[String]) -> Result<(), XtaskError> {
    match args {
        [command] if command == "lint" => manifest_lint(),
        [command] if matches!(command.as_str(), "-h" | "--help" | "help") => {
            print_manifests_help();
            Ok(())
        }
        [] => {
            print_manifests_help();
            Ok(())
        }
        [other, ..] => Err(XtaskError::Usage(format!(
            "unknown manifests command: {other}"
        ))),
    }
}

fn manifest_lint() -> Result<(), XtaskError> {
    let files = [
        "tests/corpus-manifest.toml",
        "tests/spec/requirements.toml",
        "tests/spec/support-matrix.toml",
        "tests/spec/dart-compatibility.toml",
    ];
    for file in files {
        let text = fs::read_to_string(file)?;
        require_schema_version(file, &text)?;
    }
    validate_quoted_values(
        "tests/corpus-manifest.toml",
        &fs::read_to_string("tests/corpus-manifest.toml")?,
        "expected",
        &["valid", "warning", "salvage", "invalid"],
    )?;
    validate_quoted_values(
        "tests/corpus-manifest.toml",
        &fs::read_to_string("tests/corpus-manifest.toml")?,
        "license",
        &[
            "public-domain",
            "redistributable",
            "private-ci",
            "generated",
        ],
    )?;
    validate_quoted_values(
        "tests/spec/support-matrix.toml",
        &fs::read_to_string("tests/spec/support-matrix.toml")?,
        "status",
        &[
            "Supported",
            "SupportedWithLimitations",
            "ParsedNotRendered",
            "UnsupportedDiagnosed",
            "RejectedForSecurity",
        ],
    )?;
    println!("manifest lint passed");
    Ok(())
}

fn require_schema_version(file: &str, text: &str) -> Result<(), XtaskError> {
    if text.lines().any(|line| line.trim() == "schema_version = 1") {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "{file} must contain schema_version = 1"
        )))
    }
}

fn validate_quoted_values(
    file: &str,
    text: &str,
    key: &str,
    allowed: &[&str],
) -> Result<(), XtaskError> {
    let prefix = format!("{key} = ");
    for line in text.lines().map(str::trim) {
        let Some(value) = line.strip_prefix(&prefix) else {
            continue;
        };
        let Some(value) = value
            .trim()
            .strip_prefix('"')
            .and_then(|v| v.split('"').next())
        else {
            return Err(XtaskError::Command(format!(
                "{file} has invalid quoted value for {key}"
            )));
        };
        if !allowed.contains(&value) {
            return Err(XtaskError::Command(format!(
                "{file} has invalid {key} value: {value}"
            )));
        }
    }
    Ok(())
}

fn env_flag(name: &str) -> bool {
    matches!(
        env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn print_command_help(name: &str, summary: &str) -> Result<(), XtaskError> {
    println!("cargo xtask {name}");
    println!();
    println!("{summary}.");
    println!();
    println!("This command group is reserved for upcoming pagelet automation tasks.");
    Ok(())
}

fn print_help() {
    println!("pagelet xtask");
    println!();
    println!("Usage:");
    println!("  cargo xtask <command> [options]");
    println!();
    println!("Commands:");
    println!("  golden     Check or update normalized golden files");
    println!("  corpus     Run configured EPUB corpus profiles");
    println!("  manifests  Lint checked-in test manifests");
    println!("  bench      Run benchmark profiles and reports");
    println!("  release    Verify and publish the pagelet crate");
    println!("  external   Sync and verify external test tools");
    println!("  help       Print this help text");
}

fn print_golden_help() {
    println!("Usage:");
    println!("  cargo xtask golden check [--case <name>|--all]");
    println!("  cargo xtask golden update --case <name>");
    println!("  cargo xtask golden update --all --reason <msg>");
}

fn print_corpus_help() {
    println!("Usage:");
    println!("  cargo xtask corpus --profile smoke|full|robustness|locale|regression [--required]");
}

fn print_manifests_help() {
    println!("Usage:");
    println!("  cargo xtask manifests lint");
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum GoldenSelection {
    All,
    One(String),
}

#[derive(Debug, Clone)]
struct GoldenUpdate {
    selection: GoldenSelection,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct GoldenCase {
    name: &'static str,
    contents: String,
}

#[derive(Debug)]
enum XtaskError {
    Usage(String),
    Command(String),
    Io(io::Error),
}

impl From<io::Error> for XtaskError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl std::fmt::Display for XtaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) | Self::Command(message) => f.write_str(message),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for XtaskError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_profiles_are_validated() {
        assert!(validate_corpus_profile("smoke").is_ok());
        assert!(validate_corpus_profile("bogus").is_err());
    }

    #[test]
    fn built_in_golden_case_is_stable() {
        let cases = built_in_golden_cases();

        assert_eq!(cases[0].name, "minimal-epub3");
        assert!(cases[0].contents.contains("book_summary"));
        assert_eq!(
            GoldenDocument::from_json(&cases[0].contents)
                .expect("parse")
                .to_json(),
            cases[0].contents
        );
    }
}
