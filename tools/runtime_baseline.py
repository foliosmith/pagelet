"""Measure real EPUB parsing with pagelet release and Flow Read's Dart AOT parser."""

import argparse
import hashlib
import json
import math
import platform
import re
import shutil
import subprocess
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def run(command, **kwargs):
    try:
        return subprocess.run(command, check=True, text=True, capture_output=True,
                              timeout=600, **kwargs).stdout.strip()
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"{command}:\n{error.stdout}\n{error.stderr}") from error


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def revision(root):
    return {"commit": run(["git", "-C", str(root), "rev-parse", "HEAD"]),
            "dirty": bool(run(["git", "-C", str(root), "status", "--porcelain"]))}


def measure(command, log):
    result = subprocess.run(["/usr/bin/time", "-l", *map(str, command)],
                            text=True, capture_output=True, timeout=180)
    log.with_suffix(".stdout").write_text(result.stdout)
    log.with_suffix(".stderr").write_text(result.stderr)
    if result.returncode:
        raise RuntimeError(f"sample exited {result.returncode}; see {log}.stderr")
    sample = json.loads(result.stdout)
    rss = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", result.stderr, re.M)
    if not rss:
        raise RuntimeError("missing process peak RSS")
    sample["peak_rss_bytes"] = int(rss[1])
    for metric in ("parse_ns", "chapters", "visible_chars", "peak_rss_bytes"):
        if type(sample.get(metric)) is not int or sample[metric] <= 0:
            raise RuntimeError(f"invalid or empty sample: {metric}={sample.get(metric)}")
    return sample


def percentile(values, p):
    return sorted(values)[math.ceil(len(values) * p) - 1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--flow-read", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=REPO / "tests/private-corpus.toml")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runner", required=True)
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--dart", default="dart")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this collector uses macOS /usr/bin/time -l (RSS in bytes)")
    if args.samples < 1:
        parser.error("--samples must be positive")
    root = args.root.resolve(strict=True)
    books = tomllib.loads(args.manifest.read_text())["books"]
    if not books or len({b["id"] for b in books}) != len(books):
        parser.error("manifest must have nonempty unique book IDs")
    for book in books:
        path = (root / book["path"]).resolve(strict=True)
        if not path.is_relative_to(root) or sha(path) != book["sha256"]:
            parser.error(f"unsafe path or fixture hash mismatch: {book['id']}")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    flow = args.flow_read.resolve(strict=True)
    run(["cargo", "build", "--locked", "--release", "-p", "xtask"], cwd=REPO)
    dart_binary = output / "dart-parse"
    run([args.dart, "compile", "exe", f"--packages={flow / '.dart_tool/package_config.json'}",
         str(REPO / "tools/dart_parse_sample.dart"), "-o", str(dart_binary)], cwd=REPO)
    rust_binary = output / "rust-parse"
    shutil.copy2(REPO / "target/release/xtask", rust_binary)
    report = {
        "schema_version": 1, "profile": "real-book-parse-v1",
        "measured_at": datetime.now(timezone.utc).isoformat(),
        "runner": args.runner, "platform": platform.platform(),
        "cpu": run(["sysctl", "-n", "machdep.cpu.brand_string"]),
        "samples_per_runtime": args.samples,
        "scope": "EPUB bytes already loaded; parse all content and count Unicode scalars. "
                 "Cold process per sample; filesystem cache uncontrolled. RSS is whole process. "
                 "Dart eagerly builds reader blocks/images; Rust retains linear spine ChapterIR. "
                 "Not equivalent rendering, first-page or warm-repagination evidence.",
        "pagelet": revision(REPO), "flow_read": revision(flow),
        "dart_version": run([args.dart, "--version"]),
        "rust_version": run(["rustc", "--version"]),
        "binary_sha256": {"dart": sha(dart_binary), "rust": sha(rust_binary)},
        "dart_source_sha256": {str(p.relative_to(flow)): sha(p)
                               for p in sorted((flow / "packages/epub_reader_core/lib").rglob("*.dart"))},
        "manifest_sha256": sha(args.manifest), "fixtures": [],
        "flow_lock_sha256": sha(flow / "pubspec.lock"),
    }
    failures = []
    for index, book in enumerate(books):
        row = {"id": book["id"], "sha256": book["sha256"], "status": "running",
               "samples": {"dart": [], "rust": []}}
        report["fixtures"].append(row)
        try:
            path = root / book["path"]
            for sample_index in range(args.samples):
                # Alternate order to reduce systematic thermal/order bias.
                order = ("dart", "rust") if sample_index % 2 == 0 else ("rust", "dart")
                for runtime in order:
                    command = ([dart_binary, path] if runtime == "dart" else
                               [rust_binary, "bench", "parse-sample", path])
                    row["samples"][runtime].append(measure(
                        command, output / f"{index}-{runtime}-{sample_index}"))
            if sha(path) != book["sha256"]:
                raise RuntimeError("fixture changed during measurement")
            row["summary"] = {}
            for runtime, samples in row["samples"].items():
                if len({(s['chapters'], s['visible_chars']) for s in samples}) != 1:
                    raise RuntimeError(f"non-deterministic parse output: {runtime}")
                row["summary"][runtime] = {
                    metric: {"p50": percentile([s[metric] for s in samples], .5),
                             "p95": percentile([s[metric] for s in samples], .95)}
                    for metric in ("parse_ns", "peak_rss_bytes")}
            row["status"] = "observed"
        except (RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
            row["status"] = "failed"
            row["error"] = str(error)
            failures.append(book["id"])
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{book['id']}: {row['status']}", flush=True)
    lines = ["# Real-book Dart/Rust parse observations", "", report["scope"], "",
             f"Runner: `{args.runner}`; {args.samples} fresh processes per runtime/fixture.", "",
             "| Fixture | Runtime | Parse p50 / p95 (ms) | Peak RSS p50 / p95 (MiB) | Chapters / chars |",
             "|---|---|---:|---:|---:|"]
    for row in report["fixtures"]:
        if row["status"] == "failed":
            lines.append(f"| {row['id']} | FAILED | | | |")
            continue
        for runtime, summary in row["summary"].items():
            timing = summary["parse_ns"]
            rss = summary["peak_rss_bytes"]
            sample = row["samples"][runtime][0]
            lines.append(f"| {row['id']} | {runtime} | {timing['p50']/1e6:.2f} / {timing['p95']/1e6:.2f} "
                         f"| {rss['p50']/2**20:.2f} / {rss['p95']/2**20:.2f} "
                         f"| {sample['chapters']} / {sample['visible_chars']} |")
    (output / "report.md").write_text("\n".join(lines) + "\n")
    print(output / "report.md")
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
