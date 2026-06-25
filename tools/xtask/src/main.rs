#![forbid(unsafe_code)]

use std::{
    env, fs,
    hint::black_box,
    io,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use pagelet::epub::{open_book_ir, open_spine_item_chapter_ir};
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

    if let Some(root) = &root {
        if !root.exists() {
            return Err(XtaskError::Command(format!(
                "corpus root does not exist: {}",
                root.display()
            )));
        }
    } else if required {
        return Err(XtaskError::Command(
            "PAGELET_CORPUS_ROOT is required but not set".into(),
        ));
    }

    let manifest_text = fs::read_to_string(&manifest)?;
    let books = parse_corpus_manifest(&manifest, &manifest_text)?;
    let selected = select_corpus_books(&books, &profile);
    if selected.is_empty() {
        if required {
            return Err(XtaskError::Command(format!(
                "corpus profile={profile} selected no books"
            )));
        }
        println!(
            "corpus profile={profile} manifest={} selected=0",
            manifest.display()
        );
        return Ok(());
    }

    let mut failures = Vec::new();
    for book in &selected {
        match corpus_book_bytes(book, root.as_deref()) {
            Ok(bytes) => match validate_corpus_book(book, &bytes) {
                Ok(summary) => println!(
                    "corpus case={} status=ok chapters_checked={} visible_chars={}",
                    book.id, summary.chapters_checked, summary.visible_chars
                ),
                Err(error) => failures.push(format!("{}: {error}", book.id)),
            },
            Err(error) => {
                if book.license == "generated" || required {
                    failures.push(format!("{}: {error}", book.id));
                } else {
                    println!("corpus case={} status=skipped reason={error}", book.id);
                }
            }
        }
    }

    println!(
        "corpus profile={profile} root={} manifest={} selected={}",
        root.as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<generated-only>".to_owned()),
        manifest.display(),
        selected.len()
    );

    if failures.is_empty() {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "corpus validation failed:\n{}",
            failures.join("\n")
        )))
    }
}

fn validate_corpus_profile(profile: &str) -> Result<(), XtaskError> {
    match profile {
        "smoke" | "full" | "robustness" | "locale" | "regression" => Ok(()),
        other => Err(XtaskError::Usage(format!(
            "unknown corpus profile: {other}"
        ))),
    }
}

fn parse_corpus_manifest(path: &Path, text: &str) -> Result<Vec<CorpusBook>, XtaskError> {
    require_schema_version(&path.display().to_string(), text)?;
    let mut books = Vec::new();
    let mut current: Option<CorpusBookDraft> = None;
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() || line.starts_with('[') && line != "[[books]]" {
            continue;
        }
        if line == "[[books]]" {
            push_corpus_book(path, current.take(), &mut books)?;
            current = Some(CorpusBookDraft::default());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(book) = current.as_mut() else {
            continue;
        };
        match key.trim() {
            "id" => book.id = Some(toml_string(path, line_number, value.trim())?),
            "path" => book.path = Some(toml_string(path, line_number, value.trim())?),
            "sha256" => book.sha256 = Some(toml_string(path, line_number, value.trim())?),
            "license" => book.license = Some(toml_string(path, line_number, value.trim())?),
            "expected" => book.expected = Some(toml_string(path, line_number, value.trim())?),
            "categories" => book.categories = parse_toml_string_array(path, line_number, value)?,
            _ => {}
        }
    }
    push_corpus_book(path, current.take(), &mut books)?;
    if books.is_empty() {
        return Err(XtaskError::Command(format!(
            "{} must define at least one [[books]] entry",
            path.display()
        )));
    }
    Ok(books)
}

fn push_corpus_book(
    path: &Path,
    current: Option<CorpusBookDraft>,
    books: &mut Vec<CorpusBook>,
) -> Result<(), XtaskError> {
    if let Some(current) = current {
        let book = current.finish(path)?;
        if books.iter().any(|existing| existing.id == book.id) {
            return Err(XtaskError::Command(format!(
                "{} duplicate corpus book id: {}",
                path.display(),
                book.id
            )));
        }
        books.push(book);
    }
    Ok(())
}

fn parse_toml_string_array(
    path: &Path,
    line_number: usize,
    value: &str,
) -> Result<Vec<String>, XtaskError> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for raw in inner.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        out.push(toml_string(path, line_number, raw)?);
    }
    Ok(out)
}

fn select_corpus_books<'a>(books: &'a [CorpusBook], profile: &str) -> Vec<&'a CorpusBook> {
    books
        .iter()
        .filter(|book| {
            profile == "full" || book.categories.iter().any(|category| category == profile)
        })
        .collect()
}

