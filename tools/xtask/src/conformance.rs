use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use pagelet::epub::{open_book_ir, open_spine_item_chapter_ir};
use pagelet_testkit::{EpubMutation, EpubMutator, FixtureKind, ValidEpubBuilder};

use super::{
    atomic_write, external, parse_toml_string_array, require_schema_version, strip_toml_comment,
    toml_string, XtaskError,
};

const W3C_MAPPING_PATH: &str = "tests/conformance/w3c-mapping.toml";
const REPORT_ROOT: &str = "target/pagelet-conformance";
const REFERENCE_REPORTS: [(&str, &str); 2] = [
    ("Apple Books macOS", "apple-macos.json"),
    ("Thorium Reader 2.2", "thorium22-osx-win-deb.json"),
];

pub(super) fn lint_mapping() -> Result<(), XtaskError> {
    read_w3c_mappings().map(|_| ())
}

pub(super) fn run_w3c(args: &[String]) -> Result<(), XtaskError> {
    let options = W3cOptions::parse(args)?;
    if options.help {
        print_w3c_help();
        return Ok(());
    }
    let artifacts = match external::verified_artifacts() {
        Ok(artifacts) => artifacts,
        Err(error) if !options.required => {
            println!("w3c status=skipped reason={error}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let mappings = read_w3c_mappings()?;
    let selected: Vec<_> = mappings
        .into_iter()
        .filter(|mapping| options.profile == "all" || mapping.profile == options.profile)
        .collect();
    if selected.is_empty() {
        return Err(XtaskError::Command(format!(
            "W3C profile={} selected no mapped tests",
            options.profile
        )));
    }

    let references = read_reference_reports(&artifacts)?;
    let mut results = Vec::with_capacity(selected.len());
    for mapping in selected {
        let result = match mapping.automation {
            Automation::Structural | Automation::Semantic => {
                run_w3c_case(&artifacts, &mapping, &references)
            }
            Automation::VisualManual => W3cResult::not_automated(
                mapping,
                "manual",
                "requires reading-system visual inspection",
                &references,
            ),
            Automation::NotApplicable => W3cResult::not_automated(
                mapping,
                "not-applicable",
                "outside the parser/layout library boundary",
                &references,
            ),
        };
        println!(
            "w3c case={} status={} feature={}",
            result.id, result.status, result.feature_id
        );
        results.push(result);
    }

    write_w3c_reports(&options, &artifacts, &results)?;
    let failures: Vec<_> = results
        .iter()
        .filter(|result| result.status == "fail")
        .map(|result| format!("{}: {}", result.id, result.detail))
        .collect();
    if failures.is_empty() || !options.required {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "W3C conformance failed:\n{}",
            failures.join("\n")
        )))
    }
}

