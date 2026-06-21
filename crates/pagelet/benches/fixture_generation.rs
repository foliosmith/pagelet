use std::{
    hint::black_box,
    time::{Duration, Instant},
};

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

    let warmup = benchmark_fixture_bytes(kind);
    black_box(&warmup);

    let started = Instant::now();
    let mut total_bytes = 0_usize;
    for _ in 0..iterations {
        let fixture = benchmark_fixture_bytes(kind);
        total_bytes = total_bytes.wrapping_add(fixture.len());
        black_box(&fixture);
    }
    let elapsed = started.elapsed();

    BenchRow {
        fixture: kind.id(),
        iterations,
        total_bytes,
        elapsed,
    }
}

#[derive(Debug, Clone, Copy)]
enum BenchmarkFixtureKind {
    TinyText,
    SmallNovel,
    LargeNovel,
    ImageHeavy,
    FootnoteHeavy,
    CssHeavy,
    CjkRtl,
    Pathological,
}

impl BenchmarkFixtureKind {
    const ALL: [Self; 8] = [
        Self::TinyText,
        Self::SmallNovel,
        Self::LargeNovel,
        Self::ImageHeavy,
        Self::FootnoteHeavy,
        Self::CssHeavy,
        Self::CjkRtl,
        Self::Pathological,
    ];

    const fn id(self) -> &'static str {
        match self {
            Self::TinyText => "tiny-text",
            Self::SmallNovel => "small-novel",
            Self::LargeNovel => "large-novel",
            Self::ImageHeavy => "image-heavy",
            Self::FootnoteHeavy => "footnote-heavy",
            Self::CssHeavy => "css-heavy",
            Self::CjkRtl => "cjk-rtl",
            Self::Pathological => "pathological",
        }
    }
}

fn benchmark_fixture_bytes(kind: BenchmarkFixtureKind) -> Vec<u8> {
    let mut bytes = Vec::from("PAGELET-BENCH\n");
    bytes.extend_from_slice(kind.id().as_bytes());
    bytes.push(b'\n');

    let (chapters, paragraphs, payload_size) = match kind {
        BenchmarkFixtureKind::TinyText => (1, 1, 128),
        BenchmarkFixtureKind::SmallNovel => (5, 8, 512),
        BenchmarkFixtureKind::LargeNovel => (24, 12, 2_048),
        BenchmarkFixtureKind::ImageHeavy => (2, 2, 128 * 1024),
        BenchmarkFixtureKind::FootnoteHeavy => (8, 12, 1_024),
        BenchmarkFixtureKind::CssHeavy => (4, 8, 4_096),
        BenchmarkFixtureKind::CjkRtl => (3, 8, 1_024),
        BenchmarkFixtureKind::Pathological => (10, 24, 512 * 1024),
    };

    for chapter in 0..chapters {
        bytes.extend_from_slice(format!("chapter={chapter}\n").as_bytes());
        for paragraph in 0..paragraphs {
            bytes
                .extend_from_slice(format!("p={paragraph}: generated benchmark text\n").as_bytes());
        }
    }
    bytes.resize(bytes.len() + payload_size, b'x');
    bytes
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
