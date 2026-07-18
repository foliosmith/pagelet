use std::{
    alloc::System,
    collections::BTreeMap,
    env, fs,
    hint::black_box,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use pagelet::{
    core::{CancellationToken, PageletError},
    engine::Engine,
    layout::{
        anchor_to_page, paginate_chapter_with_options, paginate_next_page,
        repaginate_incrementally, BreakToken, IncrementalPaginationOutcome,
        IncrementalPaginationRequest, LayoutGeneration, LayoutImpact, LayoutOptions, PageScene,
    },
    text::{FontSetFingerprint, MeasureBatch, MeasuredBatch, TextBackend, TextBackendId},
    wire::PageBatch,
};
use pagelet_testkit::{DeterministicTextBackend, Fixture, ValidEpubBuilder};
use sha2::{Digest, Sha256};
use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const DEFAULT_REGRESSION_PCT: u64 = 10;
const FIXTURE_ID: &str = "small-novel";
const LOCAL_RUNNER_ID: &str = "local-unpinned";

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let options = BenchReportOptions::parse(args)?;
    if options.help {
        print_help();
        return Ok(());
    }
    if let Some(child_snapshot) = &options.child_snapshot {
        let snapshot = collect_snapshot(&options)?;
        write_snapshot(child_snapshot, &snapshot)?;
        return Ok(());
    }

    if (options.baseline.is_some() || options.record_baseline.is_some()) && cfg!(debug_assertions) {
        return Err(
            "baseline comparison and recording require `cargo run --release -p xtask -- bench report ...`"
                .into(),
        );
    }
    if options.record_baseline.is_some() && options.runner_id == LOCAL_RUNNER_ID {
        return Err("recording a baseline requires --runner <pinned-runner-id>".into());
    }
    if let Some(path) = &options.record_baseline {
        if options.baseline.is_some() {
            return Err("--baseline and --record-baseline are mutually exclusive".into());
        }
        if options.reason.as_deref().is_none_or(str::is_empty) {
            return Err("--record-baseline requires --reason <message>".into());
        }
        if path.exists() && !options.replace_baseline {
            return Err(format!(
                "{} already exists; pass --replace-baseline with --reason to replace it",
                path.display()
            ));
        }
    } else if options.replace_baseline || options.reason.is_some() {
        return Err("--replace-baseline and --reason require --record-baseline".into());
    }

    let mut snapshot = collect_with_process_peak(&options)?;
    if options.record_baseline.is_some() {
        snapshot.reason = options.reason.clone();
    }
    snapshot
        .metrics
        .sort_by(|left, right| left.name.cmp(&right.name));

    let baseline = options.baseline.as_deref().map(read_snapshot).transpose()?;
    let comparison = baseline
        .as_ref()
        .map(|baseline| compare_snapshots(baseline, &snapshot))
        .transpose()?;

    write_snapshot(&options.snapshot, &snapshot)?;
    write_report(
        &options.report,
        &snapshot,
        baseline.as_ref(),
        comparison.as_ref(),
    )?;

    if let Some(path) = &options.record_baseline {
        write_snapshot(path, &snapshot)?;
        println!("recorded baseline: {}", path.display());
    }

    println!("benchmark snapshot: {}", options.snapshot.display());
    println!("benchmark report: {}", options.report.display());
    if let Some(comparison) = comparison {
        if comparison.blocking_regressions.is_empty() {
            println!("performance gate passed");
        } else {
            return Err(format!(
                "performance gate blocked by:\n{}",
                comparison.blocking_regressions.join("\n")
            ));
        }
    }
    Ok(())
}