pub(super) fn run_epubcheck(args: &[String]) -> Result<(), XtaskError> {
    let options = EpubcheckOptions::parse(args)?;
    if options.help {
        print_epubcheck_help();
        return Ok(());
    }
    let artifacts = match external::verified_artifacts() {
        Ok(artifacts) => artifacts,
        Err(error) if !options.required => {
            println!("epubcheck status=skipped reason={error}");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let jar = prepare_epubcheck(&artifacts)?;
    if let Err(error) = command_output(Command::new("java").arg("-version"), "java") {
        if options.required {
            return Err(error);
        }
        println!("epubcheck status=skipped reason={error}");
        return Ok(());
    }

    let fixture_root = Path::new(REPORT_ROOT).join("epubcheck-fixtures");
    fs::create_dir_all(&fixture_root)?;
    let mut results = Vec::new();
    for kind in &options.fixtures {
        let (id, fixture, expected_valid) = match kind.as_str() {
            "valid" => (
                "generated/minimal-epub3",
                ValidEpubBuilder::preset(FixtureKind::MinimalEpub3).build(),
                true,
            ),
            "invalid" => (
                "generated/missing-package-document",
                EpubMutator::new(ValidEpubBuilder::preset(FixtureKind::MinimalEpub3).build())
                    .apply(EpubMutation::MissingPackageDocument),
                false,
            ),
            _ => unreachable!("fixture names were validated"),
        };
        let epub_path = fixture_root.join(format!("{}.epub", id.replace('/', "-")));
        let detail_path = fixture_root.join(format!("{}.json", id.replace('/', "-")));
        atomic_write(&epub_path, fixture.bytes())?;
        let output = command_output(
            Command::new("java")
                .arg("-jar")
                .arg(&jar)
                .arg(&epub_path)
                .args(["--json", detail_path.to_string_lossy().as_ref(), "--quiet"]),
            "EPUBCheck",
        )?;
        let accepted = output.status.success();
        let passed = accepted == expected_valid;
        let detail = first_output_line(&output).unwrap_or_else(|| {
            if accepted {
                "no EPUBCheck errors".to_owned()
            } else {
                "EPUBCheck rejected the fixture".to_owned()
            }
        });
        println!(
            "epubcheck case={id} expected={} status={}",
            if expected_valid { "valid" } else { "invalid" },
            if passed { "pass" } else { "fail" }
        );
        results.push(EpubcheckResult {
            id: id.to_owned(),
            expected: if expected_valid { "valid" } else { "invalid" },
            status: if passed { "pass" } else { "fail" },
            detail,
            detail_report: detail_path,
        });
    }
    write_epubcheck_reports(&options, &artifacts, &results)?;

    let failures: Vec<_> = results
        .iter()
        .filter(|result| result.status == "fail")
        .map(|result| result.id.as_str())
        .collect();
    if failures.is_empty() || !options.required {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "EPUBCheck fixture classification failed: {}",
            failures.join(", ")
        )))
    }
}

fn run_w3c_case(
    artifacts: &external::VerifiedArtifacts,
    mapping: &W3cMapping,
    references: &[(String, String)],
) -> W3cResult {
    match package_w3c_case(artifacts, &mapping.id).and_then(|path| validate_epub(&path)) {
        Ok(summary) => W3cResult {
            id: mapping.id.clone(),
            requirement_id: mapping.requirement_id.clone(),
            feature_id: mapping.feature_id.clone(),
            automation: mapping.automation.as_str(),
            status: "pass",
            chapters: summary.chapters,
            visible_chars: summary.visible_chars,
            detail: "pagelet opened all linear spine items".into(),
            references: reference_values(references, &mapping.id),
        },
        Err(error) => W3cResult {
            id: mapping.id.clone(),
            requirement_id: mapping.requirement_id.clone(),
            feature_id: mapping.feature_id.clone(),
            automation: mapping.automation.as_str(),
            status: "fail",
            chapters: 0,
            visible_chars: 0,
            detail: error.to_string(),
            references: reference_values(references, &mapping.id),
        },
    }
}

fn package_w3c_case(
    artifacts: &external::VerifiedArtifacts,
    test_id: &str,
) -> Result<PathBuf, XtaskError> {
    validate_id(test_id)?;
    let root = env::current_dir()?
        .join(REPORT_ROOT)
        .join("w3c-work")
        .join(test_id);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    let prefix = format!("epub-tests-{}", artifacts.w3c_commit);
    let pattern = format!("{prefix}/tests/{test_id}/*");
    command_success(
        Command::new("unzip")
            .args(["-qq"])
            .arg(&artifacts.w3c_archive)
            .arg(pattern)
            .arg("-d")
            .arg(&root),
        "extract W3C test",
    )?;
    let source = root.join(prefix).join("tests").join(test_id);
    for required in ["mimetype", "META-INF", "EPUB"] {
        if !source.join(required).exists() {
            return Err(XtaskError::Command(format!(
                "W3C test {test_id} is missing {required}"
            )));
        }
    }
    let output = root.join(format!("{test_id}.epub"));
    command_success(
        Command::new("zip")
            .current_dir(&source)
            .args(["-q", "-X", "-0"])
            .arg(&output)
            .arg("mimetype"),
        "package W3C mimetype",
    )?;
    command_success(
        Command::new("zip")
            .current_dir(&source)
            .args(["-q", "-X", "-r"])
            .arg(&output)
            .args([".", "-x", "mimetype"]),
        "package W3C publication",
    )?;
    Ok(output)
}

