use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tokio::fs::File as TokioFile;
use tracing::{info, warn};

static FSYNC_DISABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_fsync_disabled(disabled: bool) {
    FSYNC_DISABLED.store(disabled, Ordering::Relaxed);
}

pub(crate) fn fsync_disabled() -> bool {
    FSYNC_DISABLED.load(Ordering::Relaxed)
}

pub(crate) fn sync_all(file: &File, context: &str, path: Option<&Path>) -> io::Result<()> {
    run_sync("fsync", context, path, || file.sync_all())
}

pub(crate) fn sync_data(file: &File, context: &str, path: Option<&Path>) -> io::Result<()> {
    run_sync("fdatasync", context, path, || file.sync_data())
}

pub(crate) async fn sync_all_async(
    file: &TokioFile,
    context: &str,
    path: Option<&Path>,
) -> io::Result<()> {
    if fsync_disabled() {
        log_sync_skipped("fsync", context, path);
        return Ok(());
    }
    log_sync_start("fsync", context, path);
    let start = Instant::now();
    let result = file.sync_all().await;
    log_sync_result("fsync", context, path, start.elapsed(), &result);
    result
}

pub(crate) fn sync_parent_dir(path: &Path, context: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            let parent_file = File::open(parent)?;
            return sync_all(&parent_file, context, Some(parent));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

pub(crate) fn sync_path_data(path: &Path, context: &str) -> io::Result<()> {
    let file = File::options().read(true).write(true).open(path)?;
    sync_data(&file, context, Some(path))
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8], context: &str) -> io::Result<()> {
    let tmp_path = tmp_path(path);
    std::fs::write(&tmp_path, bytes)?;
    let tmp_file = File::open(&tmp_path)?;
    sync_all(&tmp_file, context, Some(&tmp_path))?;
    std::fs::rename(&tmp_path, path)?;
    sync_parent_dir(path, context)?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".tmp");
    PathBuf::from(os)
}

fn run_sync<F>(op: &'static str, context: &str, path: Option<&Path>, f: F) -> io::Result<()>
where
    F: FnOnce() -> io::Result<()>,
{
    if fsync_disabled() {
        log_sync_skipped(op, context, path);
        return Ok(());
    }
    log_sync_start(op, context, path);
    let start = Instant::now();
    let result = f();
    log_sync_result(op, context, path, start.elapsed(), &result);
    result
}

fn log_sync_start(op: &str, context: &str, path: Option<&Path>) {
    match path {
        Some(path) => info!(
            sync_op = op,
            sync_context = context,
            path = %path.display(),
            "durability sync start"
        ),
        None => info!(
            sync_op = op,
            sync_context = context,
            "durability sync start"
        ),
    }
}

fn log_sync_skipped(op: &str, context: &str, path: Option<&Path>) {
    match path {
        Some(path) => info!(
            sync_op = op,
            sync_context = context,
            path = %path.display(),
            "durability sync skipped (fsync disabled)"
        ),
        None => info!(
            sync_op = op,
            sync_context = context,
            "durability sync skipped (fsync disabled)"
        ),
    }
}

fn log_sync_result(
    op: &str,
    context: &str,
    path: Option<&Path>,
    elapsed: std::time::Duration,
    result: &io::Result<()>,
) {
    let elapsed_ms = duration_ms(elapsed);
    match (path, result) {
        (Some(path), Ok(())) => info!(
            sync_op = op,
            sync_context = context,
            path = %path.display(),
            elapsed_ms,
            "durability sync done"
        ),
        (None, Ok(())) => info!(
            sync_op = op,
            sync_context = context,
            elapsed_ms,
            "durability sync done"
        ),
        (Some(path), Err(err)) => warn!(
            sync_op = op,
            sync_context = context,
            path = %path.display(),
            elapsed_ms,
            error = %err,
            "durability sync failed"
        ),
        (None, Err(err)) => warn!(
            sync_op = op,
            sync_context = context,
            elapsed_ms,
            error = %err,
            "durability sync failed"
        ),
    }
}

fn duration_ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
