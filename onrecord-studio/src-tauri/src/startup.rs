//! What the window shows while startup maintenance runs, and the notices that maintenance leaves.
//!
//! The window is presented before the databases open, so the webview attaches at an arbitrary
//! point in the sequence and an event emitted before it listened is lost. The status is therefore
//! pulled (`get_startup_status`), each change carries a monotonic `version` so a late reply never
//! overwrites a newer one, and notices are written to disk before the step that could fail next,
//! so a launch that dies afterwards still reports them on the following one.

use crate::atomic_write::write_atomic;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// Emitted after every status change as a hint to re-fetch; the payload is the full status.
pub const STARTUP_STATUS_EVENT: &str = "startup-status";
const NOTICE_FILE: &str = "startup-notices.json";
/// Bounds the persisted list: one notice per file per kind family is the steady state.
const MAXIMUM_NOTICES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupPhase {
    Preparing,
    /// `rewrite_to_current_storage` is running on `file`; on a current file it returns at once.
    OptimizingStorage,
    OpeningRecords,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupNoticeKind {
    StorageRewritten,
    /// The rewrite was attempted and refused or failed; the original stays in use.
    StorageNotRewritten,
    LegacyDatabaseMigrated,
}

impl StartupNoticeKind {
    fn is_storage(self) -> bool {
        matches!(self, Self::StorageRewritten | Self::StorageNotRewritten)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupNotice {
    pub id: String,
    pub kind: StartupNoticeKind,
    pub file: String,
    pub message: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupStatus {
    pub version: u64,
    pub phase: StartupPhase,
    pub file: Option<String>,
    /// Unix epoch milliseconds, so elapsed time stays correct however late the webview attaches.
    pub phase_started_at_ms: u64,
    pub error: Option<String>,
    /// Outcomes not yet dismissed, including any an earlier launch left undismissed.
    pub notices: Vec<StartupNotice>,
}

pub struct StartupState {
    status: Mutex<StartupStatus>,
    notice_path: PathBuf,
}

impl StartupState {
    pub fn load(app_data_dir: &Path) -> Self {
        let notice_path = app_data_dir.join(NOTICE_FILE);
        let notices = match std::fs::read(&notice_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                eprintln!(
                    "Startup notices at '{}' could not be read and are discarded: {error}",
                    notice_path.display()
                );
                Vec::new()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                eprintln!(
                    "Startup notices at '{}' could not be read: {error}",
                    notice_path.display()
                );
                Vec::new()
            }
        };
        Self {
            status: Mutex::new(StartupStatus {
                version: 1,
                phase: StartupPhase::Preparing,
                file: None,
                phase_started_at_ms: now_ms(),
                error: None,
                notices,
            }),
            notice_path,
        }
    }

    pub fn snapshot(&self) -> StartupStatus {
        self.lock().clone()
    }

    pub fn enter(&self, phase: StartupPhase, file: Option<String>) -> StartupStatus {
        let mut status = self.lock();
        status.version += 1;
        status.phase = phase;
        status.file = file;
        status.phase_started_at_ms = now_ms();
        status.clone()
    }

    pub fn fail(&self, message: String) -> StartupStatus {
        let mut status = self.lock();
        status.version += 1;
        status.phase = StartupPhase::Failed;
        status.file = None;
        status.phase_started_at_ms = now_ms();
        status.error = Some(message);
        status.clone()
    }

    /// Replaces any earlier notice of the same family for the same file: a refusal repeated on
    /// every launch is one standing notice, not a growing list.
    pub fn record_notice(
        &self,
        kind: StartupNoticeKind,
        file: &str,
        message: String,
    ) -> StartupStatus {
        let mut status = self.lock();
        status.notices.retain(|notice| {
            !(notice.file == file && notice.kind.is_storage() == kind.is_storage())
        });
        status.notices.push(StartupNotice {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            file: file.to_string(),
            message,
            recorded_at: chrono::Utc::now().to_rfc3339(),
        });
        let excess = status.notices.len().saturating_sub(MAXIMUM_NOTICES);
        status.notices.drain(..excess);
        status.version += 1;
        self.persist(&status.notices);
        status.clone()
    }

    pub fn acknowledge(&self, ids: &[String]) -> StartupStatus {
        let mut status = self.lock();
        let before = status.notices.len();
        status.notices.retain(|notice| !ids.contains(&notice.id));
        if status.notices.len() != before {
            status.version += 1;
            self.persist(&status.notices);
        }
        status.clone()
    }

    /// A failed write keeps the notice for this session only; it is never fatal to startup.
    fn persist(&self, notices: &[StartupNotice]) {
        let written = serde_json::to_vec_pretty(notices)
            .map_err(|error| {
                AppError::Storage(format!("Failed to encode startup notices: {error}"))
            })
            .and_then(|bytes| write_atomic(&self.notice_path, "the startup notices", &bytes));
        if let Err(error) = written {
            eprintln!("{error}");
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StartupStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{StartupNoticeKind, StartupState};

    #[test]
    fn notices_survive_a_restart_until_dismissed_and_repeat_as_one() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let first = StartupState::load(directory.path());
        first.record_notice(
            StartupNoticeKind::StorageNotRewritten,
            "core.db",
            "needs 10 MB free".into(),
        );
        first.record_notice(
            StartupNoticeKind::LegacyDatabaseMigrated,
            "core.db",
            "migrated".into(),
        );
        let refused_again = first.record_notice(
            StartupNoticeKind::StorageNotRewritten,
            "core.db",
            "needs 12 MB free".into(),
        );
        assert_eq!(refused_again.notices.len(), 2);

        let relaunched = StartupState::load(directory.path());
        let notices = relaunched.snapshot().notices;
        assert_eq!(notices, refused_again.notices);
        let storage = notices
            .iter()
            .find(|notice| notice.kind == StartupNoticeKind::StorageNotRewritten)
            .expect("the storage notice must persist");
        assert_eq!(storage.message, "needs 12 MB free");

        relaunched.acknowledge(std::slice::from_ref(&storage.id));
        let after_dismissal = StartupState::load(directory.path()).snapshot().notices;
        assert_eq!(after_dismissal.len(), 1);
        assert_eq!(
            after_dismissal[0].kind,
            StartupNoticeKind::LegacyDatabaseMigrated
        );
    }
}
