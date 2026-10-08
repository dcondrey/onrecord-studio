//! Runs the PST reader in a child process so a crate panic, abort or runaway costs one file
//! rather than the app: release builds use `panic = "abort"`, so `catch_unwind` cannot contain it.
//!
//! The child is this app's own executable re-invoked with [`PST_CHILD_FLAG`]; `main` dispatches
//! that before Tauri starts. No second binary is signed or bundled, and a sandboxed parent's child
//! runs in the same App Sandbox container. The parent owns the staging directory: it creates it,
//! names it to the child, re-measures what the child wrote against the PST budgets, and is the
//! only side that removes or commits it.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::AppError;
use crate::parsers::pst::{expand_into_staging, MAX_ENTRY_COUNT, MAX_TOTAL_BYTES};

/// argv[1] that turns the app executable into a PST child. Only `main` matches it.
pub const PST_CHILD_FLAG: &str = "--onrecord-expand-pst";

const EXIT_EXPANDED: i32 = 0;
const EXIT_REPORTED_ERROR: i32 = 3;
const EXIT_USAGE: i32 = 4;
const EXIT_PARENT_GONE: i32 = 5;
const ERROR_LINE_PREFIX: &str = "onrecord-pst-error\t";

/// Everything beyond this is drained and dropped so a chatty child cannot block on a full pipe.
const MAX_CAPTURED_STDERR_BYTES: usize = 64 * 1024;
const MAX_REPORTED_STDERR_BYTES: usize = 512;
const WAIT_STEP: Duration = Duration::from_millis(25);
const OUTPUT_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Floor plus a 2 MiB/s throughput allowance; the reader fsyncs every file, so a mailbox of many
/// small messages is slower per byte than one of large attachments.
const BASE_TIMEOUT: Duration = Duration::from_secs(120);
const TIMEOUT_BYTES_PER_SECOND: u64 = 2 * 1024 * 1024;

pub(crate) fn child_timeout(source_bytes: u64) -> Duration {
    BASE_TIMEOUT + Duration::from_secs(source_bytes / TIMEOUT_BYTES_PER_SECOND)
}

/// Child side. Returns the process exit code; never starts Tauri.
pub fn run_pst_child(arguments: &[OsString]) -> i32 {
    let [pst, staging] = arguments else {
        eprintln!("{ERROR_LINE_PREFIX}Internal\texpected <pst> <staging>");
        return EXIT_USAGE;
    };
    // The parent holds the write end of stdin until it has reaped this process. EOF means the
    // parent is gone (an abort under panic=abort included), so stop rather than write on unowned.
    thread::spawn(|| {
        let mut sink = [0_u8; 64];
        let mut stdin = io::stdin();
        while matches!(stdin.read(&mut sink), Ok(count) if count > 0) {}
        std::process::exit(EXIT_PARENT_GONE);
    });
    #[cfg(test)]
    tests::apply_fault();
    match expand_into_staging(Path::new(pst), Path::new(staging)) {
        Ok(()) => EXIT_EXPANDED,
        Err(error) => {
            let (kind, message) = match &error {
                AppError::DataParsing(message) => ("DataParsing", message.as_str()),
                AppError::Storage(message) => ("Storage", message.as_str()),
                AppError::Internal(message) => ("Internal", message.as_str()),
                _ => ("Internal", ""),
            };
            let message = if message.is_empty() {
                error.to_string()
            } else {
                message.to_string()
            };
            eprintln!("{ERROR_LINE_PREFIX}{kind}\t{}", message.replace('\n', " "));
            EXIT_REPORTED_ERROR
        }
    }
}

/// Bounds the parent enforces on one child run.
pub(crate) struct ChildBudget {
    pub(crate) timeout: Duration,
    pub(crate) max_entries: usize,
    pub(crate) max_bytes: u64,
}

impl ChildBudget {
    pub(crate) fn for_source(source_bytes: u64) -> Self {
        Self {
            timeout: child_timeout(source_bytes),
            max_entries: MAX_ENTRY_COUNT,
            max_bytes: MAX_TOTAL_BYTES,
        }
    }
}