fn validate_epub(path: &Path) -> Result<W3cSummary, XtaskError> {
    let bytes = fs::read(path)?;
    let book =
        open_book_ir(bytes.clone()).map_err(|error| XtaskError::Command(error.to_string()))?;
    let mut chapters = 0;
    let mut visible_chars = 0;
    for (index, spine) in book.spine.iter().enumerate() {
        if !spine.linear {
            continue;
        }
        let chapter = open_spine_item_chapter_ir(bytes.clone(), index)
            .map_err(|error| XtaskError::Command(error.to_string()))?;
        chapters += 1;
        visible_chars += chapter.visible_text().chars().count();
    }
    if chapters == 0 {
        return Err(XtaskError::Command(
            "publication has no linear spine items".into(),
        ));
    }
    Ok(W3cSummary {
        chapters,
        visible_chars,
    })
}

fn prepare_epubcheck(artifacts: &external::VerifiedArtifacts) -> Result<PathBuf, XtaskError> {
    let root = env::current_dir()?
        .join(REPORT_ROOT)
        .join(format!("epubcheck-{}", artifacts.epubcheck_version));
    let jar = root
        .join(format!("epubcheck-{}", artifacts.epubcheck_version))
        .join("epubcheck.jar");
    if !jar.exists() {
        fs::create_dir_all(&root)?;
        command_success(
            Command::new("unzip")
                .args(["-q", "-o"])
                .arg(&artifacts.epubcheck_archive)
                .arg(format!(
                    "epubcheck-{}/epubcheck.jar",
                    artifacts.epubcheck_version
                ))
                .arg(format!("epubcheck-{}/lib/*", artifacts.epubcheck_version))
                .arg("-d")
                .arg(&root),
            "extract EPUBCheck",
        )?;
    }
    if !jar.exists() {
        return Err(XtaskError::Command(format!(
            "EPUBCheck archive does not contain {}",
            jar.display()
        )));
    }
    Ok(jar)
}

fn read_w3c_mappings() -> Result<Vec<W3cMapping>, XtaskError> {
    let path = Path::new(W3C_MAPPING_PATH);
    let text = fs::read_to_string(path)?;
    require_schema_version(W3C_MAPPING_PATH, &text)?;
    let mut current = None;
    let mut mappings = Vec::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() || line.starts_with('[') && line != "[[tests]]" {
            continue;
        }
        if line == "[[tests]]" {
            push_mapping(path, current.take(), &mut mappings)?;
            current = Some(W3cMappingDraft::default());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(mapping) = current.as_mut() else {
            continue;
        };
        match key.trim() {
            "id" => mapping.id = Some(toml_string(path, line_number, value.trim())?),
            "requirement_id" => {
                mapping.requirement_id = Some(toml_string(path, line_number, value.trim())?)
            }
            "feature_id" => {
                mapping.feature_id = Some(toml_string(path, line_number, value.trim())?)
            }
            "profile" => mapping.profile = Some(toml_string(path, line_number, value.trim())?),
            "automation" => {
                mapping.automation = Some(toml_string(path, line_number, value.trim())?)
            }
            "references" => mapping.references = parse_toml_string_array(path, line_number, value)?,
            _ => {}
        }
    }
    push_mapping(path, current.take(), &mut mappings)?;
    if mappings.is_empty() {
        return Err(XtaskError::Command(format!(
            "{W3C_MAPPING_PATH} must define at least one [[tests]] entry"
        )));
    }
    Ok(mappings)
}

