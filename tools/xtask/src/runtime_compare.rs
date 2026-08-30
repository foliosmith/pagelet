use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::{atomic_write, XtaskError};

const REQUIRED_METRICS: [(&str, Target); 3] = [
    ("peak_rss_bytes", Target::ReductionPct(40)),
    ("first_page_ready", Target::Speedup(2)),
    ("height_only_repack", Target::Speedup(5)),
];

pub(super) fn run(args: &[String]) -> Result<(), XtaskError> {
    let options = Options::parse(args)?;
    if options.help {
        print_help();
        return Ok(());
    }
    let baseline_path = options.baseline.ok_or_else(|| {
        XtaskError::Usage("compare-runtime requires --baseline <dart.csv>".into())
    })?;
    let candidate_path = options.candidate.ok_or_else(|| {
        XtaskError::Usage("compare-runtime requires --candidate <rust.csv>".into())
    })?;
    let baseline = Snapshot::read(&baseline_path)?;
    let candidate = Snapshot::read(&candidate_path)?;
    if baseline.runtime != "dart" || candidate.runtime != "rust" {
        return Err(XtaskError::Command(format!(
            "runtime order must be dart baseline then rust candidate, got {} then {}",
            baseline.runtime, candidate.runtime
        )));
    }
    baseline.ensure_same_boundary(&candidate)?;

    let mut comparisons = Vec::new();
    for (name, target) in REQUIRED_METRICS {
        let dart = baseline.metric(name)?;
        let rust = candidate.metric(name)?;
        if dart.unit != rust.unit {
            return Err(XtaskError::Command(format!(
                "metric {name} unit mismatch: dart={}, rust={}",
                dart.unit, rust.unit
            )));
        }
        let value = target.value(dart.p95, rust.p95);
        comparisons.push(Comparison {
            name,
            unit: dart.unit.clone(),
            dart_p95: dart.p95,
            rust_p95: rust.p95,
            target,
            value,
            passed: target.passed(value),
        });
    }
    write_report(&options.report, &baseline, &candidate, &comparisons)?;
    println!("runtime comparison report: {}", options.report.display());

    let failures: Vec<_> = comparisons
        .iter()
        .filter(|comparison| !comparison.passed)
        .map(|comparison| comparison.name)
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "runtime comparison missed stage targets: {}",
            failures.join(", ")
        )))
    }
}

fn write_report(
    path: &Path,
    baseline: &Snapshot,
    candidate: &Snapshot,
    comparisons: &[Comparison],
) -> Result<(), XtaskError> {
    let mut out = format!(
        "# Dart/Rust same-boundary performance comparison\n\n- Runner: `{}`\n- Platform: `{}/{}`\n- Profile: `{}`\n- Fixture: `{}` (`{}`)\n- Dart samples: `{}`\n- Rust samples: `{}`\n\n| Metric | Unit | Dart p95 | Rust p95 | Result | Target | Status |\n|---|---|---:|---:|---:|---:|---:|\n",
        baseline.runner_id,
        baseline.os,
        baseline.arch,
        baseline.profile,
        baseline.fixture_id,
        baseline.fixture_sha256,
        baseline.samples,
        candidate.samples
    );
    for comparison in comparisons {
        out.push_str(&format!(
            "| `{}` | `{}` | {} | {} | {} | {} | `{}` |\n",
            comparison.name,
            comparison.unit,
            comparison.dart_p95,
            comparison.rust_p95,
            comparison.target.format_value(comparison.value),
            comparison.target.label(),
            if comparison.passed { "pass" } else { "fail" }
        ));
    }
    out.push_str("\nCache work remains deferred until this report identifies a measured warm-path or memory gap.\n");
    atomic_write(path, out.as_bytes())
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
struct Options {
    baseline: Option<PathBuf>,
    candidate: Option<PathBuf>,
    report: PathBuf,
    help: bool,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, XtaskError> {
        let mut options = Self {
            report: PathBuf::from("target/pagelet-bench/runtime-comparison.md"),
            ..Self::default()
        };
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--baseline" => {
                    index += 1;
                    options.baseline =
                        Some(PathBuf::from(args.get(index).ok_or_else(|| {
                            XtaskError::Usage("--baseline requires a value".into())
                        })?));
                }
                "--candidate" => {
                    index += 1;
                    options.candidate = Some(PathBuf::from(args.get(index).ok_or_else(|| {
                        XtaskError::Usage("--candidate requires a value".into())
                    })?));
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
                        "unknown compare-runtime option: {other}"
                    )))
                }
            }
            index += 1;
        }
        Ok(options)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Snapshot {
    runtime: String,
    runner_id: String,
    os: String,
    arch: String,
    profile: String,
    fixture_id: String,
    fixture_sha256: String,
    samples: u64,
    metrics: BTreeMap<String, Metric>,
}

