//! Screenshot memory leak finder — concurrent mode simulates real server load.
//!
//! Usage:
//!   cargo build -p ffmpeg-tool --bin screenshot-smb-bench
//!   LD_LIBRARY_PATH=bin/ffmpeg/linux-x86_64/lib ./target/debug/screenshot-smb-bench \
//!     --host 10.0.0.10 --share media --username william --password '...' \
//!     --paths '/tv/A.mkv,/tv/B.mkv,/tv/C.mkv' --rounds 5 --concurrency 8

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_in_result,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use tokimo_package_ffmpeg::{DirectInput, ImageFormat, VideoScreenshotOptions, capture_video_screenshot_direct};
use tokimo_vfs::Vfs;
use tokimo_vfs::drivers::smb::factory as smb_factory;
use tokimo_vfs_op::{StorageManager, StorageMount};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Parser, Debug)]
#[command(about = "Screenshot memory leak finder")]
struct Args {
    #[arg(long)]
    host: String,
    #[arg(long, default_value = "")]
    share: String,
    #[arg(long, default_value = "")]
    username: String,
    #[arg(long, default_value = "")]
    password: String,
    #[arg(long, default_value = "")]
    domain: String,
    #[arg(long, default_value = "")]
    root: String,
    /// Comma-separated remote paths (e.g. "/tv/a.mkv,/tv/b.mkv")
    #[arg(long)]
    paths: String,
    /// How many rounds to repeat the full concurrent batch
    #[arg(long, default_value = "5")]
    rounds: u32,
    /// Max concurrent screenshots per round (0 = all paths at once)
    #[arg(long, default_value = "8")]
    concurrency: usize,
    /// Seek offset in seconds
    #[arg(long, default_value = "60.0")]
    offset: f64,
    /// Target width
    #[arg(long, default_value = "1280")]
    width: u32,
    /// Call malloc_trim(0) after each round to release glibc arena memory
    #[arg(long)]
    trim: bool,
}

// ── /proc/self/status ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct ProcMem {
    vm_rss_kb: u64,
    vm_hwm_kb: u64,
}

impl ProcMem {
    fn read() -> Self {
        let Ok(s) = std::fs::read_to_string("/proc/self/status") else {
            return Self::default();
        };
        let mut m = Self::default();
        for line in s.lines() {
            let mut p = line.splitn(2, ':');
            let (Some(k), Some(v)) = (p.next(), p.next()) else {
                continue;
            };
            let kb: u64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
            match k.trim() {
                "VmHWM" => m.vm_hwm_kb = kb,
                "VmRSS" => m.vm_rss_kb = kb,
                _ => {}
            }
        }
        m
    }
    fn rss_mb(&self) -> f64 {
        self.vm_rss_kb as f64 / 1024.0
    }
    fn hwm_mb(&self) -> f64 {
        self.vm_hwm_kb as f64 / 1024.0
    }
}

// ── jemalloc stats ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct Jem {
    allocated: u64,
    resident: u64,
    retained: u64,
}

impl Jem {
    fn read() -> Self {
        use tikv_jemalloc_ctl::{epoch, stats};
        let _ = epoch::mib().ok().and_then(|e| e.advance().ok());
        Self {
            allocated: stats::allocated::mib().ok().and_then(|m| m.read().ok()).unwrap_or(0) as u64,
            resident: stats::resident::mib().ok().and_then(|m| m.read().ok()).unwrap_or(0) as u64,
            retained: stats::retained::mib().ok().and_then(|m| m.read().ok()).unwrap_or(0) as u64,
        }
    }
    fn amb(&self) -> f64 {
        self.allocated as f64 / 1024.0 / 1024.0
    }
    fn rmb(&self) -> f64 {
        self.resident as f64 / 1024.0 / 1024.0
    }
    fn tmb(&self) -> f64 {
        self.retained as f64 / 1024.0 / 1024.0
    }
}

struct Snap {
    proc: ProcMem,
    jem: Jem,
}
impl Snap {
    fn take() -> Self {
        Self {
            proc: ProcMem::read(),
            jem: Jem::read(),
        }
    }
    fn print(&self, label: &str, base: Option<&Snap>) {
        println!(
            "{label}: RSS={:.1}MB HWM={:.1}MB | jAlloc={:.1}MB jRes={:.1}MB jRetain={:.1}MB",
            self.proc.rss_mb(),
            self.proc.hwm_mb(),
            self.jem.amb(),
            self.jem.rmb(),
            self.jem.tmb()
        );
        if let Some(b) = base {
            println!(
                "  Δ: RSS={:+.1}MB jAlloc={:+.1}MB jRes={:+.1}MB",
                (self.proc.vm_rss_kb as i64 - b.proc.vm_rss_kb as i64) as f64 / 1024.0,
                (self.jem.allocated as i64 - b.jem.allocated as i64) as f64 / 1024.0 / 1024.0,
                (self.jem.resident as i64 - b.jem.resident as i64) as f64 / 1024.0 / 1024.0
            );
        }
    }
}

