//! Measures what one CAPTCHA costs: wall time, allocation, and encoded size.
//!
//! Runs the production path — `CaptchaService::generate`, which renders and
//! JPEG-encodes exactly as the HTTP handler does — so the numbers are what a
//! request actually pays rather than what the renderer alone costs.
//!
//! ```text
//! cargo run --release --example render_bench
//! ```
//!
//! Memory is measured with a counting allocator rather than process RSS,
//! because RSS reports whatever the allocator has chosen to keep from previous
//! work and would attribute none of it to a particular image. Two figures are
//! reported: the peak bytes outstanding above the pre-render baseline, which is
//! the high-water mark one render needs live at once, and the total bytes
//! requested, which is churn and shows how hard the allocator is worked.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use captchapi::services::CaptchaService;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            TOTAL.fetch_add(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct Sample {
    nanos: u128,
    peak: usize,
    churn: usize,
    bytes: usize,
}

fn measure(service: &CaptchaService, difficulty: i64, w: i64, h: i64, len: i64) -> Sample {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let churn_before = TOTAL.load(Ordering::Relaxed);

    let start = Instant::now();
    let (_text, bytes) = service
        .generate(len, difficulty, w, h, false, 40)
        .expect("render succeeds");
    let elapsed = start.elapsed();

    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let churn = TOTAL.load(Ordering::Relaxed) - churn_before;

    Sample {
        nanos: elapsed.as_nanos(),
        peak,
        churn,
        bytes: bytes.len(),
    }
}

fn summarise(label: &str, mut samples: Vec<Sample>) {
    samples.sort_by_key(|s| s.nanos);
    let n = samples.len();
    let mean_kib = |total: usize| total as f64 / n as f64 / 1024.0;

    let median_ms = samples[n / 2].nanos as f64 / 1e6;
    let p95_ms = samples[(n * 95 / 100).min(n - 1)].nanos as f64 / 1e6;
    let peak_kb = mean_kib(samples.iter().map(|s| s.peak).sum());
    let churn_kb = mean_kib(samples.iter().map(|s| s.churn).sum());
    let size_kb = mean_kib(samples.iter().map(|s| s.bytes).sum());

    println!(
        "{label:<22} {median_ms:>8.2} {p95_ms:>8.2} {peak_kb:>10.1} {churn_kb:>10.1} {size_kb:>9.2}"
    );
}

/// Sustained renders per second with `threads` workers going flat out.
///
/// Measured rather than derived from the single-shot median, because the
/// per-image figure hides allocator contention and memory bandwidth effects
/// that only appear once every core is rendering at once.
fn throughput(service: &CaptchaService, difficulty: i64, length: i64, threads: usize) -> f64 {
    let completed = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let start = Instant::now();

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let _ = service
                        .generate(length, difficulty, 220, 120, false, 40)
                        .expect("render succeeds");
                    completed.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        std::thread::sleep(Duration::from_secs(4));
        stop.store(true, Ordering::Relaxed);
    });

    completed.load(Ordering::Relaxed) as f64 / start.elapsed().as_secs_f64()
}

fn main() {
    let service = CaptchaService::new();
    let runs = 200;

    // Warm the lazily-parsed font so its one-time cost is not charged to the
    // first difficulty measured.
    let _ = service.generate(5, 5, 220, 120, false, 40);

    println!(
        "{:<22} {:>8} {:>8} {:>10} {:>10} {:>9}",
        "", "med ms", "p95 ms", "peak KiB", "churn KiB", "jpeg KiB"
    );
    println!("{}", "-".repeat(72));

    println!("difficulty sweep, 220x120, length 5, quality 40");
    for difficulty in 1..=10 {
        let samples = (0..runs)
            .map(|_| measure(&service, difficulty, 220, 120, 5))
            .collect();
        summarise(&format!("  difficulty {difficulty}"), samples);
    }

    println!();
    println!("canvas size, difficulty 10, length 5, quality 40");
    for (w, h) in [(150, 60), (220, 120), (400, 150), (800, 300)] {
        let samples = (0..runs).map(|_| measure(&service, 10, w, h, 5)).collect();
        summarise(&format!("  {w}x{h}"), samples);
    }

    println!();
    println!("solution length, difficulty 10, 220x120, quality 40");
    for len in [3, 5, 8, 12] {
        let samples = (0..runs)
            .map(|_| measure(&service, 10, 220, 120, len))
            .collect();
        summarise(&format!("  length {len}"), samples);
    }

    println!();
    println!("sustained throughput, 220x120, quality 40, every deformation active");
    println!(
        "{:<22} {:>10} {:>12} {:>12}",
        "", "renders/s", "per core", "ms/render"
    );
    for difficulty in [5i64, 8, 10] {
        for threads in [1usize, 2, 4] {
            let rate = throughput(&service, difficulty, 5, threads);
            println!(
                "  difficulty {difficulty}, {threads} thread{:<3} {rate:>10.1} {:>12.1} {:>12.2}",
                if threads == 1 { "" } else { "s" },
                rate / threads as f64,
                1000.0 / (rate / threads as f64)
            );
        }
    }
}
