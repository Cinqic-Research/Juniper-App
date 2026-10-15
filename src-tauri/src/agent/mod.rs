//! Juniper's agent runtime (Beta). The native host owns task scheduling,
//! tool dispatch, permission decisions, lifecycle, side-effect records, and
//! durable state; the interface only presents tasks and sends user intents.
//! See docs/architecture/agent-runtime.md.

pub mod commands;
pub mod context;
pub mod mcp;
pub mod policy;
pub mod retrieval;
pub mod runner;
pub mod sandbox;
pub mod schedule;
pub mod skills;
pub mod store;
#[cfg(test)]
mod tests;
pub mod tools;
pub mod types;
pub mod web;
pub mod workspace;

use crate::commands::Cancellation;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::Notify;

pub const EVENT_TOPIC: &str = "juniper://agent/event";

/// Why an executing task was asked to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Pause,
    /// A pending task must be shown to the user again after its model route
    /// changes, before any saved private context is sent to that destination.
    PauseForRouteChange,
    /// A pending task must be explicitly continued after its agent policy
    /// changes.
    PauseForProfileChange,
    Cancel,
    /// The global stop: running work is interrupted and nothing new starts.
    Stop,
}

#[derive(Default)]
pub struct AgentState {
    paths: OnceLock<Paths>,
    /// Prevents a second app process from running recovery or the task worker
    /// against the same database. Held until this AppState is dropped.
    runtime_lock: Mutex<Option<File>>,
    pub wake: Notify,
    /// Tasks executing right now, with their cancellation handle and the
    /// reason one was raised.
    active: Mutex<HashMap<String, (Cancellation, Option<Intent>)>>,
    /// Serializes agent context resolution against application route changes.
    route_configuration: Mutex<()>,
    pub stopped: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub database: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn artifacts(&self) -> PathBuf {
        self.data.join("agent").join("artifacts")
    }

    #[cfg(target_os = "linux")]
    pub fn sandboxes(&self) -> PathBuf {
        self.data.join("agent").join("sandbox")
    }
}

