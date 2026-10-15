//! Local full-text retrieval over a project's granted folders (SQLite FTS5).
//! Every passage keeps its folder, path, content version, and character
//! offsets. Changed files are re-indexed; removed files and revoked folders
//! disappear from the index, and so from future task context.

use super::store::{self, db_error};
use super::types::Project;
use super::workspace;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

const CHUNK_CHARS: usize = 1_200;
const MAX_HITS: usize = 8;

pub struct Hit {
    pub path: String,
    pub version: String,
    pub start: i64,
    pub end: i64,
    pub text: String,
}

/// Splits text into passages of about `CHUNK_CHARS`, preferring paragraph
/// breaks, with character offsets into the original.
pub fn chunks(text: &str) -> Vec<(usize, usize, String)> {
    let characters: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut start = 0;
    while start < characters.len() {
        let mut end = (start + CHUNK_CHARS).min(characters.len());
        if end < characters.len() {
            let window: String = characters[start..end].iter().collect();
            if let Some(position) = window
                .rfind("\n\n")
                .filter(|position| *position > CHUNK_CHARS / 3)
            {
                end = start + window[..position].chars().count();
            }
        }
        let passage: String = characters[start..end].iter().collect();
        if !passage.trim().is_empty() {
            found.push((start, end, passage));
        }
        start = end.max(start + 1);
    }
    found
}