fn print_help() {
    println!("Usage:");
    println!("  cargo run --release -p xtask -- bench report [options]");
    println!();
    println!("Options:");
    println!("  --profile smoke|full        5 or 30 samples by default");
    println!("  --samples <n>               Override lifecycle sample count");
    println!("  --runner <id>               Stable pinned-runner identity");
    println!("  --baseline <path>           Compare against an existing snapshot");
    println!("  --snapshot <path>           Write machine-readable CSV snapshot");
    println!("  --report <path>             Write Markdown performance report");
    println!("  --record-baseline <path>    Record the current pinned snapshot");
    println!("  --replace-baseline          Permit replacing an existing baseline");
    println!("  --reason <message>          Required when recording a baseline");
    println!("  --incremental-cases         Include incremental repagination observations");
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct BenchReportOptions {
    profile: String,
    samples: u32,
    runner_id: String,
    baseline: Option<PathBuf>,
    snapshot: PathBuf,
    report: PathBuf,
    record_baseline: Option<PathBuf>,
    replace_baseline: bool,
    reason: Option<String>,
    child_snapshot: Option<PathBuf>,
    incremental_cases: bool,
    help: bool,
}

impl BenchReportOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut profile = "smoke".to_owned();
        let mut samples = None;
        let mut runner_id =
            env::var("PAGELET_PERF_RUNNER_ID").unwrap_or_else(|_| LOCAL_RUNNER_ID.to_owned());
        let mut baseline = None;
        let mut snapshot = PathBuf::from("target/pagelet-bench/current.csv");
        let mut report = PathBuf::from("target/pagelet-bench/report.md");
        let mut record_baseline = None;
        let mut replace_baseline = false;
        let mut reason = None;
        let mut child_snapshot = None;
        let mut incremental_cases = false;
        let mut help = false;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--profile" => profile = option_value(args, &mut index, "--profile")?.to_owned(),
                "--samples" => {
                    let value = option_value(args, &mut index, "--samples")?;
                    let parsed = value
                        .parse::<u32>()
                        .map_err(|_| "--samples requires a positive integer".to_owned())?;
                    if parsed == 0 {
                        return Err("--samples requires a positive integer".into());
                    }
                    samples = Some(parsed);
                }
                "--runner" => runner_id = option_value(args, &mut index, "--runner")?.to_owned(),
                "--baseline" => {
                    baseline = Some(PathBuf::from(option_value(args, &mut index, "--baseline")?))
                }
                "--snapshot" => {
                    snapshot = PathBuf::from(option_value(args, &mut index, "--snapshot")?)
                }
                "--report" => report = PathBuf::from(option_value(args, &mut index, "--report")?),
                "--record-baseline" => {
                    record_baseline = Some(PathBuf::from(option_value(
                        args,
                        &mut index,
                        "--record-baseline",
                    )?))
                }
                "--replace-baseline" => replace_baseline = true,
                "--reason" => reason = Some(option_value(args, &mut index, "--reason")?.to_owned()),
                "--child-snapshot" => {
                    child_snapshot = Some(PathBuf::from(option_value(
                        args,
                        &mut index,
                        "--child-snapshot",
                    )?))
                }
                "--incremental-cases" => incremental_cases = true,
                "-h" | "--help" | "help" => help = true,
                other => return Err(format!("unknown bench report option: {other}")),
            }
            index += 1;
        }
        if !matches!(profile.as_str(), "smoke" | "full") {
            return Err(format!("unknown benchmark report profile: {profile}"));
        }
        if runner_id.is_empty() || runner_id.contains([',', '\n', '\r']) {
            return Err("--runner must be a non-empty single CSV-safe value".into());
        }
        if reason
            .as_deref()
            .is_some_and(|reason| reason.contains([',', '\n', '\r']))
        {
            return Err("--reason must be a single CSV-safe value".into());
        }
        let samples = samples.unwrap_or(if profile == "smoke" { 5 } else { 30 });
        Ok(Self {
            profile,
            samples,
            runner_id,
            baseline,
            snapshot,
            report,
            record_baseline,
            replace_baseline,
            reason,
            child_snapshot,
            incremental_cases,
            help,
        })
    }

    fn child_args(&self, path: &Path) -> Vec<String> {
        let mut args = vec![
            "bench".into(),
            "report".into(),
            "--profile".into(),
            self.profile.clone(),
            "--samples".into(),
            self.samples.to_string(),
            "--runner".into(),
            self.runner_id.clone(),
            "--child-snapshot".into(),
            path.display().to_string(),
        ];
        if self.incremental_cases {
            args.push("--incremental-cases".into());
        }
        args
    }
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn collect_with_process_peak(options: &BenchReportOptions) -> Result<Snapshot, String> {
    let temp_dir = Path::new("target/pagelet-bench");
    fs::create_dir_all(temp_dir).map_err(io_error)?;
    let child_path = temp_dir.join(format!(".measure-{}.csv", std::process::id()));
    let executable = env::current_exe().map_err(io_error)?;
    let child_args = options.child_args(&child_path);
    let profiled = profile_command(&executable, &child_args);

    let (mut snapshot, peak_rss) = match profiled {
        Ok(peak_rss) => {
            let snapshot = read_snapshot(&child_path)?;
            if peak_rss == 0 {
                return Err("process profiler returned zero peak RSS".into());
            }
            (snapshot, peak_rss)
        }
        Err(error) if options.baseline.is_none() && options.record_baseline.is_none() => {
            eprintln!("warning: {error}; using in-process RSS observation");
            let snapshot = collect_snapshot(options)?;
            (snapshot, current_rss_bytes().unwrap_or(0))
        }
        Err(error) => return Err(error),
    };
    let _ = fs::remove_file(&child_path);
    snapshot.upsert_metric(MetricSummary::single(
        metric_spec("peak_rss_bytes"),
        peak_rss,
    ));
    Ok(snapshot)
}

