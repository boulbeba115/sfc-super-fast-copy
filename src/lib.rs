//! Super Fast Copy engine: parallel, Windows-native file copying tuned for folders
//! with many small files.
//!
//! How it gets its speed:
//! * Directories are enumerated with `FindFirstFileEx` (large fetch, no 8.3
//!   names), which also returns size/attributes/times, so files are never
//!   stat'ed individually.
//! * Enumeration and copying run together on a work-stealing pool, so
//!   copying starts immediately and every disk stays busy.
//! * Small files take a minimal-syscall path with a reused per-thread
//!   buffer; metadata is set on the open handle and the (slow, antivirus-
//!   scanned) close happens on separate closer threads.
//! * Large and special files use `CopyFileEx`, unbuffered for huge files.
//! * Re-runs with "skip same" compare against one listing of each
//!   destination folder instead of probing every file.

pub mod fmt;
pub mod shell;
pub mod tune;
pub mod win;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use rayon::prelude::*;

use tune::Media;
use win::{Entry, Handle, Wide};

const BUF_SIZE: usize = 1 << 20;
const BATCH_FILES: usize = 16;
const BATCH_BYTES: u64 = 8 << 20;
const NO_BUFFERING_MIN: u64 = 256 << 20;
const MAX_ERRORS_KEPT: usize = 10_000;
/// FAT/exFAT store write times with 2 s resolution.
const MTIME_TOLERANCE: i64 = 20_000_000;

/// What to do when a file already exists in the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    Overwrite,
    /// Skip files whose size and modification time match.
    SkipSame,
    /// Skip any file that already exists.
    SkipExisting,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Worker threads; 0 picks a value from the drive types.
    pub threads: usize,
    pub existing: Existing,
    /// Copy the contents of source folders straight into the destination
    /// instead of creating a sub-folder named after the source.
    pub contents_only: bool,
    /// Use CopyFileEx for every file (keeps alternate data streams).
    pub native: bool,
    /// Files at least this large use CopyFileEx.
    pub large_threshold: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            threads: 0,
            existing: Existing::Overwrite,
            contents_only: false,
            native: false,
            large_threshold: 8 << 20,
        }
    }
}

#[derive(Default)]
struct Stats {
    files_found: AtomicU64,
    dirs_found: AtomicU64,
    bytes_found: AtomicU64,
    files_copied: AtomicU64,
    files_skipped: AtomicU64,
    files_failed: AtomicU64,
    bytes_copied: AtomicU64,
    bytes_done: AtomicU64,
    links_skipped: AtomicU64,
}

/// Point-in-time view of a running job.
#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub files_found: u64,
    pub dirs_found: u64,
    pub bytes_found: u64,
    pub files_copied: u64,
    pub files_skipped: u64,
    pub files_failed: u64,
    pub bytes_copied: u64,
    /// Bytes copied, skipped or given up on; use for progress bars.
    pub bytes_done: u64,
    pub links_skipped: u64,
    pub errors: u64,
    /// True once every folder has been listed, so the totals are final.
    pub scan_done: bool,
}

impl Snapshot {
    pub fn files_done(&self) -> u64 {
        self.files_copied + self.files_skipped + self.files_failed
    }

    /// Blend of file and byte progress: per-file overhead dominates for
    /// small files, bytes dominate for big ones.
    pub fn fraction(&self) -> f32 {
        let f = if self.files_found > 0 { self.files_done() as f64 / self.files_found as f64 } else { 1.0 };
        let b = if self.bytes_found > 0 { self.bytes_done as f64 / self.bytes_found as f64 } else { f };
        ((f + b) / 2.0).clamp(0.0, 1.0) as f32
    }
}

/// Shared state of one copy job: progress counters, errors and cancellation.
pub struct Job {
    stats: Stats,
    cancel: AtomicI32,
    pending_dirs: AtomicUsize,
    errors_total: AtomicU64,
    errors: Mutex<Vec<String>>,
    plan: Mutex<Option<String>>,
}

