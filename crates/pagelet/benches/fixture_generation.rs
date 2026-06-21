use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use pagelet::testkit::{benchmark_fixture, BenchmarkFixtureKind};

fn main() {
    println!("fixture,iterations,total_bytes,elapsed_ns,ns_per_iter");
    for kind in BenchmarkFixtureKind::ALL {
        let row = bench_fixture_generation(kind);
        println!(
            "{},{},{},{},{}",
            row.fixture,
            row.iterations,
            row.total_bytes,
            row.elapsed.as_nanos(),
            row.ns_per_iter()
        );
    }
}

fn bench_fixture_generation(kind: BenchmarkFixtureKind) -> BenchRow {
    let iterations = iterations_for(kind);

    let warmup = benchmark_fixture(kind);
    black_box(warmup.bytes());

    let started = Instant::now();
    let mut total_bytes = 0_usize;
    for _ in 0..iterations {
        let fixture = benchmark_fixture(kind);
        total_bytes = total_bytes.wrapping_add(fixture.bytes().len());
        black_box(fixture.bytes());
    }
    let elapsed = started.elapsed();

    BenchRow {
        fixture: kind.id(),
        iterations,
        total_bytes,
        elapsed,
    }
}

fn iterations_for(kind: BenchmarkFixtureKind) -> u32 {
    match kind {
        BenchmarkFixtureKind::TinyText => 1_000,
        BenchmarkFixtureKind::SmallNovel => 300,
        BenchmarkFixtureKind::LargeNovel => 80,
        BenchmarkFixtureKind::ImageHeavy => 120,
        BenchmarkFixtureKind::FootnoteHeavy => 500,
        BenchmarkFixtureKind::CssHeavy => 500,
        BenchmarkFixtureKind::CjkRtl => 500,
        BenchmarkFixtureKind::Pathological => 60,
    }
}

struct BenchRow {
    fixture: &'static str,
    iterations: u32,
    total_bytes: usize,
    elapsed: Duration,
}

impl BenchRow {
    fn ns_per_iter(&self) -> u128 {
        self.elapsed.as_nanos() / u128::from(self.iterations)
    }
}
