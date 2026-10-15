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
pub mod tools;
pub mod types;
pub mod web;
pub mod workspace;

use crate::commands::Cancellation;
use serde::Serialize;
use std::collections::HashMap;
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
    Cancel,
    /// The global stop: running work is interrupted and nothing new starts.
    Stop,
}

#[derive(Default)]
pub struct AgentState {
    paths: OnceLock<Paths>,
    pub wake: Notify,
    /// Tasks executing right now, with their cancellation handle and the
    /// reason one was raised.
    active: Mutex<HashMap<String, (Cancellation, Option<Intent>)>>,
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

    pub fn sandboxes(&self) -> PathBuf {
        self.data.join("agent").join("sandbox")
    }
}

impl AgentState {
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

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
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