impl Default for Job {
    fn default() -> Self {
        Self::new()
    }
}

impl Job {
    pub fn new() -> Self {
        Self {
            stats: Stats::default(),
            cancel: AtomicI32::new(0),
            // Held until all roots are queued, so the scan can't look done early.
            pending_dirs: AtomicUsize::new(1),
            errors_total: AtomicU64::new(0),
            errors: Mutex::new(Vec::new()),
            plan: Mutex::new(None),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(1, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed) != 0
    }

    /// Thread count and drive types chosen for this job, once known.
    pub fn plan(&self) -> Option<String> {
        self.plan.lock().unwrap().clone()
    }

    /// The first `n` error messages.
    pub fn errors(&self, n: usize) -> Vec<String> {
        let e = self.errors.lock().unwrap();
        e[..e.len().min(n)].to_vec()
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = &self.stats;
        Snapshot {
            files_found: s.files_found.load(Relaxed),
            dirs_found: s.dirs_found.load(Relaxed),
            bytes_found: s.bytes_found.load(Relaxed),
            files_copied: s.files_copied.load(Relaxed),
            files_skipped: s.files_skipped.load(Relaxed),
            files_failed: s.files_failed.load(Relaxed),
            bytes_copied: s.bytes_copied.load(Relaxed),
            bytes_done: s.bytes_done.load(Relaxed),
            links_skipped: s.links_skipped.load(Relaxed),
            errors: self.errors_total.load(Relaxed),
            scan_done: self.pending_dirs.load(Relaxed) == 0,
        }
    }

    fn error(&self, msg: String) {
        self.errors_total.fetch_add(1, Relaxed);
        let mut e = self.errors.lock().unwrap();
        if e.len() < MAX_ERRORS_KEPT {
            e.push(msg);
        }
    }
}

struct DirMeta {
    path: Wide,
    meta: Entry,
}

struct Ctx<'a> {
    job: &'a Job,
    opts: &'a Options,
    closer: Sender<Handle>,
    dir_meta: Mutex<Vec<DirMeta>>,
    /// On spinning disks, large files are copied one at a time so their
    /// sequential streams don't fight over the disk head.
    large_gate: Option<Mutex<()>>,
}

struct Root {
    src: Wide,
    dst: Wide,
    meta: Entry,
    set_meta: bool,
}

/// Copies every path in `sources` into the folder `dest` (created if
/// needed). Per-file problems are recorded on `job` and don't stop the
/// copy; the returned error is only for problems that prevent starting.
pub fn run(job: &Job, sources: &[PathBuf], dest: &Path, opts: &Options) -> Result<(), String> {
    let result = run_inner(job, sources, dest, opts);
    // Make sure observers see the scan as finished even on early errors.
    job.pending_dirs.store(0, Relaxed);
    result
}

