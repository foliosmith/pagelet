#![forbid(unsafe_code)]

use std::{
    env, fs,
    hint::black_box,
    io,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
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
        "bench" => run_bench(&args[1..]),
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
        "perf/performance-budgets.toml",
    ];
    for file in files {
        let text = fs::read_to_string(file)?;
        require_schema_version(file, &text)?;
    }
    read_perf_budget_manifest(Path::new("perf/performance-budgets.toml"))?;
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

fn run_bench(args: &[String]) -> Result<(), XtaskError> {
    if matches!(
        args.first().map(String::as_str),
        Some("-h" | "--help" | "help")
    ) {
        print_bench_help();
        return Ok(());
    }
    let options = parse_bench_options(args)?;
    let manifest = read_perf_budget_manifest(Path::new("perf/performance-budgets.toml"))?;
    let cases = bench_cases_for_profile(&options.profile)?;

    println!(
        "bench profile={} runner={} generated_cases={}",
        options.profile,
        manifest.runner.id,
        cases.len()
    );
    println!(
        "case,iterations,total_bytes,elapsed_ns,ns_per_iter,absolute_p95_ms,relative_regression_pct,minimum_effect_ms,policy"
    );

    let mut failures = Vec::new();
    for case in cases {
        let case_name = case.name();
        let budget = manifest.budget_for_case(&case_name).ok_or_else(|| {
            XtaskError::Command(format!(
                "perf/performance-budgets.toml is missing budget for {}",
                case_name
            ))
        })?;
        let row = measure_generated_fixture(case, options.iterations);
        if budget.policy == BudgetPolicy::Block
            && row.ns_per_iter() > u128::from(budget.absolute_p95_ms) * 1_000_000
        {
            failures.push(format!(
                "{} exceeded absolute budget: {}ns > {}ms",
                case_name,
                row.ns_per_iter(),
                budget.absolute_p95_ms
            ));
        }
        println!(
            "{},{},{},{},{},{},{},{},{}",
            case_name,
            row.iterations,
            row.total_bytes,
            row.elapsed.as_nanos(),
            row.ns_per_iter(),
            budget.absolute_p95_ms,
            budget.relative_regression_pct,
            budget.minimum_effect_ms,
            budget.policy.as_str()
        );
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "bench profile={} failed:\n{}",
            options.profile,
            failures.join("\n")
        )))
    }
}

fn parse_bench_options(args: &[String]) -> Result<BenchOptions, XtaskError> {
    let mut profile = "smoke".to_owned();
    let mut iterations = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(XtaskError::Usage("--profile requires a value".into()));
                };
                validate_bench_profile(value)?;
                profile = value.clone();
            }
            "--iterations" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(XtaskError::Usage("--iterations requires a value".into()));
                };
                let parsed = value.parse::<u32>().map_err(|_| {
                    XtaskError::Usage("--iterations requires a positive integer".into())
                })?;
                if parsed == 0 {
                    return Err(XtaskError::Usage(
                        "--iterations requires a positive integer".into(),
                    ));
                }
                iterations = Some(parsed);
            }
            "-h" | "--help" | "help" => {
                print_bench_help();
                return Ok(BenchOptions {
                    profile,
                    iterations: 1,
                });
            }
            other => {
                return Err(XtaskError::Usage(format!("unknown bench option: {other}")));
            }
        }
        index += 1;
    }

    validate_bench_profile(&profile)?;
    let iterations = iterations.unwrap_or_else(|| default_bench_iterations(&profile));
    Ok(BenchOptions {
        profile,
        iterations,
    })
}

fn validate_bench_profile(profile: &str) -> Result<(), XtaskError> {
    match profile {
        "smoke" | "full" => Ok(()),
        other => Err(XtaskError::Usage(format!("unknown bench profile: {other}"))),
    }
}

fn default_bench_iterations(profile: &str) -> u32 {
    match profile {
        "smoke" => 64,
        "full" => 512,
        _ => 64,
    }
}

fn bench_cases_for_profile(profile: &str) -> Result<Vec<BenchCase>, XtaskError> {
    validate_bench_profile(profile)?;
    let kinds: &[FixtureKind] = match profile {
        "smoke" => &[
            FixtureKind::MinimalEpub3,
            FixtureKind::Epub2WithNcx,
            FixtureKind::CssCascade,
            FixtureKind::Rtl,
        ],
        "full" => &FixtureKind::ALL,
        _ => unreachable!("bench profile already validated"),
    };
    Ok(kinds
        .iter()
        .copied()
        .map(|kind| BenchCase { kind })
        .collect())
}

fn measure_generated_fixture(case: BenchCase, iterations: u32) -> BenchRow {
    let warmup = ValidEpubBuilder::preset(case.kind).build();
    black_box(&warmup);

    let started = Instant::now();
    let mut total_bytes = 0_usize;
    for _ in 0..iterations {
        let fixture = ValidEpubBuilder::preset(case.kind).build();
        total_bytes = total_bytes.wrapping_add(fixture.bytes().len());
        black_box(&fixture);
    }
    let elapsed = started.elapsed();

    BenchRow {
        iterations,
        total_bytes,
        elapsed,
    }
}