/// Parent side. `staging` must already exist and be empty. On `Ok` it holds only what the child
/// wrote, re-measured here; the returned paths are relative to it. On `Err` the caller removes it.
pub(crate) fn expand_in_child(
    pst: &Path,
    staging: &Path,
    budget: &ChildBudget,
    configure: impl FnOnce(&mut Command),
) -> Result<Vec<PathBuf>, AppError> {
    let mut command = child_command(pst, staging)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    configure(&mut command);
    let mut child = command.spawn().map_err(|error| {
        AppError::Internal(format!(
            "PST_CHILD_SPAWN_FAILED: failed to start the PST reader process: {error}"
        ))
    })?;
    let stderr = child.stderr.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut captured = Vec::new();
            let mut buffer = [0_u8; 8 * 1024];
            while let Ok(count) = pipe.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let room = MAX_CAPTURED_STDERR_BYTES.saturating_sub(captured.len());
                captured.extend_from_slice(&buffer[..count.min(room)]);
            }
            captured
        })
    });
    let outcome = supervise(&mut child, staging, budget);
    // Closing stdin only after the child is reaped; the child treats EOF as "parent gone".
    drop(child.stdin.take());
    let stderr = stderr
        .and_then(|reader| reader.join().ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    match outcome? {
        status if status.code() == Some(EXIT_EXPANDED) => {
            measure_staging(staging, budget.max_entries, budget.max_bytes)
        }
        status if status.code() == Some(EXIT_REPORTED_ERROR) => {
            Err(reported_error(&stderr).unwrap_or_else(|| crashed(pst, status, &stderr)))
        }
        status => Err(crashed(pst, status, &stderr)),
    }
}

#[cfg(not(test))]
fn child_command(pst: &Path, staging: &Path) -> Result<Command, AppError> {
    let executable = std::env::current_exe().map_err(|error| {
        AppError::Internal(format!(
            "PST_CHILD_SPAWN_FAILED: failed to locate the app executable: {error}"
        ))
    })?;
    let mut command = Command::new(executable);
    command.arg(PST_CHILD_FLAG).arg(pst).arg(staging);
    Ok(command)
}

/// Under `cargo test` the executable is the test harness, so the child is reached through an
/// ignored test that calls the same [`run_pst_child`].
#[cfg(test)]
fn child_command(pst: &Path, staging: &Path) -> Result<Command, AppError> {
    let executable = std::env::current_exe()
        .map_err(|error| AppError::Internal(format!("PST_CHILD_SPAWN_FAILED: {error}")))?;
    let mut command = Command::new(executable);
    command
        .args([
            "--ignored",
            "--exact",
            "parsers::pst_isolation::tests::pst_child_entry",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(tests::CHILD_PST_ENV, pst)
        .env(tests::CHILD_STAGING_ENV, staging);
    Ok(command)
}

fn supervise(
    child: &mut Child,
    staging: &Path,
    budget: &ChildBudget,
) -> Result<ExitStatus, AppError> {
    let started = Instant::now();
    let mut next_output_check = started + OUTPUT_CHECK_INTERVAL;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            AppError::Internal(format!(
                "PST_CHILD_WAIT_FAILED: failed to wait for the PST reader process: {error}"
            ))
        })? {
            return Ok(status);
        }
        let now = Instant::now();
        if now.duration_since(started) >= budget.timeout {
            kill(child);
            return Err(AppError::DataParsing(format!(
                "PST_CHILD_TIMEOUT: the PST reader did not finish within {} seconds and was stopped; no messages were extracted from this file",
                budget.timeout.as_secs()
            )));
        }
        if now >= next_output_check {
            next_output_check = now + OUTPUT_CHECK_INTERVAL;
            if let Err(error) = measure_staging(staging, budget.max_entries, budget.max_bytes) {
                kill(child);
                return Err(error);
            }
        }
        thread::sleep(WAIT_STEP);
    }
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn reported_error(stderr: &str) -> Option<AppError> {
    let line = stderr
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(ERROR_LINE_PREFIX))?;
    let (kind, message) = line.split_once('\t')?;
    let message = message.to_string();
    Some(match kind {
        "DataParsing" => AppError::DataParsing(message),
        "Storage" => AppError::Storage(message),
        _ => AppError::Internal(message),
    })
}