fn profile_command(executable: &Path, args: &[String]) -> Result<u64, String> {
    if !matches!(env::consts::OS, "macos" | "linux") {
        return Err(format!(
            "peak RSS profiler is not supported on {}",
            env::consts::OS
        ));
    }
    let mut child = Command::new(executable)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch benchmark child: {error}"))?;
    let mut peak_rss = 0;
    let status = loop {
        if let Some(rss) = process_rss_bytes(child.id()) {
            peak_rss = peak_rss.max(rss);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll benchmark child: {error}"))?
        {
            break status;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let mut stderr = String::new();
    if let Some(mut child_stderr) = child.stderr.take() {
        child_stderr
            .read_to_string(&mut stderr)
            .map_err(|error| format!("failed to read benchmark child stderr: {error}"))?;
    }
    if !status.success() {
        return Err(format!("benchmark measurement child failed:\n{stderr}"));
    }
    Ok(peak_rss)
}

fn process_rss_bytes(process_id: u32) -> Option<u64> {
    if env::consts::OS == "linux" {
        let status = fs::read_to_string(format!("/proc/{process_id}/status")).ok()?;
        return parse_linux_rss(&status);
    }
    if env::consts::OS == "macos" {
        let output = Command::new("ps")
            .args(["-o", "rss=", "-p", &process_id.to_string()])
            .output()
            .ok()?;
        return parse_ps_rss(&String::from_utf8_lossy(&output.stdout));
    }
    None
}

fn current_rss_bytes() -> Option<u64> {
    if env::consts::OS == "linux" {
        let status = fs::read_to_string("/proc/self/status").ok()?;
        return parse_linux_rss(&status);
    }
    if env::consts::OS == "macos" {
        let output = Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        return parse_ps_rss(&String::from_utf8_lossy(&output.stdout));
    }
    None
}

fn parse_linux_rss(status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .or_else(|| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kib| kib.saturating_mul(1024))
    })
}

fn parse_ps_rss(output: &str) -> Option<u64> {
    output
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kib| kib.saturating_mul(1024))
}

fn collect_snapshot(options: &BenchReportOptions) -> Result<Snapshot, String> {
    let fixture = benchmark_fixture();
    let fixture_hash = sha256_hex(fixture.bytes());
    let mut samples = MetricSamples::default();
    measure_iteration(
        &fixture,
        &mut MetricSamples::default(),
        options.incremental_cases,
    )?;
    for _ in 0..options.samples {
        measure_iteration(&fixture, &mut samples, options.incremental_cases)?;
    }
    Ok(Snapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        runner_id: options.runner_id.clone(),
        os: env::consts::OS.to_owned(),
        arch: env::consts::ARCH.to_owned(),
        rust_toolchain: rust_toolchain()?,
        profile: options.profile.clone(),
        fixture_id: FIXTURE_ID.to_owned(),
        fixture_sha256: fixture_hash,
        samples: options.samples,
        reason: None,
        metrics: samples.summaries(),
    })
}

fn benchmark_fixture() -> Fixture {
    let mut body = String::from("<h1 id=\"start\">Pinned benchmark chapter</h1>");
    for index in 0..72 {
        body.push_str(&format!(
            "<p id=\"p-{index}\">Paragraph {index}. Deterministic pagination text covers Latin, 中文, العربية, and emoji 🧭. The same payload is repeated so page boundaries and wire size remain stable.</p>"
        ));
    }
    ValidEpubBuilder::epub3("pagelet-pinned-small-novel")
        .feature("pinned-performance")
        .xhtml("EPUB/chapter-1.xhtml", "Pinned benchmark", &body)
        .build()
}