impl AgentState {
    pub(crate) fn lock_route_configuration(&self) -> std::sync::MutexGuard<'_, ()> {
        self.route_configuration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn acquire_runtime_lock(&self, path: &std::path::Path) -> io::Result<()> {
        let file = OpenOptions::new().write(true).create(true).open(path)?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Another Juniper process already owns this data directory. Close it before opening another window.",
            ),
            std::fs::TryLockError::Error(error) => error,
        })?;
        let mut lock = self
            .runtime_lock
            .lock()
            .map_err(|_| io::Error::other("Agent runtime lock state is unavailable."))?;
        if lock.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "This Juniper process already owns the agent runtime lock.",
            ));
        }
        *lock = Some(file);
        Ok(())
    }

    pub fn paths(&self) -> Result<&Paths, String> {
        self.paths
            .get()
            .ok_or_else(|| "AGENT_UNAVAILABLE: The agent runtime has not started.".into())
    }

    pub fn connection(&self) -> Result<rusqlite::Connection, String> {
        store::open(&self.paths()?.database)
    }

    pub(crate) fn begin(&self, task_id: &str) -> Result<Cancellation, String> {
        let cancellation = Cancellation::default();
        let mut active = self.active.lock().map_err(|_| "AGENT_STATE_ERROR")?;
        if active.contains_key(task_id) {
            return Err("TASK_ALREADY_RUNNING".into());
        }
        active.insert(task_id.to_owned(), (cancellation.clone(), None));
        Ok(cancellation)
    }

    pub(crate) fn end(&self, task_id: &str) -> Option<Intent> {
        self.active
            .lock()
            .ok()
            .and_then(|mut active| active.remove(task_id))
            .and_then(|(_, intent)| intent)
    }

    pub(crate) fn intent(&self, task_id: &str) -> Option<Intent> {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.get(task_id).and_then(|(_, intent)| *intent))
    }

    /// Signals an executing task. Returns false when the task is not
    /// executing, so the caller changes its stored state directly.
    pub(crate) fn signal(&self, task_id: &str, intent: Intent) -> bool {
        let Ok(mut active) = self.active.lock() else {
            return false;
        };
        match active.get_mut(task_id) {
            Some((cancellation, slot)) => {
                // A cancel outranks a pause that is already on its way.
                if *slot != Some(Intent::Cancel) {
                    *slot = Some(intent);
                }
                cancellation.cancel();
                true
            }
            None => false,
        }
    }

    pub(crate) fn active_ids(&self) -> Vec<String> {
        self.active
            .lock()
            .map(|active| active.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn set_paths_for_tests(&self, database: PathBuf, data: PathBuf) {
        let _ = self.paths.set(Paths { database, data });
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod runtime_lock_tests {
    use super::AgentState;

    #[test]
    fn route_configuration_gate_serializes_task_start_with_snapshot_write() {
        use std::sync::{Arc, mpsc};
        use std::time::Duration;

        let state = Arc::new(AgentState::default());
        let write_guard = state.lock_route_configuration();
        let (started_tx, started_rx) = mpsc::channel();
        let (active_tx, active_rx) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker = std::thread::spawn(move || {
            started_tx.send(()).expect("worker should announce startup");
            let _route_guard = worker_state.lock_route_configuration();
            active_tx
                .send(worker_state.begin("task").is_ok())
                .expect("worker should report task start");
        });

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker should be scheduled");
        assert!(active_rx.recv_timeout(Duration::from_millis(30)).is_err());
        assert!(state.active_ids().is_empty());

        drop(write_guard);
        assert!(
            active_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("worker should start after the snapshot write")
        );
        worker.join().expect("worker should finish");
        assert_eq!(state.end("task"), None);
    }

    #[test]
    fn a_second_agent_runtime_cannot_lock_the_same_data_directory() {
        let path = std::env::temp_dir().join(format!(
            "juniper-agent-runtime-lock-{}.lock",
            uuid::Uuid::new_v4()
        ));
        let first = AgentState::default();
        let second = AgentState::default();
        first.acquire_runtime_lock(&path).unwrap();
        let error = second.acquire_runtime_lock(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        drop(first);
        let next = AgentState::default();
        next.acquire_runtime_lock(&path).unwrap();
        drop(next);
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent<'a> {
    pub task_id: Option<&'a str>,
    pub kind: &'a str,
}

/// Tells the interface something changed; it re-reads what it shows. The
/// event carries no task content, and missing it loses nothing.
pub fn notify<R: Runtime>(app: &AppHandle<R>, task_id: Option<&str>, kind: &str) {
    let _ = app.emit(EVENT_TOPIC, AgentEvent { task_id, kind });
}

/// Called once from setup: records paths, recovers state left by an earlier
/// run, restores the global stop, and starts the worker.
pub fn start<R: Runtime>(app: AppHandle<R>, state: &AgentState, database: PathBuf, data: PathBuf) {
    mcp::set_plugin_root(&data);
    if state.paths.set(Paths { database, data }).is_err() {
        return;
    }
    match state.connection() {
        Ok(connection) => {
            if let Err(error) = store::recover_after_restart(&connection) {
                eprintln!("juniper: agent recovery failed: {error}");
            }
            if let Err(error) = skills::install_bundled(&connection) {
                eprintln!("juniper: bundled skills unavailable: {error}");
            }
            let stopped = store::agent_settings(&connection)
                .map(|settings| settings["stopped"].as_bool().unwrap_or(false))
                .unwrap_or(false);
            state.stopped.store(stopped, Ordering::Release);
        }
        Err(error) => eprintln!("juniper: agent store unavailable: {error}"),
    }
    tauri::async_runtime::spawn(runner::worker(app));
}
