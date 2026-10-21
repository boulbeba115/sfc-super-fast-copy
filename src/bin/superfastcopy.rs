use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use superfastcopy::{fmt, Existing, Job, Options};
use windows_sys::Win32::Foundation::BOOL;
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

/// Super Fast Copy (SFC): very fast parallel file copy for Windows.
///
/// Copies each SOURCE (file or folder) into DEST, e.g.
/// `superfastcopy C:\Photos D:\Backup` creates D:\Backup\Photos.
#[derive(Parser)]
#[command(name = "superfastcopy", version)]
struct Cli {
    /// One or more sources followed by the destination folder
    #[arg(required = true, num_args = 2.., value_name = "SOURCE... DEST")]
    paths: Vec<PathBuf>,

    /// Worker threads (0 = choose from drive type: SSD, HDD or network)
    #[arg(short, long, default_value_t = 0)]
    threads: usize,

    /// What to do with files that already exist in the destination
    #[arg(short, long, value_enum, default_value_t = ExistingArg::Overwrite)]
    existing: ExistingArg,

    /// Copy the contents of source folders into DEST instead of a sub-folder
    #[arg(short, long)]
    contents: bool,

    /// Use Windows' CopyFileEx for every file (keeps alternate data streams
    /// such as "downloaded from the internet" marks; slower for small files)
    #[arg(long)]
    native: bool,

    /// Files at least this many MiB are copied with CopyFileEx
    #[arg(long, default_value_t = 8, value_name = "MiB")]
    large: u64,

    /// Only print the final summary
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ExistingArg {
    /// Replace existing files
    Overwrite,
    /// Skip files with the same size and modification time (fast re-sync)
    SkipSame,
    /// Skip every file that already exists
    Skip,
}

static JOB: OnceLock<Arc<Job>> = OnceLock::new();

unsafe extern "system" fn on_ctrl(_kind: u32) -> BOOL {
    match JOB.get() {
        Some(job) => {
            job.cancel();
            1
        }
        None => 0,
    }
}

fn main() {
    let cli = Cli::parse();
    let mut paths = cli.paths;
    let dest = paths.pop().expect("clap enforces two paths");
    let opts = Options {
        threads: cli.threads,
        existing: match cli.existing {
            ExistingArg::Overwrite => Existing::Overwrite,
            ExistingArg::SkipSame => Existing::SkipSame,
            ExistingArg::Skip => Existing::SkipExisting,
        },
        contents_only: cli.contents,
        native: cli.native,
        large_threshold: cli.large.max(1) << 20,
    };

    let job = JOB.get_or_init(|| Arc::new(Job::new())).clone();
    unsafe {
        SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }

    let start = Instant::now();
    let worker = {
        let job = job.clone();
        std::thread::spawn(move || superfastcopy::run(&job, &paths, &dest, &opts))
    };

    let mut err = std::io::stderr();
    let mut printed_plan = false;
    while !worker.is_finished() {
        std::thread::sleep(Duration::from_millis(200));
        if cli.quiet {
            continue;
        }
        if !printed_plan {
            if let Some(plan) = job.plan() {
                let _ = writeln!(err, "superfastcopy: {plan}");
                printed_plan = true;
            }
        }
        let s = job.snapshot();
        let t = start.elapsed();
        let pct = if s.scan_done { format!("{:5.1}%", s.fraction() * 100.0) } else { "scan…".into() };
        let _ = write!(
            err,
            "\r{pct}  {} / {} files  {}  {}/s  {} files/s  {} errors   ",
            fmt::count(s.files_done()),
            fmt::count(s.files_found),
            fmt::bytes(s.bytes_copied),
            fmt::bytes(fmt::rate(s.bytes_copied, t) as u64),
            fmt::count(fmt::rate(s.files_copied, t) as u64),
            s.errors,
        );
        let _ = err.flush();
    }
    let result = worker.join().unwrap_or_else(|_| Err("internal error".into()));
    let elapsed = start.elapsed();
    if !cli.quiet {
        let _ = writeln!(err);
    }

    if let Err(e) = result {
        eprintln!("superfastcopy: {e}");
        std::process::exit(2);
    }

    let s = job.snapshot();
    let errors = job.errors(20);
    for e in &errors {
        eprintln!("error: {e}");
    }
    if s.errors > errors.len() as u64 {
        eprintln!("... and {} more errors", fmt::count(s.errors - errors.len() as u64));
    }
    if s.links_skipped > 0 {
        eprintln!("note: {} folder links/junctions were not followed", s.links_skipped);
    }
    println!(
        "{} in {}: {} copied, {} skipped, {} failed, {} ({}/s, {} files/s)",
        if job.is_cancelled() { "Cancelled" } else { "Done" },
        fmt::duration(elapsed),
        fmt::count(s.files_copied),
        fmt::count(s.files_skipped),
        fmt::count(s.files_failed),
        fmt::bytes(s.bytes_copied),
        fmt::bytes(fmt::rate(s.bytes_copied, elapsed) as u64),
        fmt::count(fmt::rate(s.files_copied, elapsed) as u64),
    );
    std::process::exit(if job.is_cancelled() {
        130
    } else if s.errors > 0 {
        1
    } else {
        0
    });
}
