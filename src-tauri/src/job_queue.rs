//! Background job queue with bounded concurrency and event notification.
//!
//! Jobs are submitted via [`JobQueue::add_job`] and execute asynchronously.
//! A [`tokio::sync::Semaphore`] limits how many jobs run at once (default 2).
//! Each running job receives a [`JobHandle`] to report its own progress.
//!
//! Register an [`OnJobEvent`] callback via [`JobQueue::on_progress`] to
//! forward state changes to any sink (Tauri events, logging, etc.).
//! This module has **zero** Tauri dependency — the integration lives in lib.rs.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Maximum number of jobs that execute concurrently.
const MAX_CONCURRENT: usize = 2;

// ── Public types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Job {
    pub id: String,
    pub status: JobStatus,
    pub progress: u8,
    /// Human-readable error when `status == Failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Payload passed to the [`OnJobEvent`] callback on every state change.
/// Fields serialize as camelCase to match JS/TS conventions.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub job_id: String,
    pub progress: u8,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl JobEvent {
    fn from_job(job: &Job) -> Self {
        Self {
            job_id: job.id.clone(),
            progress: job.progress,
            status: job.status,
            error: job.error.clone(),
        }
    }
}

/// Callback type invoked on every job status/progress change.
pub type OnJobEvent = Arc<dyn Fn(JobEvent) + Send + Sync>;

// ── Job handle ──

/// Lightweight, cloneable handle given to each job so it can report progress.
#[derive(Clone)]
pub struct JobHandle {
    id: String,
    state: Arc<Mutex<HashMap<String, Job>>>,
    on_event: Arc<Mutex<Option<OnJobEvent>>>,
}

impl JobHandle {
    /// Update this job's progress (clamped to 0–100) and notify listeners.
    pub fn set_progress(&self, pct: u8) {
        let Ok(mut map) = self.state.lock() else {
            log::error!("Job state mutex poisoned in set_progress for job {}", self.id);
            return;
        };
        if let Some(job) = map.get_mut(&self.id) {
            job.progress = pct.min(100);
            notify(&self.on_event, job);
        }
    }