fn run_inner(job: &Job, sources: &[PathBuf], dest: &Path, opts: &Options) -> Result<(), String> {
    if sources.is_empty() {
        return Err("nothing to copy".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let dest_w = win::to_verbatim(dest).map_err(|e| format!("{}: {e}", dest.display()))?;

    let mut roots = Vec::new();
    for s in sources {
        let src = win::to_verbatim(s).map_err(|e| format!("{}: {e}", s.display()))?;
        let meta = win::metadata(&src).map_err(|e| format!("{}: {e}", s.display()))?;
        let name = win::file_name(&src).to_vec();
        let into_dest = name.is_empty() || (meta.is_dir() && opts.contents_only);
        let dst = if into_dest { dest_w.clone() } else { win::join(&dest_w, &name) };
        if meta.is_dir() && win::is_within(&dst, &src) {
            return Err(format!(
                "cannot copy {} into itself ({})",
                win::display(&src),
                win::display(&dst)
            ));
        }
        if !meta.is_dir() && win::is_within(&dst, &src) {
            return Err(format!("{} is both source and destination", win::display(&src)));
        }
        roots.push(Root { src, dst, meta, set_meta: !into_dest });
    }

    let src_media = tune::media(&roots[0].src);
    let dst_media = tune::media(&dest_w);
    let threads = if opts.threads > 0 { opts.threads } else { tune::auto_threads(src_media, dst_media) };
    *job.plan.lock().unwrap() = Some(format!(
        "{threads} threads{} ({} → {})",
        if opts.threads > 0 { "" } else { ", auto" },
        src_media.name(),
        dst_media.name()
    ));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("sfc-{i}"))
        .build()
        .map_err(|e| format!("cannot start worker threads: {e}"))?;
    let closer = win::Closer::new((threads / 2).clamp(2, 16));
    let hdd = src_media == Media::Hdd || dst_media == Media::Hdd;
    let ctx = Ctx {
        job,
        opts,
        closer: closer.sender(),
        dir_meta: Mutex::new(Vec::new()),
        large_gate: hdd.then(|| Mutex::new(())),
    };

    pool.scope(|s| {
        for root in roots {
            if root.meta.is_dir() {
                job.pending_dirs.fetch_add(1, Relaxed);
                let ctx = &ctx;
                s.spawn(move |s| process_dir(s, ctx, root.src, root.dst, root.set_meta.then_some(root.meta)));
            } else {
                let st = &job.stats;
                st.files_found.fetch_add(1, Relaxed);
                st.bytes_found.fetch_add(root.meta.size, Relaxed);
                let existing = win::metadata(&root.dst).ok().filter(|d| !d.is_dir());
                if existing.is_some_and(|d| should_skip(opts.existing, &root.meta, &d)) {
                    st.files_skipped.fetch_add(1, Relaxed);
                    st.bytes_done.fetch_add(root.meta.size, Relaxed);
                    continue;
                }
                let ctx = &ctx;
                s.spawn(move |_| {
                    copy_file(ctx, &win::zstr(&root.src), &win::zstr(&root.dst), &root.meta)
                });
            }
        }
        job.pending_dirs.fetch_sub(1, Relaxed);
    });

    let Ctx { closer: sender, dir_meta, .. } = ctx;
    drop(sender);
    closer.finish();

    // Folder times last: creating files inside a folder changes its times.
    let dir_meta = dir_meta.into_inner().unwrap();
    pool.install(|| {
        dir_meta.par_iter().for_each(|d| {
            if let Err(e) = win::set_dir_meta(&d.path, &d.meta) {
                job.error(format!("{}: setting folder times: {e}", win::display(&d.path)));
            }
        })
    });
    Ok(())
}

fn should_skip(mode: Existing, src: &Entry, dst: &Entry) -> bool {
    match mode {
        Existing::Overwrite => false,
        Existing::SkipExisting => true,
        Existing::SkipSame => src.size == dst.size && (src.mtime - dst.mtime).abs() <= MTIME_TOLERANCE,
    }
}

fn name_key(name: &[u16]) -> Wide {
    name.iter()
        .map(|&c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c })
        .collect()
}

/// Decrements the pending-folder count however `process_dir` exits.
struct PendingGuard<'a>(&'a AtomicUsize);

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