fn push_mapping(
    path: &Path,
    draft: Option<W3cMappingDraft>,
    mappings: &mut Vec<W3cMapping>,
) -> Result<(), XtaskError> {
    if let Some(draft) = draft {
        let mapping = draft.finish(path)?;
        if mappings.iter().any(|existing| existing.id == mapping.id) {
            return Err(XtaskError::Command(format!(
                "{} duplicate W3C test id: {}",
                path.display(),
                mapping.id
            )));
        }
        mappings.push(mapping);
    }
    Ok(())
}

fn read_reference_reports(
    artifacts: &external::VerifiedArtifacts,
) -> Result<Vec<(String, String)>, XtaskError> {
    let prefix = format!("epub-tests-{}", artifacts.w3c_commit);
    REFERENCE_REPORTS
        .iter()
        .map(|(name, file)| {
            let entry = format!("{prefix}/reports/{file}");
            let output = command_output(
                Command::new("unzip")
                    .arg("-p")
                    .arg(&artifacts.w3c_archive)
                    .arg(entry),
                "read W3C reference report",
            )?;
            if !output.status.success() {
                return Err(command_failed("read W3C reference report", &output));
            }
            Ok((
                (*name).to_owned(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
            ))
        })
        .collect()
}

fn reference_values(reports: &[(String, String)], id: &str) -> Vec<(String, String)> {
    reports
        .iter()
        .map(|(name, report)| {
            let value = json_object_value(report, id).unwrap_or_else(|| "unreported".into());
            (name.clone(), value)
        })
        .collect()
}

fn json_object_value(json: &str, key: &str) -> Option<String> {
    let prefix = format!("\"{key}\":");
    json.lines().find_map(|line| {
        let line = line.trim();
        let value = line
            .strip_prefix(&prefix)?
            .trim()
            .trim_end_matches(',')
            .trim();
        Some(value.trim_matches('"').to_owned())
    })
}

fn write_w3c_reports(
    options: &W3cOptions,
    artifacts: &external::VerifiedArtifacts,
    results: &[W3cResult],
) -> Result<(), XtaskError> {
    let automated = results
        .iter()
        .filter(|result| matches!(result.status, "pass" | "fail"))
        .count();
    let passed = results
        .iter()
        .filter(|result| result.status == "pass")
        .count();
    let mut json = format!(
        "{{\n  \"schema_version\": 1,\n  \"w3c_commit\": \"{}\",\n  \"profile\": \"{}\",\n  \"automated\": {automated},\n  \"passed\": {passed},\n  \"results\": [\n",
        artifacts.w3c_commit, options.profile
    );
    for (index, result) in results.iter().enumerate() {
        let refs = result
            .references
            .iter()
            .map(|(name, value)| format!("\"{}\": \"{}\"", json_escape(name), json_escape(value)))
            .collect::<Vec<_>>()
            .join(", ");
        json.push_str(&format!(
            "    {{\"id\": \"{}\", \"requirement_id\": \"{}\", \"feature_id\": \"{}\", \"automation\": \"{}\", \"status\": \"{}\", \"chapters\": {}, \"visible_chars\": {}, \"detail\": \"{}\", \"references\": {{{refs}}}}}{}\n",
            json_escape(&result.id), json_escape(&result.requirement_id), json_escape(&result.feature_id),
            result.automation, result.status, result.chapters, result.visible_chars,
            json_escape(&result.detail), if index + 1 == results.len() { "" } else { "," }
        ));
    }
    json.push_str("  ]\n}\n");
    atomic_write(&options.json, json.as_bytes())?;

    let pass_rate = if automated == 0 {
        0.0
    } else {
        passed as f64 * 100.0 / automated as f64
    };
    let mut markdown = format!(
        "# W3C EPUB conformance\n\n- Commit: `{}`\n- Profile: `{}`\n- Automated pass rate: `{passed}/{automated}` ({pass_rate:.1}%)\n\n| Test | Feature | Automation | Pagelet | Chapters | Visible chars | Apple Books | Thorium | Detail |\n|---|---|---|---:|---:|---:|---:|---:|---|\n",
        artifacts.w3c_commit, options.profile
    );
    for result in results {
        markdown.push_str(&format!(
            "| `{}` | `{}` | `{}` | `{}` | {} | {} | `{}` | `{}` | {} |\n",
            result.id,
            result.feature_id,
            result.automation,
            result.status,
            result.chapters,
            result.visible_chars,
            result
                .references
                .first()
                .map_or("unreported", |value| value.1.as_str()),
            result
                .references
                .get(1)
                .map_or("unreported", |value| value.1.as_str()),
            markdown_cell(&result.detail)
        ));
    }
    atomic_write(&options.report, markdown.as_bytes())?;
    println!("W3C JSON report: {}", options.json.display());
    println!("W3C Markdown report: {}", options.report.display());
    Ok(())
}

fn write_epubcheck_reports(
    options: &EpubcheckOptions,
    artifacts: &external::VerifiedArtifacts,
    results: &[EpubcheckResult],
) -> Result<(), XtaskError> {
    let passed = results
        .iter()
        .filter(|result| result.status == "pass")
        .count();
    let mut json = format!(
        "{{\n  \"schema_version\": 1,\n  \"epubcheck_version\": \"{}\",\n  \"profile\": \"{}\",\n  \"passed\": {passed},\n  \"total\": {},\n  \"results\": [\n",
        artifacts.epubcheck_version,
        json_escape(&artifacts.epubcheck_profile),
        results.len()
    );
    for (index, result) in results.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"id\": \"{}\", \"expected\": \"{}\", \"status\": \"{}\", \"detail\": \"{}\", \"detail_report\": \"{}\"}}{}\n",
            json_escape(&result.id), result.expected, result.status, json_escape(&result.detail),
            json_escape(&result.detail_report.display().to_string()),
            if index + 1 == results.len() { "" } else { "," }
        ));
    }
    json.push_str("  ]\n}\n");
    atomic_write(&options.json, json.as_bytes())?;

    let mut markdown = format!(
        "# EPUBCheck fixture validation\n\n- EPUBCheck: `{}`\n- Target profile: `{}`\n- Classification: `{passed}/{}`\n\n| Fixture | Expected | Status | Detail |\n|---|---|---|---|\n",
        artifacts.epubcheck_version,
        artifacts.epubcheck_profile,
        results.len()
    );
    for result in results {
        markdown.push_str(&format!(
            "| `{}` | `{}` | `{}` | {} |\n",
            result.id,
            result.expected,
            result.status,
            markdown_cell(&result.detail)
        ));
    }
    atomic_write(&options.report, markdown.as_bytes())?;
    println!("EPUBCheck JSON report: {}", options.json.display());
    println!("EPUBCheck Markdown report: {}", options.report.display());
    Ok(())
}