impl Snapshot {
    fn read(path: &PathBuf) -> Result<Self, XtaskError> {
        let text = fs::read_to_string(path)?;
        Self::parse(&text)
            .map_err(|error| XtaskError::Command(format!("{}: {error}", path.display())))
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut metadata = BTreeMap::new();
        let mut metrics = BTreeMap::new();
        let mut metric_columns = None;
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "metric,unit,direction,policy,minimum_effect,p50,p95" {
                metric_columns = Some(7);
                continue;
            }
            if line == "metric,unit,p50,p95" {
                metric_columns = Some(4);
                continue;
            }
            let fields: Vec<_> = line.split(',').collect();
            match metric_columns {
                Some(7) if fields.len() == 7 => {
                    metrics.insert(
                        fields[0].to_owned(),
                        Metric {
                            unit: fields[1].to_owned(),
                            p50: parse_u64("p50", fields[5])?,
                            p95: parse_u64("p95", fields[6])?,
                        },
                    );
                }
                Some(4) if fields.len() == 4 => {
                    metrics.insert(
                        fields[0].to_owned(),
                        Metric {
                            unit: fields[1].to_owned(),
                            p50: parse_u64("p50", fields[2])?,
                            p95: parse_u64("p95", fields[3])?,
                        },
                    );
                }
                None if fields.len() == 2 => {
                    metadata.insert(fields[0].to_owned(), fields[1].to_owned());
                }
                _ => return Err(format!("invalid runtime snapshot row: {line}")),
            }
        }
        let runtime = metadata
            .remove("runtime")
            .or_else(|| {
                metadata
                    .contains_key("rust_toolchain")
                    .then(|| "rust".into())
            })
            .ok_or_else(|| "snapshot is missing runtime".to_owned())?;
        let schema_version = metadata
            .remove("schema_version")
            .ok_or_else(|| "snapshot is missing schema_version".to_owned())?;
        if schema_version != "1" {
            return Err(format!(
                "unsupported runtime snapshot schema: {schema_version}"
            ));
        }
        let mut required = |key: &str| {
            metadata
                .remove(key)
                .ok_or_else(|| format!("snapshot is missing {key}"))
        };
        let runner_id = required("runner_id")?;
        let os = required("os")?;
        let arch = required("arch")?;
        let profile = required("profile")?;
        let fixture_id = required("fixture_id")?;
        let fixture_sha256 = required("fixture_sha256")?;
        let samples = parse_u64("samples", &required("samples")?)?;
        if samples == 0 {
            return Err("snapshot samples must be positive".into());
        }
        if metrics.is_empty() {
            return Err("snapshot contains no metrics".into());
        }
        Ok(Self {
            runtime,
            runner_id,
            os,
            arch,
            profile,
            fixture_id,
            fixture_sha256,
            samples,
            metrics,
        })
    }

    fn ensure_same_boundary(&self, candidate: &Self) -> Result<(), XtaskError> {
        for (name, dart, rust) in [
            ("runner", &self.runner_id, &candidate.runner_id),
            ("os", &self.os, &candidate.os),
            ("arch", &self.arch, &candidate.arch),
            ("profile", &self.profile, &candidate.profile),
            ("fixture", &self.fixture_id, &candidate.fixture_id),
            (
                "fixture hash",
                &self.fixture_sha256,
                &candidate.fixture_sha256,
            ),
        ] {
            if dart != rust {
                return Err(XtaskError::Command(format!(
                    "same-boundary {name} mismatch: dart={dart}, rust={rust}"
                )));
            }
        }
        Ok(())
    }

    fn metric(&self, name: &str) -> Result<&Metric, XtaskError> {
        let metric = self.metrics.get(name).ok_or_else(|| {
            XtaskError::Command(format!(
                "{} snapshot is missing metric {name}",
                self.runtime
            ))
        })?;
        if metric.p50 > metric.p95 {
            return Err(XtaskError::Command(format!(
                "{} metric {name} has p50 greater than p95",
                self.runtime
            )));
        }
        if metric.p50 == 0 || metric.p95 == 0 {
            return Err(XtaskError::Command(format!(
                "{} metric {name} must be non-zero",
                self.runtime
            )));
        }
        Ok(metric)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Metric {
    unit: String,
    p50: u64,
    p95: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Target {
    ReductionPct(u64),
    Speedup(u64),
}

impl Target {
    fn value(self, baseline: u64, candidate: u64) -> u64 {
        match self {
            Self::ReductionPct(_) => {
                baseline.saturating_sub(candidate).saturating_mul(100) / baseline.max(1)
            }
            Self::Speedup(_) => baseline / candidate.max(1),
        }
    }

    fn passed(self, value: u64) -> bool {
        match self {
            Self::ReductionPct(target) | Self::Speedup(target) => value >= target,
        }
    }

    fn label(self) -> String {
        match self {
            Self::ReductionPct(value) => format!(">={value}% reduction"),
            Self::Speedup(value) => format!(">={value}x"),
        }
    }

    fn format_value(self, value: u64) -> String {
        match self {
            Self::ReductionPct(_) => format!("{value}% reduction"),
            Self::Speedup(_) => format!("{value}x"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Comparison {
    name: &'static str,
    unit: String,
    dart_p95: u64,
    rust_p95: u64,
    target: Target,
    value: u64,
    passed: bool,
}

fn parse_u64(name: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("invalid {name} value: {value}"))
}

fn print_help() {
    println!("Usage:");
    println!("  cargo xtask bench compare-runtime --baseline <dart.csv> --candidate <rust.csv> [--report <path>]");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(runtime: &str, rss: u64, first: u64, warm: u64) -> String {
        format!(
            "schema_version,1\nruntime,{runtime}\nrunner_id,pinned\nos,macos\narch,aarch64\nprofile,full\nfixture_id,small-novel\nfixture_sha256,abc\nsamples,30\nmetric,unit,p50,p95\npeak_rss_bytes,bytes,{rss},{rss}\nfirst_page_ready,ns,{first},{first}\nheight_only_repack,ns,{warm},{warm}\n"
        )
    }

    #[test]
    fn parses_generic_and_existing_rust_snapshot_headers() {
        let dart = Snapshot::parse(&snapshot("dart", 100, 100, 100)).expect("dart");
        let rust_text = snapshot("rust", 50, 40, 10)
            .replace("runtime,rust\n", "rust_toolchain,rustc 1.95.0\n")
            .replace(
                "metric,unit,p50,p95",
                "metric,unit,direction,policy,minimum_effect,p50,p95",
            )
            .replace(
                "peak_rss_bytes,bytes,50,50",
                "peak_rss_bytes,bytes,lower,block,1,50,50",
            )
            .replace(
                "first_page_ready,ns,40,40",
                "first_page_ready,ns,lower,block,1,40,40",
            )
            .replace(
                "height_only_repack,ns,10,10",
                "height_only_repack,ns,lower,block,1,10,10",
            );
        let rust = Snapshot::parse(&rust_text).expect("rust");

        assert_eq!(dart.runtime, "dart");
        assert_eq!(rust.runtime, "rust");
        dart.ensure_same_boundary(&rust).expect("same boundary");
    }

    #[test]
    fn stage_targets_match_memory_and_speedup_policy() {
        assert!(Target::ReductionPct(40).passed(Target::ReductionPct(40).value(100, 60)));
        assert!(Target::Speedup(2).passed(Target::Speedup(2).value(100, 50)));
        assert!(!Target::Speedup(5).passed(Target::Speedup(5).value(100, 21)));
    }
}