fn measure_iteration(
    fixture: &Fixture,
    samples: &mut MetricSamples,
    incremental_cases: bool,
) -> Result<(), String> {
    let bytes = fixture.bytes();
    let engine = Engine::new();
    let backend = DeterministicTextBackend::new();
    let options = LayoutOptions::default();

    let started = Instant::now();
    let metadata_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    black_box(metadata_book.summary());
    samples.push("open_to_metadata", elapsed_ns(started));

    let started = Instant::now();
    let navigation_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    black_box(navigation_book.navigation());
    samples.push("open_to_navigation", elapsed_ns(started));

    let chapter_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    let started = Instant::now();
    let chapter = chapter_book.open_spine_item(0).map_err(pagelet_error)?;
    black_box(&chapter);
    samples.push("chapter_to_ir", elapsed_ns(started));

    let first_page_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    let first_page_chapter = first_page_book.open_spine_item(0).map_err(pagelet_error)?;
    let started = Instant::now();
    let first_page = paginate_next_page(&first_page_chapter, &backend, options, None)
        .map_err(pagelet_error)?
        .ok_or_else(|| "benchmark chapter produced no first page".to_owned())?;
    black_box(&first_page);
    samples.push("first_page_ready", elapsed_ns(started));

    let batch_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    let batch_chapter = batch_book.open_spine_item(0).map_err(pagelet_error)?;
    let started = Instant::now();
    let pages = paginate_page_batch(&batch_chapter, &backend, options, 3)?;
    let page_batch_elapsed = elapsed_ns(started);
    if pages.len() != 3 {
        return Err(format!(
            "benchmark fixture produced {} pages; expected at least 3",
            pages.len()
        ));
    }
    samples.push("page_batch_ready", page_batch_elapsed);
    let wire = PageBatch::new(pages).encode().map_err(other_error)?;
    samples.push("wire_bytes", usize_to_u64(wire.len()));

    let full_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    let full_chapter = full_book.open_spine_item(0).map_err(pagelet_error)?;
    let started = Instant::now();
    let document =
        paginate_chapter_with_options(&full_chapter, &backend, options).map_err(pagelet_error)?;
    samples.push("full_chapter_paginate", elapsed_ns(started));

    if incremental_cases {
        measure_height_only_repack(&full_chapter, &document, &backend, options, samples)?;
    }

    let anchor = document
        .pages
        .last()
        .and_then(|page| page.start_anchor.or(page.end_anchor))
        .ok_or_else(|| "benchmark pages contain no text anchor".to_owned())?;
    const ANCHOR_LOOKUPS: u64 = 256;
    let started = Instant::now();
    for _ in 0..ANCHOR_LOOKUPS {
        black_box(anchor_to_page(&document.pages, anchor));
    }
    samples.push(
        "anchor_to_page",
        elapsed_ns(started).saturating_div(ANCHOR_LOOKUPS),
    );

    let cache_book = engine.open_bytes(bytes.to_vec()).map_err(pagelet_error)?;
    black_box(cache_book.open_spine_item(0).map_err(pagelet_error)?);
    black_box(cache_book.open_spine_item(0).map_err(pagelet_error)?);
    let stats = cache_book.stats();
    samples.push("chapter_cache_hits", stats.chapter_cache_hits);
    samples.push("chapter_cache_misses", stats.chapter_cache_misses);

    let allocation_region = Region::new(GLOBAL);
    let allocation_wire_bytes = cold_page_batch_wire(bytes, &backend, options)?;
    black_box(allocation_wire_bytes);
    let allocation_stats = allocation_region.change();
    samples.push(
        "allocated_bytes",
        usize_to_u64(allocation_stats.bytes_allocated),
    );
    Ok(())
}

fn measure_height_only_repack(
    chapter: &pagelet::document::ChapterIr,
    document: &pagelet::layout::PaginatedDocument,
    backend: &DeterministicTextBackend,
    options: LayoutOptions,
    samples: &mut MetricSamples,
) -> Result<(), String> {
    let repack_options = LayoutOptions {
        constraints: pagelet::layout::LayoutConstraints {
            viewport_height: options.constraints.viewport_height
                - pagelet::core::LayoutUnit::from_px(80),
            margin_top: options.constraints.margin_top + pagelet::core::LayoutUnit::from_px(8),
            margin_bottom: options.constraints.margin_bottom
                + pagelet::core::LayoutUnit::from_px(8),
            ..options.constraints
        },
        ..options
    };
    let cached_only = CachedOnlyTextBackend {
        backend_id: backend.backend_id(),
        font_fingerprint: backend.font_fingerprint(),
    };
    let started = Instant::now();
    let repacked = repaginate_incrementally(
        IncrementalPaginationRequest::new(
            chapter,
            document,
            options,
            chapter,
            repack_options,
            LayoutGeneration::new(1),
            LayoutGeneration::new(1),
        ),
        &cached_only,
    )
    .map_err(pagelet_error)?;
    let IncrementalPaginationOutcome::Applied(repacked) = repacked else {
        return Err("height-only repack unexpectedly became stale".into());
    };
    if repacked.report.impact != LayoutImpact::RepackOnly
        || repacked.report.text_measurements_reused == 0
    {
        return Err("height-only repack did not reuse cached text measurements".into());
    }
    black_box(&repacked.document);
    samples.push("height_only_repack", elapsed_ns(started));
    Ok(())
}

struct CachedOnlyTextBackend {
    backend_id: TextBackendId,
    font_fingerprint: FontSetFingerprint,
}

impl TextBackend for CachedOnlyTextBackend {
    fn backend_id(&self) -> TextBackendId {
        self.backend_id
    }

    fn font_fingerprint(&self) -> FontSetFingerprint {
        self.font_fingerprint
    }

    fn measure_batch(
        &self,
        _request: &MeasureBatch,
        _cancel: &CancellationToken,
    ) -> Result<MeasuredBatch, PageletError> {
        panic!("height-only repack attempted to reshape text")
    }
}

fn cold_page_batch_wire(
    bytes: &[u8],
    backend: &DeterministicTextBackend,
    options: LayoutOptions,
) -> Result<usize, String> {
    let book = Engine::new()
        .open_bytes(bytes.to_vec())
        .map_err(pagelet_error)?;
    let chapter = book.open_spine_item(0).map_err(pagelet_error)?;
    let pages = paginate_page_batch(&chapter, backend, options, 3)?;
    PageBatch::new(pages)
        .encode()
        .map(|wire| wire.len())
        .map_err(other_error)
}

