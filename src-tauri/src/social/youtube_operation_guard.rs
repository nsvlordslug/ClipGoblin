//! Serialize all YouTube operations for one database/clip, across formats.
//!
//! Acquire before reading/claiming upload history and retain the guard through
//! network work and final persistence. Windows uses an exclusive file handle,
//! which the OS releases on process exit; lockfiles must never be unlinked.
//! Other platforms currently provide in-process exclusion only.

use crate::error::AppError;
use crate::DbConn;
use sha2::{Digest, Sha256};

#[cfg(not(windows))]
use std::collections::HashSet;
#[cfg(windows)]
use std::fs::File;
#[cfg(not(windows))]
use std::sync::{Mutex, OnceLock};

#[cfg(not(windows))]
static ACTIVE_OPERATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Its lifetime owns the operation slot; dropping it allows another operation.
#[derive(Debug)]
pub struct YouTubeOperationGuard {
    #[cfg(windows)]
    _file: File,
    #[cfg(not(windows))]
    key: String,
}

fn busy_error() -> AppError {
    AppError::Api(
        "A YouTube operation for this clip is already running. Wait for it to finish before checking or resuming.".to_string(),
    )
}

fn database_identity(db: &DbConn) -> Result<String, AppError> {
    let filename = {
        let conn = db
            .lock()
            .map_err(|_| AppError::Database("Upload database is unavailable".to_string()))?;
        conn.query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get::<_, String>(0),
        )?
    };
    if filename.is_empty() {
        // Separate in-memory databases have no disk identity. The managed
        // DbConn address is stable for its lifetime; process ID separates apps.
        return Ok(format!("memory:{}:{db:p}", std::process::id()));
    }
    let canonical = std::fs::canonicalize(filename).map_err(|error| {
        AppError::Database(format!(
            "Could not identify the upload database ({:?})",
            error.kind()
        ))
    })?;
    let identity = canonical.to_string_lossy().into_owned();
    #[cfg(windows)]
    let identity = identity.to_lowercase();
    Ok(format!("file:{identity}"))
}

fn operation_key(db: &DbConn, clip_id: &str) -> Result<String, AppError> {
    let database = database_identity(db)?;
    let mut hash = Sha256::new();
    hash.update(b"clipgoblin-youtube-operation-v1");
    hash.update((database.len() as u64).to_le_bytes());
    hash.update(database.as_bytes());
    hash.update((clip_id.len() as u64).to_le_bytes());
    hash.update(clip_id.as_bytes());
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Does not hold the database mutex after returning. Call without a DB guard.
pub fn acquire(db: &DbConn, clip_id: &str) -> Result<YouTubeOperationGuard, AppError> {
    let key = operation_key(db, clip_id)?;
    acquire_key(key)
}

#[cfg(windows)]
fn acquire_key(key: String) -> Result<YouTubeOperationGuard, AppError> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let filesystem_error = |error: std::io::Error| {
        AppError::Unknown(format!(
            "Could not acquire the YouTube operation lock ({:?})",
            error.kind()
        ))
    };
    let temp = std::fs::canonicalize(std::env::temp_dir()).map_err(filesystem_error)?;
    let directory = temp.join("clipgoblin-youtube-locks");
    std::fs::create_dir_all(&directory).map_err(filesystem_error)?;
    let directory = std::fs::canonicalize(&directory).map_err(filesystem_error)?;
    if directory.parent() != Some(temp.as_path()) {
        return Err(AppError::Unknown(
            "YouTube operation lock folder is invalid".into(),
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(directory.join(format!("{key}.lock")))
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(32) | Some(33)) {
                busy_error()
            } else {
                filesystem_error(error)
            }
        })?;
    let metadata = file.metadata().map_err(filesystem_error)?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(AppError::Unknown(
            "YouTube operation lock file is invalid".into(),
        ));
    }
    Ok(YouTubeOperationGuard { _file: file })
}

#[cfg(not(windows))]
fn acquire_key(key: String) -> Result<YouTubeOperationGuard, AppError> {
    // No new native dependency or post-1.77 std locking API. This fallback does
    // not protect two app processes; Windows is the supported desktop target.
    let mut active = ACTIVE_OPERATIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| AppError::Unknown("YouTube operation lock is unavailable".into()))?;
    if !active.insert(key.clone()) {
        return Err(busy_error());
    }
    Ok(YouTubeOperationGuard { key })
}