pub fn index_project(connection: &Connection, project: &Project) -> Result<Value, String> {
    let mut indexed = 0;
    let mut unchanged = 0;
    let mut removed = 0;
    let mut skipped = 0;
    for root in &project.roots {
        let dir = workspace::open_root(&root.path).map_err(|(_, message)| message)?;
        let files = workspace::text_files(&dir);
        let mut present = std::collections::HashSet::new();
        for path in files.into_iter() {
            let key = path.to_string_lossy().into_owned();
            let Ok(bytes) = workspace::read_bytes(&dir, &path) else {
                skipped += 1;
                continue;
            };
            let Ok(text) = String::from_utf8(bytes.clone()) else {
                skipped += 1;
                continue;
            };
            present.insert(key.clone());
            let version = workspace::version_of(&bytes);
            let current: Option<String> = connection
                .query_row(
                    "SELECT version FROM doc_files WHERE root_id = ?1 AND path = ?2",
                    params![root.id, key],
                    |row| row.get(0),
                )
                .ok();
            if current.as_deref() == Some(version.as_str()) {
                unchanged += 1;
                continue;
            }
            let transaction = connection.unchecked_transaction().map_err(db_error)?;
            transaction
                .execute(
                    "DELETE FROM doc_chunks WHERE root_id = ?1 AND path = ?2",
                    params![root.id, key],
                )
                .map_err(db_error)?;
            for (start, end, passage) in chunks(&text) {
                transaction
                    .execute(
                        "INSERT INTO doc_chunks(text, project_id, root_id, path, version, start_offset, end_offset) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![passage, project.id, root.id, key, version, start as i64, end as i64],
                    )
                    .map_err(db_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO doc_files(root_id, path, version, size_bytes, modified_nanos, indexed_at) VALUES(?1, ?2, ?3, ?4, 0, ?5) ON CONFLICT(root_id, path) DO UPDATE SET version = excluded.version, size_bytes = excluded.size_bytes, indexed_at = excluded.indexed_at",
                    params![root.id, key, version, bytes.len() as i64, store::now()],
                )
                .map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
            indexed += 1;
        }
        let known: Vec<String> = {
            let mut statement = connection
                .prepare("SELECT path FROM doc_files WHERE root_id = ?1")
                .map_err(db_error)?;
            statement
                .query_map([&root.id], |row| row.get(0))
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        for path in known.into_iter().filter(|path| !present.contains(path)) {
            connection
                .execute(
                    "DELETE FROM doc_chunks WHERE root_id = ?1 AND path = ?2",
                    params![root.id, path],
                )
                .map_err(db_error)?;
            connection
                .execute(
                    "DELETE FROM doc_files WHERE root_id = ?1 AND path = ?2",
                    params![root.id, path],
                )
                .map_err(db_error)?;
            removed += 1;
        }
    }
    Ok(
        json!({ "indexed": indexed, "unchanged": unchanged, "removed": removed, "skipped": skipped }),
    )
}

/// Builds an FTS5 query from plain words; nothing the model writes is
/// passed through as query syntax.
fn fts_query(query: &str, any: bool) -> Option<String> {
    let words: Vec<String> = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() > 1)
        .take(12)
        .map(|word| format!("\"{}\"", word.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(if any { " OR " } else { " " }))
}

pub fn search(connection: &Connection, project: &Project, query: &str) -> Result<Vec<Hit>, String> {
    let live_roots: Vec<String> = project.roots.iter().map(|root| root.id.clone()).collect();
    for any in [false, true] {
        let Some(expression) = fts_query(query, any) else {
            return Ok(Vec::new());
        };
        let mut statement = connection
            .prepare("SELECT path, version, start_offset, end_offset, text, root_id FROM doc_chunks WHERE doc_chunks MATCH ?1 AND project_id = ?2 ORDER BY bm25(doc_chunks) LIMIT 32")
            .map_err(db_error)?;
        let hits: Vec<Hit> = statement
            .query_map(params![expression, project.id], |row| {
                Ok((
                    Hit {
                        path: row.get(0)?,
                        version: row.get(1)?,
                        start: row.get(2)?,
                        end: row.get(3)?,
                        text: row.get::<_, String>(4)?.chars().take(900).collect(),
                    },
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(db_error)?
            .filter_map(Result::ok)
            .filter(|(_, root)| live_roots.contains(root))
            .map(|(hit, _)| hit)
            .take(MAX_HITS)
            .collect();
        if !hits.is_empty() {
            return Ok(hits);
        }
    }
    Ok(Vec::new())
}

pub fn forget_root(connection: &Connection, root_id: &str) -> Result<(), String> {
    connection
        .execute("DELETE FROM doc_chunks WHERE root_id = ?1", [root_id])
        .map_err(db_error)?;
    connection
        .execute("DELETE FROM doc_files WHERE root_id = ?1", [root_id])
        .map_err(db_error)?;
    Ok(())
}

pub fn forget_project(connection: &Connection, project_id: &str) -> Result<(), String> {
    for root in store::roots(connection, project_id, true)? {
        forget_root(connection, &root.id)?;
    }
    connection
        .execute("DELETE FROM doc_chunks WHERE project_id = ?1", [project_id])
        .map_err(db_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::ProjectRoot;

    #[test]
    fn chunks_cover_the_text_with_offsets() {
        let text = format!("{}\n\n{}", "a".repeat(900), "b".repeat(900));
        let parts = chunks(&text);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].0, 0);
        assert_eq!(parts[1].1, text.chars().count());
    }

    #[test]
    fn model_text_is_never_fts_syntax() {
        assert_eq!(
            fts_query("NEAR(a b) OR x*", false).as_deref(),
            Some("\"near\" \"or\"")
        );
        assert_eq!(
            fts_query("budget report", true).as_deref(),
            Some("\"budget\" OR \"report\"")
        );
        assert!(fts_query("* ( )", false).is_none());
    }

    #[test]
    fn indexing_updates_versions_and_forgets_removed_files_and_revoked_roots() {
        let database =
            std::env::temp_dir().join(format!("juniper-index-{}.db", uuid::Uuid::new_v4()));
        crate::storage::initialize(&database).expect("db");
        let connection = store::open(&database).expect("open");
        let folder =
            std::env::temp_dir().join(format!("juniper-index-root-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("plan.md"), "The quarterly budget is approved.").expect("file");
        std::fs::write(folder.join("other.md"), "Unrelated notes about gardening.").expect("file");
        connection.execute_batch(&format!(
            "INSERT INTO projects(id, name, created_at, updated_at) VALUES('p1', 'P', 'x', 'x');
             INSERT INTO projects(id, name, created_at, updated_at) VALUES('p2', 'Q', 'x', 'x');
             INSERT INTO project_roots(id, project_id, path, label, access, created_at) VALUES('r1', 'p1', '{}', 'docs', 'read', 'x');",
            folder.to_string_lossy()
        )).expect("rows");
        let project = store::project(&connection, "p1").expect("project");
        let first = index_project(&connection, &project).expect("index");
        assert_eq!(first["indexed"], 2);
        let hits = search(&connection, &project, "budget approved").expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "plan.md");
        assert!(hits[0].version.starts_with("sha256:"));
        // Another project never sees these passages.
        let other = store::project(&connection, "p2").expect("project");
        assert!(
            search(&connection, &other, "budget")
                .expect("search")
                .is_empty()
        );
        // Unchanged files are skipped; a change re-indexes; a removal is forgotten.
        assert_eq!(
            index_project(&connection, &project).unwrap()["unchanged"],
            2
        );
        std::fs::write(folder.join("plan.md"), "The budget was rejected.").expect("edit");
        std::fs::remove_file(folder.join("other.md")).expect("remove");
        let second = index_project(&connection, &project).expect("index");
        assert_eq!(second["indexed"], 1);
        assert_eq!(second["removed"], 1);
        assert!(
            search(&connection, &project, "approved")
                .unwrap()
                .is_empty()
        );
        assert!(
            search(&connection, &project, "gardening")
                .unwrap()
                .is_empty()
        );
        forget_root(&connection, "r1").expect("forget");
        let stale = Project {
            roots: vec![ProjectRoot {
                id: "r1".into(),
                project_id: "p1".into(),
                label: "docs".into(),
                path: String::new(),
                access: "read".into(),
                created_at: String::new(),
                revoked_at: None,
            }],
            ..project
        };
        assert!(search(&connection, &stale, "rejected").unwrap().is_empty());
        std::fs::remove_dir_all(folder).ok();
        std::fs::remove_file(database).ok();
    }
}