fn paginate_page_batch(
    chapter: &pagelet::document::ChapterIr,
    backend: &DeterministicTextBackend,
    options: LayoutOptions,
    max_pages: usize,
) -> Result<Vec<PageScene>, String> {
    let mut pages = Vec::with_capacity(max_pages);
    let mut token: Option<BreakToken> = None;
    while pages.len() < max_pages {
        let Some(page) =
            paginate_next_page(chapter, backend, options, token).map_err(pagelet_error)?
        else {
            break;
        };
        token = page.next_break_token.clone();
        let complete = token.is_none();
        pages.push(page);
        if complete {
            break;
        }
    }
    Ok(pages)
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn rust_toolchain() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|error| format!("failed to read rustc version: {error}"))?;
    if !output.status.success() {
        return Err("rustc --version failed".into());
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("rustc --version was not UTF-8: {error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn pagelet_error(error: pagelet::core::PageletError) -> String {
    format!("benchmark lifecycle failed: {error}")
}

fn other_error(error: impl std::fmt::Display) -> String {
    format!("benchmark lifecycle failed: {error}")
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

#[derive(Debug, Default)]
struct MetricSamples {
    values: BTreeMap<&'static str, Vec<u64>>,
}

impl MetricSamples {
    fn push(&mut self, name: &'static str, value: u64) {
        self.values.entry(name).or_default().push(value);
    }

    fn summaries(self) -> Vec<MetricSummary> {
        self.values
            .into_iter()
            .map(|(name, values)| MetricSummary::from_values(metric_spec(name), values))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Direction {
    Lower,
    Higher,
}

impl Direction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lower => "lower",
            Self::Higher => "higher",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "lower" => Ok(Self::Lower),
            "higher" => Ok(Self::Higher),
            _ => Err(format!("unknown metric direction: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum GatePolicy {
    Block,
    Observe,
}

impl GatePolicy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Observe => "observe",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "block" => Ok(Self::Block),
            "observe" => Ok(Self::Observe),
            _ => Err(format!("unknown metric policy: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MetricSpec {
    name: &'static str,
    unit: &'static str,
    direction: Direction,
    policy: GatePolicy,
    minimum_effect: u64,
}

fn metric_spec(name: &str) -> MetricSpec {
    match name {
        "open_to_metadata"
        | "open_to_navigation"
        | "chapter_to_ir"
        | "first_page_ready"
        | "page_batch_ready"
        | "full_chapter_paginate"
        | "height_only_repack" => MetricSpec {
            name: match name {
                "open_to_metadata" => "open_to_metadata",
                "open_to_navigation" => "open_to_navigation",
                "chapter_to_ir" => "chapter_to_ir",
                "first_page_ready" => "first_page_ready",
                "page_batch_ready" => "page_batch_ready",
                "full_chapter_paginate" => "full_chapter_paginate",
                _ => "height_only_repack",
            },
            unit: "ns",
            direction: Direction::Lower,
            policy: GatePolicy::Block,
            minimum_effect: 10_000,
        },
        "anchor_to_page" => MetricSpec {
            name: "anchor_to_page",
            unit: "ns",
            direction: Direction::Lower,
            policy: GatePolicy::Block,
            minimum_effect: 5,
        },
        "allocated_bytes" => MetricSpec {
            name: "allocated_bytes",
            unit: "bytes",
            direction: Direction::Lower,
            policy: GatePolicy::Block,
            minimum_effect: 16 * 1024,
        },
        "peak_rss_bytes" => MetricSpec {
            name: "peak_rss_bytes",
            unit: "bytes",
            direction: Direction::Lower,
            policy: GatePolicy::Block,
            minimum_effect: 256 * 1024,
        },
        "wire_bytes" => MetricSpec {
            name: "wire_bytes",
            unit: "bytes",
            direction: Direction::Lower,
            policy: GatePolicy::Block,
            minimum_effect: 256,
        },
        "chapter_cache_hits" => MetricSpec {
            name: "chapter_cache_hits",
            unit: "count",
            direction: Direction::Higher,
            policy: GatePolicy::Observe,
            minimum_effect: 1,
        },
        "chapter_cache_misses" => MetricSpec {
            name: "chapter_cache_misses",
            unit: "count",
            direction: Direction::Lower,
            policy: GatePolicy::Observe,
            minimum_effect: 1,
        },
        _ => panic!("unknown benchmark metric: {name}"),
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct MetricSummary {
    name: String,
    unit: String,
    direction: Direction,
    policy: GatePolicy,
    minimum_effect: u64,
    p50: u64,
    p95: u64,
}

impl MetricSummary {
    fn from_values(spec: MetricSpec, mut values: Vec<u64>) -> Self {
        values.sort_unstable();
        Self {
            name: spec.name.to_owned(),
            unit: spec.unit.to_owned(),
            direction: spec.direction,
            policy: spec.policy,
            minimum_effect: spec.minimum_effect,
            p50: percentile(&values, 50),
            p95: percentile(&values, 95),
        }
    }

    fn single(spec: MetricSpec, value: u64) -> Self {
        Self::from_values(spec, vec![value])
    }
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    assert!(!values.is_empty());
    let rank = values.len().saturating_mul(percentile).saturating_add(99) / 100;
    values[rank.saturating_sub(1).min(values.len() - 1)]
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Snapshot {
    schema_version: u32,
    runner_id: String,
    os: String,
    arch: String,
    rust_toolchain: String,
    profile: String,
    fixture_id: String,
    fixture_sha256: String,
    samples: u32,
    reason: Option<String>,
    metrics: Vec<MetricSummary>,
}

impl Snapshot {
    fn upsert_metric(&mut self, metric: MetricSummary) {
        if let Some(existing) = self
            .metrics
            .iter_mut()
            .find(|existing| existing.name == metric.name)
        {
            *existing = metric;
        } else {
            self.metrics.push(metric);
        }
    }

    fn metric(&self, name: &str) -> Option<&MetricSummary> {
        self.metrics.iter().find(|metric| metric.name == name)
    }

    fn serialize(&self) -> String {
        let mut out = String::new();
        push_metadata(&mut out, "schema_version", &self.schema_version.to_string());
        push_metadata(&mut out, "runner_id", &self.runner_id);
        push_metadata(&mut out, "os", &self.os);
        push_metadata(&mut out, "arch", &self.arch);
        push_metadata(&mut out, "rust_toolchain", &self.rust_toolchain);
        push_metadata(&mut out, "profile", &self.profile);
        push_metadata(&mut out, "fixture_id", &self.fixture_id);
        push_metadata(&mut out, "fixture_sha256", &self.fixture_sha256);
        push_metadata(&mut out, "samples", &self.samples.to_string());
        if let Some(reason) = &self.reason {
            push_metadata(&mut out, "reason", reason);
        }
        out.push_str("metric,unit,direction,policy,minimum_effect,p50,p95\n");
        for metric in &self.metrics {
            out.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                metric.name,
                metric.unit,
                metric.direction.as_str(),
                metric.policy.as_str(),
                metric.minimum_effect,
                metric.p50,
                metric.p95
            ));
        }
        out
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut metadata = BTreeMap::new();
        let mut metrics = Vec::new();
        let mut in_metrics = false;
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "metric,unit,direction,policy,minimum_effect,p50,p95" {
                in_metrics = true;
                continue;
            }
            let fields: Vec<_> = line.split(',').collect();
            if in_metrics {
                if fields.len() != 7 {
                    return Err(format!("invalid benchmark metric row: {line}"));
                }
                metrics.push(MetricSummary {
                    name: fields[0].to_owned(),
                    unit: fields[1].to_owned(),
                    direction: Direction::parse(fields[2])?,
                    policy: GatePolicy::parse(fields[3])?,
                    minimum_effect: parse_u64("minimum_effect", fields[4])?,
                    p50: parse_u64("p50", fields[5])?,
                    p95: parse_u64("p95", fields[6])?,
                });
            } else if fields.len() == 2 {
                metadata.insert(fields[0].to_owned(), fields[1].to_owned());
            } else {
                return Err(format!("invalid benchmark metadata row: {line}"));
            }
        }
        let schema_version = take_metadata(&mut metadata, "schema_version")?
            .parse::<u32>()
            .map_err(|_| "invalid snapshot schema_version".to_owned())?;
        if schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported snapshot schema version: {schema_version}"
            ));
        }
        if metrics.is_empty() {
            return Err("benchmark snapshot contains no metrics".into());
        }
        Ok(Self {
            schema_version,
            runner_id: take_metadata(&mut metadata, "runner_id")?,
            os: take_metadata(&mut metadata, "os")?,
            arch: take_metadata(&mut metadata, "arch")?,
            rust_toolchain: take_metadata(&mut metadata, "rust_toolchain")?,
            profile: take_metadata(&mut metadata, "profile")?,
            fixture_id: take_metadata(&mut metadata, "fixture_id")?,
            fixture_sha256: take_metadata(&mut metadata, "fixture_sha256")?,
            samples: take_metadata(&mut metadata, "samples")?
                .parse::<u32>()
                .map_err(|_| "invalid snapshot samples".to_owned())?,
            reason: metadata.remove("reason"),
            metrics,
        })
    }
}

fn push_metadata(out: &mut String, key: &str, value: &str) {
    assert!(!value.contains([',', '\n', '\r']));
    out.push_str(key);
    out.push(',');
    out.push_str(value);
    out.push('\n');
}

fn take_metadata(metadata: &mut BTreeMap<String, String>, key: &str) -> Result<String, String> {
    metadata
        .remove(key)
        .ok_or_else(|| format!("benchmark snapshot is missing {key}"))
}

fn parse_u64(name: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("invalid {name} value: {value}"))
}

fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    ensure_parent(path)?;
    fs::write(path, snapshot.serialize()).map_err(io_error)
}

fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Snapshot::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct MetricComparison {
    name: String,
    baseline_p95: u64,
    current_p95: u64,
    delta_pct: i64,
    blocked: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Comparison {
    metrics: Vec<MetricComparison>,
    blocking_regressions: Vec<String>,
}

fn compare_snapshots(baseline: &Snapshot, current: &Snapshot) -> Result<Comparison, String> {
    for (name, expected, actual) in [
        ("runner", &baseline.runner_id, &current.runner_id),
        ("os", &baseline.os, &current.os),
        ("arch", &baseline.arch, &current.arch),
        (
            "rust toolchain",
            &baseline.rust_toolchain,
            &current.rust_toolchain,
        ),
        ("profile", &baseline.profile, &current.profile),
        ("fixture", &baseline.fixture_id, &current.fixture_id),
        (
            "fixture hash",
            &baseline.fixture_sha256,
            &current.fixture_sha256,
        ),
    ] {
        if expected != actual {
            return Err(format!(
                "benchmark {name} mismatch: baseline={expected}, current={actual}"
            ));
        }
    }

    let mut metrics = Vec::new();
    let mut blocking_regressions = Vec::new();
    for baseline_metric in &baseline.metrics {
        let current_metric = current.metric(&baseline_metric.name).ok_or_else(|| {
            format!(
                "current snapshot is missing baseline metric {}",
                baseline_metric.name
            )
        })?;
        if baseline_metric.unit != current_metric.unit
            || baseline_metric.direction != current_metric.direction
            || baseline_metric.policy != current_metric.policy
            || baseline_metric.minimum_effect != current_metric.minimum_effect
        {
            return Err(format!(
                "metric contract changed for {}",
                baseline_metric.name
            ));
        }
        let regressed = is_repeatable_regression(baseline_metric, current_metric);
        let blocked = regressed && current_metric.policy == GatePolicy::Block;
        let delta_pct = percent_delta(
            baseline_metric.p95,
            current_metric.p95,
            current_metric.direction,
        );
        if blocked {
            blocking_regressions.push(format!(
                "{} regressed {}% (baseline p95 {}, current p95 {})",
                current_metric.name, delta_pct, baseline_metric.p95, current_metric.p95
            ));
        }
        metrics.push(MetricComparison {
            name: current_metric.name.clone(),
            baseline_p95: baseline_metric.p95,
            current_p95: current_metric.p95,
            delta_pct,
            blocked,
        });
    }
    Ok(Comparison {
        metrics,
        blocking_regressions,
    })
}

fn is_repeatable_regression(baseline: &MetricSummary, current: &MetricSummary) -> bool {
    let (p50_regression, p95_regression, p95_effect) = match current.direction {
        Direction::Lower => (
            exceeds_pct(baseline.p50, current.p50, DEFAULT_REGRESSION_PCT),
            exceeds_pct(baseline.p95, current.p95, DEFAULT_REGRESSION_PCT),
            current.p95.saturating_sub(baseline.p95),
        ),
        Direction::Higher => (
            exceeds_pct(current.p50, baseline.p50, DEFAULT_REGRESSION_PCT),
            exceeds_pct(current.p95, baseline.p95, DEFAULT_REGRESSION_PCT),
            baseline.p95.saturating_sub(current.p95),
        ),
    };
    p50_regression && p95_regression && p95_effect >= current.minimum_effect
}

fn exceeds_pct(baseline: u64, current: u64, threshold_pct: u64) -> bool {
    u128::from(current).saturating_mul(100)
        > u128::from(baseline).saturating_mul(u128::from(100 + threshold_pct))
}

fn percent_delta(baseline: u64, current: u64, direction: Direction) -> i64 {
    if baseline == 0 {
        return if current == 0 { 0 } else { i64::MAX };
    }
    let raw =
        (i128::from(current) - i128::from(baseline)).saturating_mul(100) / i128::from(baseline);
    let signed = match direction {
        Direction::Lower => raw,
        Direction::Higher => -raw,
    };
    i64::try_from(signed).unwrap_or(if signed.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

fn write_report(
    path: &Path,
    current: &Snapshot,
    baseline: Option<&Snapshot>,
    comparison: Option<&Comparison>,
) -> Result<(), String> {
    let mut out = String::new();
    out.push_str("# Pagelet performance report\n\n");
    out.push_str(&format!("- Runner: `{}`\n", current.runner_id));
    out.push_str(&format!("- Platform: `{}/{}`\n", current.os, current.arch));
    out.push_str(&format!("- Rust: `{}`\n", current.rust_toolchain));
    out.push_str(&format!(
        "- Fixture: `{}` (`{}`)\n",
        current.fixture_id, current.fixture_sha256
    ));
    out.push_str(&format!("- Samples: `{}`\n", current.samples));
    if let Some(reason) = current
        .reason
        .as_ref()
        .or_else(|| baseline.and_then(|baseline| baseline.reason.as_ref()))
    {
        out.push_str(&format!("- Baseline reason: `{reason}`\n"));
    }
    out.push_str(&format!(
        "- Gate: `{}`\n\n",
        if let Some(comparison) = comparison {
            if comparison.blocking_regressions.is_empty() {
                "passed"
            } else {
                "blocked"
            }
        } else {
            "observation only (no baseline)"
        }
    ));
    out.push_str("| Metric | p50 | p95 | Baseline p95 | Regression | Policy |\n");
    out.push_str("| --- | ---: | ---: | ---: | ---: | --- |\n");
    for metric in &current.metrics {
        let compared = comparison.and_then(|comparison| {
            comparison
                .metrics
                .iter()
                .find(|row| row.name == metric.name)
        });
        let baseline_p95 = baseline
            .and_then(|baseline| baseline.metric(&metric.name))
            .map_or_else(
                || "—".to_owned(),
                |metric| format_value(metric.p95, &metric.unit),
            );
        let delta = compared.map_or_else(
            || "—".to_owned(),
            |row| {
                format!(
                    "{}{}%",
                    if row.delta_pct > 0 { "+" } else { "" },
                    row.delta_pct
                )
            },
        );
        let policy = if compared.is_some_and(|row| row.blocked) {
            "block"
        } else {
            metric.policy.as_str()
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            metric.name,
            format_value(metric.p50, &metric.unit),
            format_value(metric.p95, &metric.unit),
            baseline_p95,
            delta,
            policy
        ));
    }
    out.push_str("\nThe gate blocks only when both p50 and p95 regress by more than 10% and the p95 absolute change exceeds the metric's minimum effect. Cache counters remain observation-only until the layered cache task is complete.\n");
    ensure_parent(path)?;
    fs::write(path, out).map_err(io_error)
}

fn format_value(value: u64, unit: &str) -> String {
    match unit {
        "ns" if value >= 1_000_000 => format!("{:.3} ms", value as f64 / 1_000_000.0),
        "ns" if value >= 1_000 => format!("{:.3} µs", value as f64 / 1_000.0),
        "ns" => format!("{value} ns"),
        "bytes" if value >= 1024 * 1024 => {
            format!("{:.2} MiB", value as f64 / (1024.0 * 1024.0))
        }
        "bytes" if value >= 1024 => format!("{:.2} KiB", value as f64 / 1024.0),
        "bytes" => format!("{value} B"),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snapshot() -> Snapshot {
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            runner_id: "pinned".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            rust_toolchain: "rustc 1.95.0".into(),
            profile: "full".into(),
            fixture_id: FIXTURE_ID.into(),
            fixture_sha256: "abc".into(),
            samples: 30,
            reason: Some("initial baseline".into()),
            metrics: vec![MetricSummary {
                name: "first_page_ready".into(),
                unit: "ns".into(),
                direction: Direction::Lower,
                policy: GatePolicy::Block,
                minimum_effect: 50,
                p50: 1_000,
                p95: 1_100,
            }],
        }
    }

    #[test]
    fn snapshot_round_trips() {
        let snapshot = test_snapshot();
        assert_eq!(Snapshot::parse(&snapshot.serialize()).unwrap(), snapshot);
    }

    #[test]
    fn gate_requires_repeatable_relative_and_absolute_regression() {
        let baseline = test_snapshot();
        let mut current = baseline.clone();
        current.metrics[0].p50 = 1_120;
        current.metrics[0].p95 = 1_250;
        let comparison = compare_snapshots(&baseline, &current).unwrap();
        assert_eq!(comparison.blocking_regressions.len(), 1);

        current.metrics[0].p50 = 1_050;
        let comparison = compare_snapshots(&baseline, &current).unwrap();
        assert!(comparison.blocking_regressions.is_empty());
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = [10, 20, 30, 40, 50];
        assert_eq!(percentile(&values, 50), 30);
        assert_eq!(percentile(&values, 95), 50);
    }

    #[test]
    fn rss_output_parses_on_pinned_platforms() {
        assert_eq!(parse_ps_rss("  12345\n"), Some(12_345 * 1024));
        assert_eq!(
            parse_linux_rss("Name:\tpagelet\nVmHWM:\t1024 kB\n"),
            Some(1024 * 1024)
        );
    }
}