// ── main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();
    let paths: Vec<String> = args.paths.split(',').map(|s| s.trim().to_string()).collect();

    println!("Connecting SMB {}:{} …", args.host, args.share);
    let params = serde_json::json!({
        "host": args.host, "share": args.share,
        "username": args.username, "password": args.password,
        "domain": args.domain, "root": args.root,
    });
    let driver = smb_factory(&params)?;
    let _ = driver.read_bytes(Path::new(&paths[0]), 0, Some(64 * 1024)).await;
    println!("Connected.");

    let mut manager = StorageManager::new();
    manager.mount(StorageMount::new("/", Arc::from(driver))).await;
    let vfs = Arc::new(Vfs::new(manager));

    let mut file_infos: Vec<(String, u64)> = Vec::new();
    for path in &paths {
        let size = vfs.stat(Path::new(path)).await.map(|s| s.size).unwrap_or(0);
        println!("  {} ({:.0} MB)", path, size as f64 / 1024.0 / 1024.0);
        file_infos.push((path.clone(), size));
    }

    let opts = VideoScreenshotOptions {
        width: Some(args.width),
        format: ImageFormat::Jpeg,
        quality: 2,
        prefer_hardware: true,
        offset_secs: args.offset,
        ..Default::default()
    };

    let baseline = Snap::take();
    baseline.print("Baseline", None);

    let conc = if args.concurrency == 0 {
        file_infos.len()
    } else {
        args.concurrency
    };
    println!("\n── {} rounds × {} concurrent ──", args.rounds, conc);
    println!(
        "{:>5}  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}  {:>7}",
        "round", "RSS(MB)", "HWM(MB)", "jAlloc", "jRes", "jRetain", "ms"
    );
    println!("{}", "-".repeat(65));

    let sem = Arc::new(tokio::sync::Semaphore::new(conc));
    let mut prev_rss = baseline.proc.vm_rss_kb;

    for round in 1..=args.rounds {
        let t = Instant::now();
        let mut handles = Vec::new();

        for (path, size) in &file_infos {
            let sem = sem.clone();
            let vfs = vfs.clone();
            let path = path.clone();
            let size = *size;
            let opts = opts.clone();
            let hint = Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string);
            let round_n = round;

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.unwrap();
                let ra = vfs.to_read_at(Path::new(&path)).await;
                let direct = DirectInput::from_read_at(ra, size, hint, None);
                let opts2 = opts.clone();
                match tokio::task::spawn_blocking(move || capture_video_screenshot_direct(direct, &opts2)).await {
                    Ok(Ok(b)) => drop(b),
                    Ok(Err(e)) => eprintln!("  [r{round_n}] {path}: {e}"),
                    Err(e) => eprintln!("  [r{round_n}] {path}: panic {e}"),
                }
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        // Optional: force glibc to release free arena memory back to OS.
        // FFmpeg's av_malloc() uses glibc (not jemalloc), so without this
        // glibc arenas hold fragmented free memory indefinitely.
        if args.trim {
            unsafe {
                libc::malloc_trim(0);
            }
        }

        let elapsed = t.elapsed().as_millis() as u64;
        let snap = Snap::take();
        let drss = snap.proc.vm_rss_kb as i64 - prev_rss as i64;
        let flag = if drss > 30 * 1024 {
            " ⬆"
        } else if drss < -10 * 1024 {
            " ⬇"
        } else {
            ""
        };
        println!(
            "{:>5}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>8.1}  {:>7}{}",
            round,
            snap.proc.rss_mb(),
            snap.proc.hwm_mb(),
            snap.jem.amb(),
            snap.jem.rmb(),
            snap.jem.tmb(),
            elapsed,
            flag
        );
        prev_rss = snap.proc.vm_rss_kb;
    }

    let fin = Snap::take();
    println!();
    fin.print("Final", Some(&baseline));

    let drss = fin.proc.vm_rss_kb as i64 - baseline.proc.vm_rss_kb as i64;
    let dalloc = fin.jem.allocated as i64 - baseline.jem.allocated as i64;

    println!();
    if dalloc > 10 * 1024 * 1024 {
        println!(
            "🔴 HEAP LEAK: jAlloc grew {:+.1}MB over {} rounds!",
            dalloc as f64 / 1024.0 / 1024.0,
            args.rounds
        );
    } else if drss > 200 * 1024 {
        println!(
            "🟡 RSS grew {:+.1}MB but heap stable — FFmpeg mmap / fragmentation",
            drss as f64 / 1024.0
        );
    } else {
        println!("✅ No significant growth after {} rounds", args.rounds);
    }

    Ok(())
}

// ── malloc_trim helper ────────────────────────────────────────────────────────
// Forces glibc to release free arena memory back to OS.
// FFmpeg uses av_malloc() → glibc (not jemalloc), so this is needed.
fn trim_glibc_heap() {
    unsafe {
        libc::malloc_trim(0);
    }
}
