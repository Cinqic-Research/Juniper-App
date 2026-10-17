//! Native record of model-license acceptance (review F-02 of 406a0be).
//!
//! A catalog entry with `licenseAcceptance: "required"` downloads or imports
//! only after the user accepted its license in Juniper. The decision lives in
//! the host, not the webview: acceptance is kept in `model-licenses.json` in
//! the app data directory, keyed by catalog id and bound to the license name,
//! URL and pinned terms revision, so a changed revision asks again. Terms
//! recorded only as a `-sha256-` response fingerprint are mutable pages the
//! catalog cannot pin; acceptance for those is never saved and covers only the
//! next download or import in this run of Juniper.

use crate::catalog::{self, CatalogEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, Runtime};

const FILE_NAME: &str = "model-licenses.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Acceptance {
    license: String,
    license_url: String,
    terms_revision: String,
    accepted_at: String,
}

impl Acceptance {
    fn matches(&self, entry: &CatalogEntry) -> bool {
        self.license == entry.license
            && self.license_url == entry.license_url
            && Some(self.terms_revision.as_str()) == entry.license_terms_revision.as_deref()
    }
}

/// Acceptance for fingerprinted terms: one transfer, this process only.
fn one_time() -> &'static Mutex<HashSet<String>> {
    static ONE_TIME: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ONE_TIME.get_or_init(|| Mutex::new(HashSet::new()))
}

fn one_time_key(directory: &Path, entry: &CatalogEntry) -> String {
    format!(
        "{}\n{}\n{}",
        directory.display(),
        entry.id,
        entry.license_terms_revision.as_deref().unwrap_or_default()
    )
}

pub fn required(entry: &CatalogEntry) -> bool {
    entry.license_acceptance.as_deref() == Some("required")
}

/// Whether acceptance may be saved: only for a pinned terms revision.
fn persistable(entry: &CatalogEntry) -> bool {
    entry
        .license_terms_revision
        .as_deref()
        .is_some_and(|revision| !revision.is_empty() && !revision.contains("-sha256-"))
}

fn read(directory: &Path) -> Result<BTreeMap<String, Acceptance>, String> {
    match fs::read(directory.join(FILE_NAME)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            "MODEL_LICENSE_STORE_ERROR: The saved license choices could not be read.".to_owned()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(format!("MODEL_LICENSE_STORE_ERROR: {error}")),
    }
}