#[cfg(not(windows))]
impl Drop for YouTubeOperationGuard {
    fn drop(&mut self) {
        let active = ACTIVE_OPERATIONS.get_or_init(|| Mutex::new(HashSet::new()));
        match active.lock() {
            Ok(mut guard) => {
                guard.remove(&self.key);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    #[cfg(windows)]
    const CHILD_DATABASE_ENV: &str = "CLIPGOBLIN_YOUTUBE_LOCK_CHILD_DATABASE";
    #[cfg(windows)]
    const CHILD_EXPECTATION_ENV: &str = "CLIPGOBLIN_YOUTUBE_LOCK_CHILD_EXPECTATION";

    #[cfg(windows)]
    fn run_child_probe(path: &std::path::Path, expectation: &str) {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "social::youtube_operation_guard::tests::windows_operation_child_probe",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD_DATABASE_ENV, path)
            .env(CHILD_EXPECTATION_ENV, expectation)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "child lock probe failed: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
                assert!(
                    String::from_utf8_lossy(&output.stdout)
                        .contains(&format!("CLIPGOBLIN_LOCK_PROBE_OK:{expectation}")),
                    "the exact-filter child helper did not confirm the expected operation",
                );
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child lock probe did not finish within 15 seconds");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Runs only when the subprocess test supplies its isolated database path.
    #[cfg(windows)]
    #[test]
    fn windows_operation_child_probe() {
        let Some(path) = std::env::var_os(CHILD_DATABASE_ENV) else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        assert_eq!(path.file_name().unwrap(), "test.db");
        assert!(path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("clipgoblin-lock-process-test-"));
        assert_eq!(
            std::fs::canonicalize(path.parent().unwrap().parent().unwrap()).unwrap(),
            std::fs::canonicalize(std::env::temp_dir()).unwrap(),
        );
        let db = Mutex::new(Connection::open(&path).unwrap());
        match std::env::var(CHILD_EXPECTATION_ENV).unwrap().as_str() {
            "busy" => {
                let error = acquire(&db, "cross-process-clip").unwrap_err();
                assert!(error.detail().contains("already running"));
                println!("CLIPGOBLIN_LOCK_PROBE_OK:busy");
            }
            "acquire-and-exit" => {
                let _held = acquire(&db, "cross-process-clip").unwrap();
                use std::io::Write;
                println!("CLIPGOBLIN_LOCK_PROBE_OK:acquire-and-exit");
                std::io::stdout().flush().unwrap();
                // Skip Rust destructors deliberately. The operating system,
                // rather than our Drop implementation, must release the lock.
                std::process::exit(0);
            }
            other => panic!("unexpected child lock probe: {other}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_operation_lock_excludes_other_processes_and_releases_on_process_exit() {
        let directory = std::env::temp_dir().join(format!(
            "clipgoblin-lock-process-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.db");
        let db = Mutex::new(Connection::open(&path).unwrap());
        let held = acquire(&db, "cross-process-clip").unwrap();
        run_child_probe(&path, "busy");
        drop(held);
        run_child_probe(&path, "acquire-and-exit");
        // Child exited while holding its file handle: reacquisition proves the
        // OS released it without stale-state cleanup or lockfile deletion.
        drop(acquire(&db, "cross-process-clip").unwrap());
        drop(db);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn concurrent_same_clip_is_rejected_and_slot_reopens_after_drop() {
        let db = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let clip = uuid::Uuid::new_v4().to_string();
        let held = acquire(db.as_ref(), &clip).unwrap();
        let other_db = Arc::clone(&db);
        let other_clip = clip.clone();
        assert!(
            std::thread::spawn(move || acquire(other_db.as_ref(), &other_clip).is_err())
                .join()
                .unwrap()
        );
        assert!(acquire(db.as_ref(), "another-clip").is_ok());
        drop(held);
        assert!(acquire(db.as_ref(), &clip).is_ok());
    }

    #[test]
    fn independent_memory_databases_have_separate_stable_operation_keys() {
        let first = Mutex::new(Connection::open_in_memory().unwrap());
        let second = Mutex::new(Connection::open_in_memory().unwrap());
        let key = operation_key(&first, "clip").unwrap();
        assert_eq!(key, operation_key(&first, "clip").unwrap());
        assert_ne!(key, operation_key(&second, "clip").unwrap());
        let _first = acquire(&first, "clip").unwrap();
        assert!(acquire(&second, "clip").is_ok());
    }

    #[test]
    fn separate_connections_to_same_database_share_the_operation_key() {
        let directory =
            std::env::temp_dir().join(format!("clipgoblin-lock-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.db");
        let first = Mutex::new(Connection::open(&path).unwrap());
        let second = Mutex::new(Connection::open(directory.join(".").join("test.db")).unwrap());
        assert_eq!(
            operation_key(&first, "clip").unwrap(),
            operation_key(&second, "clip").unwrap()
        );
        let held = acquire(&first, "clip").unwrap();
        assert!(acquire(&second, "clip").is_err());
        drop(held);
        assert!(acquire(&second, "clip").is_ok());
        drop(first);
        drop(second);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