fn read_perf_budget_manifest(path: &Path) -> Result<PerfBudgetManifest, XtaskError> {
    let text = fs::read_to_string(path)?;
    parse_perf_budget_manifest(path, &text)
}

fn parse_perf_budget_manifest(path: &Path, text: &str) -> Result<PerfBudgetManifest, XtaskError> {
    require_schema_version(&path.display().to_string(), text)?;
    let mut section = PerfSection::Root;
    let mut runner = RunnerFingerprintDraft::default();
    let mut current_budget: Option<PerfBudgetDraft> = None;
    let mut budgets = Vec::new();

    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        match line {
            "[runner_fingerprint]" => {
                push_budget(path, current_budget.take(), &mut budgets)?;
                section = PerfSection::Runner;
                continue;
            }
            "[[budgets]]" => {
                push_budget(path, current_budget.take(), &mut budgets)?;
                current_budget = Some(PerfBudgetDraft::default());
                section = PerfSection::Budget;
                continue;
            }
            _ => {}
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(XtaskError::Command(format!(
                "{}:{line_number} invalid TOML assignment",
                path.display()
            )));
        };
        let key = key.trim();
        let value = value.trim();
        match section {
            PerfSection::Root => {
                if key != "schema_version" {
                    return Err(XtaskError::Command(format!(
                        "{}:{line_number} unexpected root key: {key}",
                        path.display()
                    )));
                }
            }
            PerfSection::Runner => runner.set(path, line_number, key, value)?,
            PerfSection::Budget => {
                let Some(budget) = current_budget.as_mut() else {
                    return Err(XtaskError::Command(format!(
                        "{}:{line_number} budget key outside budget block",
                        path.display()
                    )));
                };
                budget.set(path, line_number, key, value)?;
            }
        }
    }
    push_budget(path, current_budget.take(), &mut budgets)?;

    let runner = runner.finish(path)?;
    if budgets.is_empty() {
        return Err(XtaskError::Command(format!(
            "{} must define at least one [[budgets]] entry",
            path.display()
        )));
    }
    for budget in &budgets {
        if budget.runner != runner.id {
            return Err(XtaskError::Command(format!(
                "{} budget {} references unknown runner {}",
                path.display(),
                budget.case,
                budget.runner
            )));
        }
    }

    Ok(PerfBudgetManifest { runner, budgets })
}

fn push_budget(
    path: &Path,
    current: Option<PerfBudgetDraft>,
    budgets: &mut Vec<PerfBudget>,
) -> Result<(), XtaskError> {
    if let Some(current) = current {
        let budget = current.finish(path)?;
        if budgets.iter().any(|existing| existing.case == budget.case) {
            return Err(XtaskError::Command(format!(
                "{} duplicate budget case: {}",
                path.display(),
                budget.case
            )));
        }
        budgets.push(budget);
    }
    Ok(())
}

fn strip_toml_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or(line)
}

fn toml_string(path: &Path, line_number: usize, value: &str) -> Result<String, XtaskError> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            XtaskError::Command(format!(
                "{}:{line_number} expected quoted string",
                path.display()
            ))
        })
}