    /// The unique id of this job.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Fire the callback if one is registered.
fn notify(cb: &Arc<Mutex<Option<OnJobEvent>>>, job: &Job) {
    if let Ok(guard) = cb.lock() {
        if let Some(f) = guard.as_ref() {
            f(JobEvent::from_job(job));
        }
    }
}

// ── Queue ──

/// Thread-safe job queue. Register with Tauri via `.manage(JobQueue::new())`.
pub struct JobQueue {
    state: Arc<Mutex<HashMap<String, Job>>>,
    sem: Arc<Semaphore>,
    on_event: Arc<Mutex<Option<OnJobEvent>>>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            sem: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            on_event: Arc::new(Mutex::new(None)),
        }
    }

    /// Register a callback invoked on every job state change.
    /// Typically wired once in a Tauri `.setup()` hook to emit events.
    pub fn on_progress(&self, cb: impl Fn(JobEvent) + Send + Sync + 'static) {
        match self.on_event.lock() {
            Ok(mut guard) => *guard = Some(Arc::new(cb)),
            Err(e) => log::error!("on_event mutex poisoned, cannot register callback: {e}"),
        }
    }

    /// Submit a job for background execution.
    ///
    /// `id`   – caller-chosen identifier (e.g. `"transcribe-{vod_id}"`).
    /// `task` – async closure that receives a [`JobHandle`] for progress
    ///          updates and returns `Ok(())` on success or `Err(message)`.
    ///
    /// The job is marked **Queued** immediately. When a concurrency slot
    /// opens it transitions to **Running**, and finally to **Completed**
    /// or **Failed** depending on the closure's return value. Each
    /// transition fires the [`on_progress`](Self::on_progress) callback.
    ///
    /// Returns the job id.
    pub fn add_job<F, Fut>(&self, id: impl Into<String>, task: F) -> String
    where
        F: FnOnce(JobHandle) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let id = id.into();

        // Register job as Queued and notify.
        {
            let Ok(mut map) = self.state.lock() else {
                log::error!("Job state mutex poisoned, cannot add job {}", id);
                return id;
            };
            let job = Job {
                id: id.clone(),
                status: JobStatus::Queued,
                progress: 0,
                error: None,
            };
            notify(&self.on_event, &job);
            map.insert(id.clone(), job);
        }

        let sem = self.sem.clone();
        let state = self.state.clone();
        let on_event = self.on_event.clone();
        let job_id = id.clone();

        // Spawn an async task that waits for a semaphore permit, then runs.
        tokio::spawn(async move {
            // Wait until a concurrency slot is available.
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return, // semaphore closed — runtime shutting down
            };

            // Queued → Running
            {
                let Ok(mut map) = state.lock() else {
                    log::error!("Job state mutex poisoned transitioning {} to Running", job_id);
                    return;
                };
                if let Some(job) = map.get_mut(&job_id) {
                    job.status = JobStatus::Running;
                    notify(&on_event, job);
                }
            }

            let handle = JobHandle {
                id: job_id.clone(),
                state: state.clone(),
                on_event: on_event.clone(),
            };

            // Execute the user-provided async work.
            let result = task(handle).await;

            // Running → Completed | Failed
            {
                let Ok(mut map) = state.lock() else {
                    log::error!("Job state mutex poisoned transitioning {} to final state", job_id);
                    return;
                };
                if let Some(job) = map.get_mut(&job_id) {
                    match result {
                        Ok(()) => {
                            job.status = JobStatus::Completed;
                            job.progress = 100;
                        }
                        Err(e) => {
                            job.status = JobStatus::Failed;
                            job.error = Some(e);
                        }
                    }
                    notify(&on_event, job);
                }
            }
            // _permit dropped here → next queued job can start
        });

        id
    }

    /// Snapshot of all jobs (any status).
    pub fn list(&self) -> Vec<Job> {
        let Ok(map) = self.state.lock() else {
            log::error!("Job state mutex poisoned in list()");
            return Vec::new();
        };
        map.values().cloned().collect()
    }

    /// Snapshot of a single job, or `None` if the id is unknown.
    pub fn get(&self, id: &str) -> Option<Job> {
        self.state.lock().ok()?.get(id).cloned()
    }

    /// Update progress for a job from outside the job closure and notify.
    pub fn update_progress(&self, id: &str, pct: u8) {
        let Ok(mut map) = self.state.lock() else {
            log::error!("Job state mutex poisoned in update_progress for {}", id);
            return;
        };
        if let Some(job) = map.get_mut(id) {
            job.progress = pct.min(100);
            notify(&self.on_event, job);
        }
    }

    /// Remove a finished job (Completed or Failed) from the map.
    /// Returns `true` if the job was removed.
    pub fn remove(&self, id: &str) -> bool {
        let Ok(mut map) = self.state.lock() else {
            log::error!("Job state mutex poisoned in remove for {}", id);
            return false;
        };
        match map.get(id) {
            Some(j) if matches!(j.status, JobStatus::Completed | JobStatus::Failed) => {
                map.remove(id);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn job_lifecycle() {
        let q = JobQueue::new();

        q.add_job("test-1", |handle| async move {
            handle.set_progress(50);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            handle.set_progress(100);
            Ok(())
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let job = q.get("test-1").expect("job should exist");
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.progress, 100);
    }

    #[tokio::test]
    async fn callback_receives_all_transitions() {
        let q = JobQueue::new();
        let log: Arc<Mutex<Vec<(String, JobStatus, u8)>>> = Arc::new(Mutex::new(Vec::new()));

        let log_cb = log.clone();
        q.on_progress(move |evt| {
            log_cb.lock().unwrap().push((evt.job_id, evt.status, evt.progress));
        });

        q.add_job("cb-1", |handle| async move {
            handle.set_progress(40);
            Ok(())
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let entries = log.lock().unwrap().clone();
        // Expect: Queued(0) → Running(0) → progress(40) → Completed(100)
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0], ("cb-1".into(), JobStatus::Queued, 0));
        assert_eq!(entries[1], ("cb-1".into(), JobStatus::Running, 0));
        assert_eq!(entries[2], ("cb-1".into(), JobStatus::Running, 40));
        assert_eq!(entries[3], ("cb-1".into(), JobStatus::Completed, 100));
    }

    #[tokio::test]
    async fn failed_job_records_error() {
        let q = JobQueue::new();

        q.add_job("fail-1", |_handle| async move {
            Err("something broke".to_string())
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let job = q.get("fail-1").expect("job should exist");
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("something broke"));
    }

    #[tokio::test]
    async fn concurrency_is_bounded() {
        let q = Arc::new(JobQueue::new());
        let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for i in 0..6 {
            let r = running.clone();
            let p = peak.clone();
            q.add_job(format!("job-{i}"), move |_handle| async move {
                let current = r.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                p.fetch_max(current, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                r.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let peak_val = peak.load(std::sync::atomic::Ordering::SeqCst);
        assert!(peak_val <= MAX_CONCURRENT, "peak concurrency {peak_val} exceeded limit {MAX_CONCURRENT}");

        let completed = q.list().iter().filter(|j| j.status == JobStatus::Completed).count();
        assert_eq!(completed, 6);
    }

    #[tokio::test]
    async fn remove_only_finished() {
        let q = JobQueue::new();

        q.add_job("done", |_| async { Ok(()) });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(q.remove("done"));
        assert!(q.get("done").is_none());

        q.add_job("busy", |_| async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(())
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!q.remove("busy"));
    }

    #[test]
    fn job_event_serializes_to_camel_case() {
        let event = JobEvent {
            job_id: "abc".into(),
            progress: 42,
            status: JobStatus::Running,
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("jobId").is_some());
        assert!(json.get("progress").is_some());
        assert!(json.get("status").is_some());
        assert!(json.get("error").is_none()); // skipped when None
    }
}