fn corpus_book_bytes(book: &CorpusBook, root: Option<&Path>) -> Result<Vec<u8>, XtaskError> {
    if book.license == "generated" {
        return generated_corpus_fixture(book)
            .map(|fixture| fixture.bytes().to_vec())
            .ok_or_else(|| {
                XtaskError::Command(format!(
                    "no generated fixture is registered for {}",
                    book.id
                ))
            });
    }
    let Some(root) = root else {
        return Err(XtaskError::Command(
            "PAGELET_CORPUS_ROOT not set for private corpus case".into(),
        ));
    };
    let path = root.join(&book.path);
    let bytes = fs::read(&path)?;
    if let Some(expected) = book.sha256.as_deref() {
        if !expected.chars().all(|ch| ch == '0') {
            let actual = sha256_hex(&bytes);
            if actual != expected {
                return Err(XtaskError::Command(format!(
                    "{} sha256 mismatch: expected {expected}, got {actual}",
                    path.display()
                )));
            }
        }
    }
    Ok(bytes)
}

fn generated_corpus_fixture(book: &CorpusBook) -> Option<pagelet_testkit::Fixture> {
    let kind = match book.id.as_str() {
        "generated/minimal-epub3" => FixtureKind::MinimalEpub3,
        "generated/pathological" => FixtureKind::ZipBombLike,
        _ => return None,
    };
    Some(ValidEpubBuilder::preset(kind).build())
}

fn validate_corpus_book(book: &CorpusBook, bytes: &[u8]) -> Result<CorpusSummary, XtaskError> {
    let book_ir = match open_book_ir(bytes.to_vec()) {
        Ok(ir) if book.expected != "invalid" => ir,
        Ok(_) => {
            return Err(XtaskError::Command(
                "expected invalid corpus case opened successfully".into(),
            ));
        }
        Err(_) if book.expected == "invalid" => {
            return Ok(CorpusSummary {
                chapters_checked: 0,
                visible_chars: 0,
            });
        }
        Err(error) => return Err(XtaskError::Command(error.to_string())),
    };

    let mut chapters_checked = 0_usize;
    let mut visible_chars = 0_usize;
    for (index, spine) in book_ir.spine.iter().enumerate() {
        if !spine.linear {
            continue;
        }
        chapters_checked += 1;
        let chapter = open_spine_item_chapter_ir(bytes.to_vec(), index)
            .map_err(|error| XtaskError::Command(error.to_string()))?;
        visible_chars = visible_chars.saturating_add(chapter.visible_text().chars().count());
        if visible_chars > 0 {
            break;
        }
    }
    if visible_chars == 0 && book.expected != "invalid" {
        return Err(XtaskError::Command(
            "no visible chapter text extracted".into(),
        ));
    }
    Ok(CorpusSummary {
        chapters_checked,
        visible_chars,
    })
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
struct CorpusBook {
    id: String,
    path: String,
    sha256: Option<String>,
    license: String,
    expected: String,
    categories: Vec<String>,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
struct CorpusBookDraft {
    id: Option<String>,
    path: Option<String>,
    sha256: Option<String>,
    license: Option<String>,
    expected: Option<String>,
    categories: Vec<String>,
}

impl CorpusBookDraft {
    fn finish(self, path: &Path) -> Result<CorpusBook, XtaskError> {
        Ok(CorpusBook {
            id: self.id.ok_or_else(|| {
                XtaskError::Command(format!("{} [[books]] requires id", path.display()))
            })?,
            path: self.path.ok_or_else(|| {
                XtaskError::Command(format!("{} [[books]] requires path", path.display()))
            })?,
            sha256: self.sha256,
            license: self.license.ok_or_else(|| {
                XtaskError::Command(format!("{} [[books]] requires license", path.display()))
            })?,
            expected: self.expected.ok_or_else(|| {
                XtaskError::Command(format!("{} [[books]] requires expected", path.display()))
            })?,
            categories: self.categories,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CorpusSummary {
    chapters_checked: usize,
    visible_chars: usize,
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

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut h = [
        0x6a09_e667_u32,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks(64) {
        let mut w = [0_u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
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
    fn corpus_manifest_parses_and_selects_profile_cases() {
        let books = parse_corpus_manifest(
            Path::new("tests/corpus-manifest.toml"),
            r#"
schema_version = 1

[[books]]
id = "generated/minimal-epub3"
path = "generated/minimal-epub3.epub"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
license = "generated"
expected = "valid"
categories = ["smoke"]
features = ["package"]

[[books]]
id = "private/regression"
path = "book.epub"
sha256 = "abc"
license = "private-ci"
expected = "valid"
categories = ["regression"]
"#,
        )
        .expect("manifest");

        assert_eq!(books.len(), 2);
        assert_eq!(
            select_corpus_books(&books, "smoke")[0].id,
            "generated/minimal-epub3"
        );
        assert_eq!(
            select_corpus_books(&books, "regression")[0].path,
            "book.epub"
        );
        assert_eq!(select_corpus_books(&books, "full").len(), 2);
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
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
rust_toolchain = "1.95.0"
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