fn write(directory: &Path, records: &BTreeMap<String, Acceptance>) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| format!("MODEL_LICENSE_STORE_ERROR: {error}"))?;
    let bytes = serde_json::to_vec_pretty(records)
        .map_err(|error| format!("MODEL_LICENSE_STORE_ERROR: {error}"))?;
    let staging = directory.join(format!("{FILE_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()));
    fs::write(&staging, bytes)
        .and_then(|_| fs::File::open(&staging)?.sync_all())
        .and_then(|_| fs::rename(&staging, directory.join(FILE_NAME)))
        .map_err(|error| {
            let _ = fs::remove_file(&staging);
            format!("MODEL_LICENSE_STORE_ERROR: {error}")
        })
}

/// Whether a saved acceptance covers this entry's current terms. Entries that
/// need no acceptance report `true`.
pub fn accepted_in(directory: &Path, entry: &CatalogEntry) -> Result<bool, String> {
    if !required(entry) {
        return Ok(true);
    }
    if !persistable(entry) {
        return Ok(false);
    }
    Ok(read(directory)?
        .get(&entry.id)
        .is_some_and(|record| record.matches(entry)))
}

/// Records the user's acceptance: saved for a pinned revision, or held for
/// the next transfer only for fingerprinted terms.
pub fn accept_in(directory: &Path, entry: &CatalogEntry) -> Result<(), String> {
    if !required(entry) {
        return Ok(());
    }
    if !persistable(entry) {
        one_time()
            .lock()
            .map_err(|_| "MODEL_LICENSE_STORE_ERROR: License state unavailable.")?
            .insert(one_time_key(directory, entry));
        return Ok(());
    }
    let mut records = read(directory)?;
    records.insert(
        entry.id.clone(),
        Acceptance {
            license: entry.license.clone(),
            license_url: entry.license_url.clone(),
            terms_revision: entry.license_terms_revision.clone().unwrap_or_default(),
            accepted_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        },
    );
    write(directory, &records)
}

/// Refuses a download or import of an entry whose license the user has not
/// accepted. A one-time acceptance is used up by the call that checks it.
pub fn require_in(directory: &Path, entry: &CatalogEntry) -> Result<(), String> {
    if accepted_in(directory, entry)? {
        return Ok(());
    }
    if !persistable(entry)
        && one_time()
            .lock()
            .map_err(|_| "MODEL_LICENSE_STORE_ERROR: License state unavailable.")?
            .remove(&one_time_key(directory, entry))
    {
        return Ok(());
    }
    Err(format!(
        "MODEL_LICENSE_NOT_ACCEPTED: {} is distributed under the {}. Accept its license in Juniper before downloading or importing it.",
        entry.display_name, entry.license
    ))
}

#[cfg(test)]
thread_local! {
    /// Tests point the host store at a temporary folder: a mock app's data
    /// directory would be a real folder in the user's profile (review F-16).
    static TEST_DIRECTORY: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(directory) = TEST_DIRECTORY.with(|directory| directory.borrow().clone()) {
        return Ok(directory);
    }
    app.path()
        .app_data_dir()
        .map_err(|error| format!("MODEL_LICENSE_STORE_ERROR: {error}"))
}

pub fn require<R: Runtime>(app: &AppHandle<R>, entry: &CatalogEntry) -> Result<(), String> {
    if !required(entry) {
        return Ok(());
    }
    require_in(&directory(app)?, entry)
}

pub fn accepted<R: Runtime>(app: &AppHandle<R>, catalog_id: &str) -> Result<bool, String> {
    accepted_in(&directory(app)?, &catalog::find(catalog_id)?)
}

pub fn accept<R: Runtime>(app: &AppHandle<R>, catalog_id: &str) -> Result<(), String> {
    accept_in(&directory(app)?, &catalog::find(catalog_id)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!("juniper-licenses-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn entry_with(id: &str, revision: &str) -> CatalogEntry {
        let mut entry = catalog::entries()
            .unwrap()
            .into_iter()
            .find(required)
            .expect("the catalog has an entry that needs acceptance");
        entry.id = id.into();
        entry.license_terms_revision = Some(revision.into());
        entry
    }

    #[test]
    fn a_required_license_is_refused_until_accepted_and_bound_to_its_revision() {
        let directory = folder();
        let entry = entry_with("gated", "rev-1");
        let refused = require_in(&directory, &entry).unwrap_err();
        assert!(
            refused.starts_with("MODEL_LICENSE_NOT_ACCEPTED"),
            "{refused}"
        );
        accept_in(&directory, &entry).unwrap();
        require_in(&directory, &entry).unwrap();
        // Persisted in the host store: a fresh read (as after a restart, or
        // with the webview's storage cleared) still finds it.
        assert!(accepted_in(&directory, &entry).unwrap());
        assert!(directory.join(FILE_NAME).is_file());
        // A new terms revision asks again.
        let revised = entry_with("gated", "rev-2");
        assert!(require_in(&directory, &revised).is_err());
        // Acceptance for one model does not cover another.
        assert!(require_in(&directory, &entry_with("other", "rev-1")).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn fingerprinted_terms_are_never_saved_and_cover_one_transfer() {
        let directory = folder();
        let entry = entry_with("gemma-like", "terms-sha256-abc");
        accept_in(&directory, &entry).unwrap();
        assert!(!directory.join(FILE_NAME).exists(), "nothing is saved");
        assert!(!accepted_in(&directory, &entry).unwrap());
        require_in(&directory, &entry).unwrap();
        let again = require_in(&directory, &entry).unwrap_err();
        assert!(again.starts_with("MODEL_LICENSE_NOT_ACCEPTED"), "{again}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_catalog_entry_that_needs_acceptance_is_refused_on_a_fresh_store() {
        let directory = folder();
        let gated: Vec<CatalogEntry> = catalog::entries()
            .unwrap()
            .into_iter()
            .filter(required)
            .collect();
        // Llama 3.x and Gemma 3 are held from the 0.4.0-alpha.1 catalog (G7);
        // Nemotron 3 Nano 4B still needs acceptance.
        assert!(!gated.is_empty(), "the Nemotron entry needs acceptance");
        for entry in &gated {
            assert!(require_in(&directory, entry).is_err(), "{}", entry.id);
        }
        let open = catalog::entries()
            .unwrap()
            .into_iter()
            .find(|entry| !required(entry))
            .unwrap();
        require_in(&directory, &open).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    /// The native download and import entry points refuse before any
    /// transfer, whatever the renderer did (no dialog, cleared storage).
    #[test]
    fn native_download_and_import_refuse_a_gated_model_without_acceptance() {
        use crate::commands::{AppState, Cancellation};
        let app = tauri::test::mock_app();
        let state = AppState::default();
        let temporary = folder();
        TEST_DIRECTORY.with(|directory| *directory.borrow_mut() = Some(temporary.clone()));
        let gated = catalog::entries()
            .unwrap()
            .into_iter()
            .find(|entry| required(entry) && persistable(entry))
            .unwrap();
        // The temporary host store has no saved acceptance for it.
        assert!(!accepted(app.handle(), &gated.id).unwrap());
        let download = tauri::async_runtime::block_on(crate::managed_models::download(
            app.handle().clone(),
            &state,
            &gated.id,
            None,
            "license-test",
            Cancellation::default(),
        ))
        .unwrap_err();
        assert!(
            download.starts_with("MODEL_LICENSE_NOT_ACCEPTED"),
            "{download}"
        );
        let import = tauri::async_runtime::block_on(crate::managed_models::import(
            app.handle().clone(),
            &state,
            &gated.id,
            None,
            Path::new("/nonexistent/model.gguf"),
            "license-test",
            Cancellation::default(),
        ))
        .unwrap_err();
        assert!(import.starts_with("MODEL_LICENSE_NOT_ACCEPTED"), "{import}");
        // Acceptance recorded through the same host path is found there.
        accept(app.handle(), &gated.id).unwrap();
        assert!(temporary.join(FILE_NAME).is_file());
        require(app.handle(), &gated).unwrap();
        TEST_DIRECTORY.with(|directory| *directory.borrow_mut() = None);
        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn a_damaged_store_refuses_rather_than_allows() {
        let directory = folder();
        let entry = entry_with("gated", "rev-1");
        fs::write(directory.join(FILE_NAME), b"{not json").unwrap();
        let error = require_in(&directory, &entry).unwrap_err();
        assert!(error.starts_with("MODEL_LICENSE_STORE_ERROR"), "{error}");
        fs::remove_dir_all(directory).unwrap();
    }
}