/// A crash is a failure of this file, never an empty mailbox.
fn crashed(pst: &Path, status: ExitStatus, stderr: &str) -> AppError {
    let how = describe_exit(status);
    let tail = stderr.trim();
    let tail = match tail.char_indices().rev().nth(MAX_REPORTED_STDERR_BYTES) {
        Some((start, _)) => &tail[start..],
        None => tail,
    };
    let tail = tail.replace('\n', " | ");
    AppError::DataParsing(format!(
        "PST_CHILD_CRASHED: the PST reader stopped abnormally ({how}) while reading '{}'; no messages were extracted from this file{}",
        pst.display(),
        if tail.is_empty() {
            String::new()
        } else {
            format!(". Reader output: {tail}")
        }
    ))
}

fn describe_exit(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    match status.code() {
        // A Windows crash exits with an NTSTATUS (high bit set), which is only legible in hex:
        // `abort()` is a fail-fast, 0xC0000409.
        #[cfg(windows)]
        Some(code) if code < 0 => format!("exit code 0x{:08X}", code as u32),
        Some(code) => format!("exit code {code}"),
        None => "unknown exit status".to_string(),
    }
}

/// Walks what the child wrote without following links and returns the files relative to
/// `staging`, sorted. Counts directories as entries, as the cached re-run does.
pub(crate) fn measure_staging(
    staging: &Path,
    max_entries: usize,
    max_bytes: u64,
) -> Result<Vec<PathBuf>, AppError> {
    let inspect = |path: &Path, error: io::Error| {
        AppError::Internal(format!(
            "PST_OUTPUT_INSPECTION_FAILED: failed to inspect '{}': {error}",
            path.display()
        ))
    };
    let mut directories = vec![staging.to_path_buf()];
    let mut files = Vec::new();
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| inspect(&directory, error))? {
            let path = entry.map_err(|error| inspect(&directory, error))?.path();
            entries += 1;
            if entries > max_entries {
                return Err(AppError::DataParsing(format!(
                    "PST_ENTRY_COUNT_EXCEEDED: mailbox expands beyond {max_entries} files and folders"
                )));
            }
            let metadata = fs::symlink_metadata(&path).map_err(|error| inspect(&path, error))?;
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
                if bytes > max_bytes {
                    return Err(AppError::DataParsing(format!(
                        "PST_TOTAL_SIZE_EXCEEDED: expanded data exceeds {max_bytes} bytes"
                    )));
                }
                let relative = path
                    .strip_prefix(staging)
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "PST_OUTPUT_PATH_INVALID: '{}' is outside staging: {error}",
                            path.display()
                        ))
                    })?
                    .to_path_buf();
                files.push(relative);
            } else {
                return Err(AppError::PortalSecurity(format!(
                    "PST_OUTPUT_ENTRY_INVALID: '{}' is not a regular file or directory",
                    path.display()
                )));
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(super) const CHILD_PST_ENV: &str = "ONRECORD_TEST_PST_CHILD_SOURCE";
    pub(super) const CHILD_STAGING_ENV: &str = "ONRECORD_TEST_PST_CHILD_STAGING";
    pub(crate) const FAULT_ENV: &str = "ONRECORD_TEST_PST_CHILD_FAULT";

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/pst/tika-various-body-types.pst"
    );

    /// Test-build-only stand-ins for a crate failure; the release child has no such hook.
    pub(super) fn apply_fault() {
        match std::env::var(FAULT_ENV).as_deref() {
            Ok("abort") => std::process::abort(),
            Ok("panic") => panic!("simulated outlook-pst panic"),
            // Long enough to outlive any timeout under test, short enough that a missing kill
            // ends as a wrong result rather than a hung suite.
            Ok("hang") => thread::sleep(Duration::from_secs(20)),
            _ => {}
        }
    }

    /// The child process under `cargo test`; inert unless the parent set its environment.
    #[test]
    #[ignore = "entry point for the PST child process, run only by expand_in_child"]
    fn pst_child_entry() {
        let (Some(pst), Some(staging)) = (
            std::env::var_os(CHILD_PST_ENV),
            std::env::var_os(CHILD_STAGING_ENV),
        ) else {
            return;
        };
        std::process::exit(run_pst_child(&[pst, staging]));
    }

    struct ChildRun {
        _directory: tempfile::TempDir,
        staging: PathBuf,
        result: Result<Vec<PathBuf>, AppError>,
        elapsed: Duration,
    }

    fn run_with(fault: &str, timeout: Duration) -> ChildRun {
        let directory = tempfile::tempdir().expect("temp dir");
        let staging = directory.path().join("staging");
        fs::create_dir(&staging).expect("staging");
        let budget = ChildBudget {
            timeout,
            max_entries: MAX_ENTRY_COUNT,
            max_bytes: MAX_TOTAL_BYTES,
        };
        let started = Instant::now();
        let result = expand_in_child(Path::new(FIXTURE), &staging, &budget, |command| {
            command.env(FAULT_ENV, fault);
        });
        ChildRun {
            _directory: directory,
            staging,
            result,
            elapsed: started.elapsed(),
        }
    }

    #[test]
    fn a_child_that_aborts_or_panics_is_a_recorded_failure_of_that_file() {
        // Rust's `abort()` raises SIGABRT on Unix and is a fail-fast on Windows.
        let aborted = if cfg!(windows) {
            "exit code 0xC0000409"
        } else {
            "signal 6"
        };
        for (fault, how) in [("abort", aborted), ("panic", "exit code 101")] {
            let error = run_with(fault, Duration::from_secs(60))
                .result
                .expect_err("a crashed reader is never an empty mailbox");
            let message = error.to_string();
            assert!(
                matches!(error, AppError::DataParsing(_)),
                "not retried as transient: {message}"
            );
            assert!(message.contains("PST_CHILD_CRASHED"), "{message}");
            assert!(message.contains(how), "{fault}: {message}");
            if fault == "panic" {
                assert!(message.contains("simulated outlook-pst panic"), "{message}");
            }
        }
    }

    #[test]
    fn a_child_that_runs_past_its_timeout_is_killed() {
        let run = run_with("hang", Duration::from_secs(1));
        let message = run
            .result
            .expect_err("a hung reader is stopped")
            .to_string();
        assert!(message.contains("PST_CHILD_TIMEOUT"), "{message}");
        assert!(
            run.elapsed < Duration::from_secs(10),
            "killed, not awaited: {:?}",
            run.elapsed
        );
    }

    #[test]
    fn a_healthy_child_expands_into_the_staging_it_was_given() {
        let run = run_with("none", Duration::from_secs(60));
        let files = run.result.expect("expanded");
        assert_eq!(files.len(), 4, "{files:?}");
        assert!(files.iter().all(|file| run.staging.join(file).is_file()));
    }

    #[test]
    fn a_reported_error_keeps_its_kind_and_message() {
        let error =
            reported_error("noise\nonrecord-pst-error\tDataParsing\tPST_OPEN_FAILED: bad\n")
                .expect("parsed");
        assert!(
            matches!(&error, AppError::DataParsing(message) if message == "PST_OPEN_FAILED: bad")
        );
    }

    #[test]
    fn the_parent_measures_child_output_at_the_limit() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::create_dir(directory.path().join("Inbox")).expect("dir");
        fs::write(directory.path().join("Inbox/a.txt"), b"12").expect("file");
        fs::write(directory.path().join("b.txt"), b"345").expect("file");
        // Three entries (Inbox, a.txt, b.txt) and five bytes.
        for entries in [2, 3, 4] {
            let result = measure_staging(directory.path(), entries, 5);
            assert_eq!(result.is_ok(), entries >= 3, "{entries}: {result:?}");
        }
        for bytes in [4, 5, 6] {
            let result = measure_staging(directory.path(), 3, bytes);
            assert_eq!(result.is_ok(), bytes >= 5, "{bytes}: {result:?}");
        }
        assert_eq!(
            measure_staging(directory.path(), 3, 5).expect("within"),
            vec![PathBuf::from("Inbox/a.txt"), PathBuf::from("b.txt")]
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", directory.path().join("link")).expect("link");
            assert!(measure_staging(directory.path(), 10, 1_000).is_err());
        }
    }
}