fn command_success(command: &mut Command, label: &str) -> Result<(), XtaskError> {
    let output = command_output(command, label)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failed(label, &output))
    }
}

fn command_output(command: &mut Command, label: &str) -> Result<Output, XtaskError> {
    command.output().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            XtaskError::Command(format!(
                "{label} requires {}",
                command.get_program().to_string_lossy()
            ))
        } else {
            XtaskError::Io(error)
        }
    })
}

fn command_failed(label: &str, output: &Output) -> XtaskError {
    let detail = first_output_line(output).unwrap_or_else(|| "command failed".into());
    XtaskError::Command(format!("{label} failed: {detail}"))
}

fn first_output_line(output: &Output) -> Option<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .chain(String::from_utf8_lossy(&output.stdout).lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(240).collect())
}

fn validate_id(id: &str) -> Result<(), XtaskError> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "unsafe conformance test id: {id}"
        )))
    }
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn print_w3c_help() {
    println!("Usage:");
    println!(
        "  cargo xtask w3c --profile required|all [--required] [--json <path>] [--report <path>]"
    );
}

fn print_epubcheck_help() {
    println!("Usage:");
    println!("  cargo xtask epubcheck --fixtures valid,invalid [--required] [--json <path>] [--report <path>]");
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct W3cOptions {
    profile: String,
    required: bool,
    json: PathBuf,
    report: PathBuf,
    help: bool,
}

impl W3cOptions {
    fn parse(args: &[String]) -> Result<Self, XtaskError> {
        let mut options = Self {
            profile: "required".into(),
            required: false,
            json: PathBuf::from(REPORT_ROOT).join("w3c.json"),
            report: PathBuf::from(REPORT_ROOT).join("w3c.md"),
            help: false,
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--profile" => {
                    index += 1;
                    options.profile = args
                        .get(index)
                        .cloned()
                        .ok_or_else(|| XtaskError::Usage("--profile requires a value".into()))?;
                }
                "--required" => options.required = true,
                "--json" => {
                    index += 1;
                    options.json = PathBuf::from(
                        args.get(index)
                            .ok_or_else(|| XtaskError::Usage("--json requires a value".into()))?,
                    );
                }
                "--report" => {
                    index += 1;
                    options.report =
                        PathBuf::from(args.get(index).ok_or_else(|| {
                            XtaskError::Usage("--report requires a value".into())
                        })?);
                }
                "-h" | "--help" | "help" => options.help = true,
                other => return Err(XtaskError::Usage(format!("unknown W3C option: {other}"))),
            }
            index += 1;
        }
        if !matches!(options.profile.as_str(), "required" | "all") {
            return Err(XtaskError::Usage(format!(
                "unknown W3C profile: {}",
                options.profile
            )));
        }
        Ok(options)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct EpubcheckOptions {
    fixtures: Vec<String>,
    required: bool,
    json: PathBuf,
    report: PathBuf,
    help: bool,
}

impl EpubcheckOptions {
    fn parse(args: &[String]) -> Result<Self, XtaskError> {
        let mut options = Self {
            fixtures: vec!["valid".into(), "invalid".into()],
            required: false,
            json: PathBuf::from(REPORT_ROOT).join("epubcheck.json"),
            report: PathBuf::from(REPORT_ROOT).join("epubcheck.md"),
            help: false,
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--fixtures" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| XtaskError::Usage("--fixtures requires a value".into()))?;
                    options.fixtures = value.split(',').map(str::to_owned).collect();
                }
                "--required" => options.required = true,
                "--json" => {
                    index += 1;
                    options.json = PathBuf::from(
                        args.get(index)
                            .ok_or_else(|| XtaskError::Usage("--json requires a value".into()))?,
                    );
                }
                "--report" => {
                    index += 1;
                    options.report =
                        PathBuf::from(args.get(index).ok_or_else(|| {
                            XtaskError::Usage("--report requires a value".into())
                        })?);
                }
                "-h" | "--help" | "help" => options.help = true,
                other => {
                    return Err(XtaskError::Usage(format!(
                        "unknown EPUBCheck option: {other}"
                    )))
                }
            }
            index += 1;
        }
        if options.fixtures.is_empty()
            || options
                .fixtures
                .iter()
                .any(|fixture| !matches!(fixture.as_str(), "valid" | "invalid"))
        {
            return Err(XtaskError::Usage(
                "--fixtures accepts valid, invalid, or valid,invalid".into(),
            ));
        }
        options.fixtures.sort();
        options.fixtures.dedup();
        Ok(options)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct W3cMapping {
    id: String,
    requirement_id: String,
    feature_id: String,
    profile: String,
    automation: Automation,
}

#[derive(Debug, Default)]
struct W3cMappingDraft {
    id: Option<String>,
    requirement_id: Option<String>,
    feature_id: Option<String>,
    profile: Option<String>,
    automation: Option<String>,
    references: Vec<String>,
}

impl W3cMappingDraft {
    fn finish(self, path: &Path) -> Result<W3cMapping, XtaskError> {
        let required = |name: &str, value: Option<String>| {
            value.ok_or_else(|| {
                XtaskError::Command(format!("{} [[tests]] requires {name}", path.display()))
            })
        };
        let id = required("id", self.id)?;
        validate_id(&id)?;
        let profile = required("profile", self.profile)?;
        if profile != "required" {
            return Err(XtaskError::Command(format!(
                "{} unknown W3C mapping profile: {profile}",
                path.display()
            )));
        }
        if self.references.len() != REFERENCE_REPORTS.len() {
            return Err(XtaskError::Command(format!(
                "{} W3C mapping {id} must name both pinned reference reports",
                path.display()
            )));
        }
        for reference in &self.references {
            if !REFERENCE_REPORTS.iter().any(|(_, file)| file == reference) {
                return Err(XtaskError::Command(format!(
                    "{} unknown reference report: {reference}",
                    path.display()
                )));
            }
        }
        Ok(W3cMapping {
            id,
            requirement_id: required("requirement_id", self.requirement_id)?,
            feature_id: required("feature_id", self.feature_id)?,
            profile,
            automation: Automation::parse(&required("automation", self.automation)?)
                .ok_or_else(|| XtaskError::Command("unknown W3C automation kind".into()))?,
        })
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Automation {
    Structural,
    Semantic,
    VisualManual,
    NotApplicable,
}

impl Automation {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "structural" => Some(Self::Structural),
            "semantic" => Some(Self::Semantic),
            "visual-manual" => Some(Self::VisualManual),
            "not-applicable" => Some(Self::NotApplicable),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Structural => "structural",
            Self::Semantic => "semantic",
            Self::VisualManual => "visual-manual",
            Self::NotApplicable => "not-applicable",
        }
    }
}

#[derive(Debug)]
struct W3cResult {
    id: String,
    requirement_id: String,
    feature_id: String,
    automation: &'static str,
    status: &'static str,
    chapters: usize,
    visible_chars: usize,
    detail: String,
    references: Vec<(String, String)>,
}

impl W3cResult {
    fn not_automated(
        mapping: W3cMapping,
        status: &'static str,
        detail: &str,
        references: &[(String, String)],
    ) -> Self {
        Self {
            references: reference_values(references, &mapping.id),
            id: mapping.id,
            requirement_id: mapping.requirement_id,
            feature_id: mapping.feature_id,
            automation: mapping.automation.as_str(),
            status,
            chapters: 0,
            visible_chars: 0,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct W3cSummary {
    chapters: usize,
    visible_chars: usize,
}

#[derive(Debug)]
struct EpubcheckResult {
    id: String,
    expected: &'static str,
    status: &'static str,
    detail: String,
    detail_report: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_w3c_options_and_rejects_unknown_profiles() {
        let options = W3cOptions::parse(&["--profile".into(), "all".into(), "--required".into()])
            .expect("options");

        assert_eq!(options.profile, "all");
        assert!(options.required);
        assert!(W3cOptions::parse(&["--profile".into(), "future".into()]).is_err());
    }

    #[test]
    fn parses_reference_results_without_json_dependency() {
        let report = r#"{"tests":{"pkg-spine-order": true,"scr-support": "n/a"}}"#;
        let pretty = "{\n  \"pkg-spine-order\": true,\n  \"scr-support\": \"n/a\"\n}";

        assert_eq!(
            json_object_value(pretty, "pkg-spine-order").as_deref(),
            Some("true")
        );
        assert_eq!(
            json_object_value(pretty, "scr-support").as_deref(),
            Some("n/a")
        );
        assert_eq!(json_object_value(report, "pkg-spine-order"), None);
    }

    #[test]
    fn epubcheck_fixture_selection_is_bounded() {
        let options =
            EpubcheckOptions::parse(&["--fixtures".into(), "invalid,valid,invalid".into()])
                .expect("options");

        assert_eq!(options.fixtures, ["invalid", "valid"]);
        assert!(EpubcheckOptions::parse(&["--fixtures".into(), "all".into()]).is_err());
    }
}