fn toml_u64(path: &Path, line_number: usize, value: &str) -> Result<u64, XtaskError> {
    value.parse::<u64>().map_err(|_| {
        XtaskError::Command(format!(
            "{}:{line_number} expected unsigned integer",
            path.display()
        ))
    })
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

fn print_bench_help() {
    println!("Usage:");
    println!("  cargo xtask bench --profile smoke|full [--iterations <n>]");
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

#[derive(Debug, Clone, Eq, PartialEq)]
struct BenchOptions {
    profile: String,
    iterations: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct BenchCase {
    kind: FixtureKind,
}

impl BenchCase {
    fn name(self) -> String {
        format!("fixture_generation/{}", self.kind.id())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct BenchRow {
    iterations: u32,
    total_bytes: usize,
    elapsed: Duration,
}

impl BenchRow {
    fn ns_per_iter(self) -> u128 {
        self.elapsed.as_nanos() / u128::from(self.iterations)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PerfBudgetManifest {
    runner: RunnerFingerprint,
    budgets: Vec<PerfBudget>,
}

impl PerfBudgetManifest {
    fn budget_for_case(&self, case: &str) -> Option<&PerfBudget> {
        self.budgets.iter().find(|budget| budget.case == case)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct RunnerFingerprint {
    id: String,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
struct RunnerFingerprintDraft {
    id: Option<String>,
}

impl RunnerFingerprintDraft {
    fn set(
        &mut self,
        path: &Path,
        line_number: usize,
        key: &str,
        value: &str,
    ) -> Result<(), XtaskError> {
        match key {
            "id" => self.id = Some(toml_string(path, line_number, value)?),
            "description" | "os" | "arch" | "rust_toolchain" | "fixture_source"
            | "cache_policy" => {
                let _ = toml_string(path, line_number, value)?;
            }
            other => {
                return Err(XtaskError::Command(format!(
                    "{}:{line_number} unknown runner_fingerprint key: {other}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn finish(self, path: &Path) -> Result<RunnerFingerprint, XtaskError> {
        Ok(RunnerFingerprint {
            id: self.id.ok_or_else(|| {
                XtaskError::Command(format!(
                    "{} [runner_fingerprint] requires id",
                    path.display()
                ))
            })?,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PerfBudget {
    case: String,
    runner: String,
    absolute_p95_ms: u64,
    relative_regression_pct: u64,
    minimum_effect_ms: u64,
    policy: BudgetPolicy,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
struct PerfBudgetDraft {
    case: Option<String>,
    runner: Option<String>,
    absolute_p95_ms: Option<u64>,
    relative_regression_pct: Option<u64>,
    minimum_effect_ms: Option<u64>,
    policy: Option<BudgetPolicy>,
}

impl PerfBudgetDraft {
    fn set(
        &mut self,
        path: &Path,
        line_number: usize,
        key: &str,
        value: &str,
    ) -> Result<(), XtaskError> {
        match key {
            "case" => self.case = Some(toml_string(path, line_number, value)?),
            "runner" => self.runner = Some(toml_string(path, line_number, value)?),
            "absolute_p95_ms" => self.absolute_p95_ms = Some(toml_u64(path, line_number, value)?),
            "relative_regression_pct" => {
                self.relative_regression_pct = Some(toml_u64(path, line_number, value)?);
            }
            "minimum_effect_ms" => {
                self.minimum_effect_ms = Some(toml_u64(path, line_number, value)?)
            }
            "policy" => {
                self.policy = Some(
                    BudgetPolicy::parse(&toml_string(path, line_number, value)?).ok_or_else(
                        || {
                            XtaskError::Command(format!(
                                "{}:{line_number} unknown budget policy",
                                path.display()
                            ))
                        },
                    )?,
                );
            }
            other => {
                return Err(XtaskError::Command(format!(
                    "{}:{line_number} unknown budget key: {other}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn finish(self, path: &Path) -> Result<PerfBudget, XtaskError> {
        let case = self.case.ok_or_else(|| {
            XtaskError::Command(format!("{} [[budgets]] requires case", path.display()))
        })?;
        let runner = self.runner.ok_or_else(|| {
            XtaskError::Command(format!("{} [[budgets]] requires runner", path.display()))
        })?;
        let absolute_p95_ms = self.absolute_p95_ms.ok_or_else(|| {
            XtaskError::Command(format!(
                "{} [[budgets]] requires absolute_p95_ms",
                path.display()
            ))
        })?;
        if absolute_p95_ms == 0 {
            return Err(XtaskError::Command(format!(
                "{} budget {case} absolute_p95_ms must be positive",
                path.display()
            )));
        }
        Ok(PerfBudget {
            case,
            runner,
            absolute_p95_ms,
            relative_regression_pct: self.relative_regression_pct.ok_or_else(|| {
                XtaskError::Command(format!(
                    "{} [[budgets]] requires relative_regression_pct",
                    path.display()
                ))
            })?,
            minimum_effect_ms: self.minimum_effect_ms.ok_or_else(|| {
                XtaskError::Command(format!(
                    "{} [[budgets]] requires minimum_effect_ms",
                    path.display()
                ))
            })?,
            policy: self.policy.ok_or_else(|| {
                XtaskError::Command(format!("{} [[budgets]] requires policy", path.display()))
            })?,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BudgetPolicy {
    Block,
    Warn,
    Observe,
}

impl BudgetPolicy {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "warn" => Some(Self::Warn),
            "observe" => Some(Self::Observe),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Warn => "warn",
            Self::Observe => "observe",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PerfSection {
    Root,
    Runner,
    Budget,
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

    #[test]
    fn perf_budget_manifest_parses_budget_fields() {
        let manifest = parse_perf_budget_manifest(
            Path::new("perf/performance-budgets.toml"),
            r#"
schema_version = 1

[runner_fingerprint]
id = "local-generated-fixture-smoke"
description = "local"
os = "any"
arch = "any"
rust_toolchain = "stable"
fixture_source = "generated"
cache_policy = "warm"

[[budgets]]
case = "fixture_generation/minimal-epub3"
runner = "local-generated-fixture-smoke"
absolute_p95_ms = 50
relative_regression_pct = 25
minimum_effect_ms = 5
policy = "warn"
"#,
        )
        .expect("parse perf budget manifest");

        assert_eq!(manifest.runner.id, "local-generated-fixture-smoke");
        let budget = manifest
            .budget_for_case("fixture_generation/minimal-epub3")
            .expect("budget");
        assert_eq!(budget.absolute_p95_ms, 50);
        assert_eq!(budget.relative_regression_pct, 25);
        assert_eq!(budget.minimum_effect_ms, 5);
        assert_eq!(budget.policy, BudgetPolicy::Warn);
    }

    #[test]
    fn bench_smoke_profile_uses_generated_fixture_subset() {
        let cases = bench_cases_for_profile("smoke").expect("smoke profile");

        assert_eq!(cases.len(), 4);
        assert_eq!(cases[0].name(), "fixture_generation/minimal-epub3");
        assert!(bench_cases_for_profile("bogus").is_err());
    }
}