fn process_dir<'s, 'a: 's>(
    s: &rayon::Scope<'s>,
    ctx: &'s Ctx<'a>,
    src: Wide,
    dst: Wide,
    meta: Option<Entry>,
) {
    let job = ctx.job;
    let _pending = PendingGuard(&job.pending_dirs);
    if job.is_cancelled() {
        return;
    }
    let st = &job.stats;
    st.dirs_found.fetch_add(1, Relaxed);

    let existed = match win::create_dir(&dst) {
        Ok(created) => !created,
        Err(e) => {
            job.error(format!("{}: {e}", win::display(&dst)));
            return;
        }
    };
    let entries = match win::list_dir(&src) {
        Ok(v) => v,
        Err(e) => {
            job.error(format!("{}: {e}", win::display(&src)));
            return;
        }
    };
    let dest_index: Option<HashMap<Wide, Entry>> = (existed && ctx.opts.existing != Existing::Overwrite)
        .then(|| win::list_dir(&dst).ok())
        .flatten()
        .map(|v| v.into_iter().filter(|e| !e.is_dir()).map(|e| (name_key(&e.name), e)).collect());

    let src_dir = Arc::new(src);
    let dst_dir = Arc::new(dst);
    let mut batch: Vec<Entry> = Vec::new();
    let mut batch_bytes = 0u64;
    let flush = |batch: Vec<Entry>| {
        let (sd, dd) = (src_dir.clone(), dst_dir.clone());
        s.spawn(move |_| {
            for e in &batch {
                copy_file(ctx, &win::join_z(&sd, &e.name), &win::join_z(&dd, &e.name), e);
            }
        });
    };

    for e in entries {
        if e.is_dir() {
            if e.is_reparse() {
                // Junctions/symlinked folders: not followed, to avoid loops
                // and copying things outside the source tree.
                st.links_skipped.fetch_add(1, Relaxed);
                continue;
            }
            job.pending_dirs.fetch_add(1, Relaxed);
            let (cs, cd) = (win::join(&src_dir, &e.name), win::join(&dst_dir, &e.name));
            s.spawn(move |s| process_dir(s, ctx, cs, cd, Some(e)));
            continue;
        }

        st.files_found.fetch_add(1, Relaxed);
        st.bytes_found.fetch_add(e.size, Relaxed);
        if let Some(d) = dest_index.as_ref().and_then(|idx| idx.get(&name_key(&e.name))) {
            if should_skip(ctx.opts.existing, &e, d) {
                st.files_skipped.fetch_add(1, Relaxed);
                st.bytes_done.fetch_add(e.size, Relaxed);
                continue;
            }
        }
        if e.size >= ctx.opts.large_threshold {
            flush(vec![e]);
            continue;
        }
        batch_bytes += e.size;
        batch.push(e);
        if batch.len() >= BATCH_FILES || batch_bytes >= BATCH_BYTES {
            flush(std::mem::take(&mut batch));
            batch_bytes = 0;
        }
    }
    if !batch.is_empty() {
        flush(batch);
    }
    if let Some(meta) = meta {
        ctx.dir_meta.lock().unwrap().push(DirMeta { path: (*dst_dir).clone(), meta });
    }
}

thread_local! {
    static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// `src` and `dst` are NUL-terminated.
fn copy_file(ctx: &Ctx, src: &[u16], dst: &[u16], e: &Entry) {
    let job = ctx.job;
    if job.is_cancelled() {
        return;
    }
    let st = &job.stats;
    let mut written = 0u64;
    let result = {
        let mut progress = |n: u64| {
            written += n;
            st.bytes_copied.fetch_add(n, Relaxed);
            st.bytes_done.fetch_add(n, Relaxed);
        };
        let large = e.size >= ctx.opts.large_threshold;
        if ctx.opts.native || large || e.attrs & win::SPECIAL_ATTRS != 0 {
            let _gate = large.then(|| ctx.large_gate.as_ref().map(|g| g.lock().unwrap())).flatten();
            win::copy_native(src, dst, e.size >= NO_BUFFERING_MIN, &job.cancel, &mut progress)
        } else {
            BUF.with(|b| {
                let mut b = b.borrow_mut();
                if b.len() < BUF_SIZE {
                    b.resize(BUF_SIZE, 0);
                }
                win::copy_fast(src, dst, e, &mut b, &ctx.closer, &mut progress)
            })
        }
    };
    match result {
        Ok(()) => {
            st.files_copied.fetch_add(1, Relaxed);
        }
        Err(err) => {
            st.files_failed.fetch_add(1, Relaxed);
            st.bytes_done.fetch_add(e.size.saturating_sub(written), Relaxed);
            if !job.is_cancelled() {
                job.error(format!("{}: {err}", win::display(src)));
            }
        }
    }
}
