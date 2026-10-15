//! File access inside folders the user granted to a project. Every operation
//! goes through a capability handle on the granted folder (cap-std), so a
//! path cannot resolve outside it, and every path component is also checked
//! so symbolic links and reparse points are refused rather than followed.

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_WRITE_BYTES: usize = 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 500;
const MAX_SEARCH_FILES: usize = 2_000;
const MAX_SEARCH_ENTRIES: usize = 20_000;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SEARCH_MATCHES: usize = 50;
const SKIPPED_FOLDERS: &[&str] = &[".git", "node_modules", "target", ".venv", "__pycache__"];
pub const ABSENT: &str = "absent";
const MAX_DIFF_CHARS: usize = 200_000;
/// Folders and files whose contents other programs execute (version
/// control hooks and config, editor tasks, environment loaders, CI). An
/// agent cannot write them, because that would run code outside the sandbox.
const PROTECTED: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".vscode",
    ".idea",
    ".husky",
    ".github",
    ".gitlab",
    ".circleci",
    ".devcontainer",
    ".direnv",
    ".envrc",
    ".gitmodules",
    ".gitattributes",
    ".gitlab-ci.yml",
    ".pre-commit-config.yaml",
];

fn protected(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        PROTECTED.contains(&name.as_str())
    })
}

pub type WorkspaceResult<T> = Result<T, (String, String)>;

fn error(code: &str, message: impl Into<String>) -> (String, String) {
    (code.to_owned(), message.into())
}

/// Opens the granted folder itself. The folder must still be a real
/// directory, not a link to one.
pub fn open_root(path: &str) -> WorkspaceResult<Dir> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        error(
            "ROOT_UNAVAILABLE",
            "The granted folder is no longer available.",
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(error(
            "ROOT_UNAVAILABLE",
            "The granted folder is no longer a regular folder.",
        ));
    }
    Dir::open_ambient_dir(path, ambient_authority()).map_err(|_| {
        error(
            "ROOT_UNAVAILABLE",
            "The granted folder could not be opened.",
        )
    })
}

/// Validates a model-supplied relative path: no absolute paths, drive
/// prefixes, parent references, or empty and control-character names.
pub fn relative_path(value: &str) -> WorkspaceResult<PathBuf> {
    let trimmed = value.trim().trim_start_matches("./");
    if trimmed.len() > 1024 || trimmed.contains('\0') || trimmed.contains('\\') {
        return Err(error("PATH_REJECTED", "That path is not allowed."));
    }
    let path = Path::new(trimmed);
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.chars().any(char::is_control) || text.ends_with(' ') || text.ends_with('.')
                {
                    return Err(error("PATH_REJECTED", "That path is not allowed."));
                }
                clean.push(part);
            }
            Component::CurDir => {}
            _ => {
                return Err(error(
                    "PATH_REJECTED",
                    "Paths must stay inside the granted folder.",
                ));
            }
        }
    }
    Ok(clean)
}

/// Refuses a path if it, or any folder on the way to it, is a link.
fn check_components(dir: &Dir, path: &Path) -> WorkspaceResult<()> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        match dir.symlink_metadata(&prefix) {
            Ok(metadata) if metadata.is_symlink() => {
                return Err(error(
                    "LINK_REJECTED",
                    "Symbolic links and reparse points are not followed.",
                ));
            }
            Ok(_) => {}
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(error("PATH_UNAVAILABLE", "That path is not available.")),
        }
    }
    Ok(())
}

pub fn version_of(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn list(dir: &Dir, path: &str) -> WorkspaceResult<Value> {
    let path = relative_path(path)?;
    check_components(dir, &path)?;
    let target = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path.as_path()
    };
    let entries = dir
        .read_dir(target)
        .map_err(|_| error("PATH_UNAVAILABLE", "That folder is not available."))?;
    let mut items = Vec::new();
    let mut truncated = false;
    for entry in entries.flatten() {
        if items.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let entry_type = if kind.is_symlink() {
            "link"
        } else if kind.is_dir() {
            "folder"
        } else if kind.is_file() {
            "file"
        } else {
            "other"
        };
        let size = if kind.is_file() {
            entry.metadata().map(|metadata| metadata.len()).ok()
        } else {
            None
        };
        items.push(json!({ "name": name, "type": entry_type, "sizeBytes": size }));
    }
    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({ "path": path.to_string_lossy(), "entries": items, "truncated": truncated }))
}

/// Reads a whole regular file within the size limit.
pub fn read_bytes(dir: &Dir, path: &Path) -> WorkspaceResult<Vec<u8>> {
    check_components(dir, path)?;
    let metadata = dir
        .symlink_metadata(path)
        .map_err(|_| error("FILE_NOT_FOUND", "That file does not exist."))?;
    if !metadata.is_file() {
        return Err(error("NOT_A_FILE", "That path is not a regular file."));
    }
    if metadata.len() > MAX_READ_BYTES {
        return Err(error("FILE_TOO_LARGE", "That file is larger than 4 MB."));
    }
    let mut file = dir
        .open(path)
        .map_err(|_| error("FILE_NOT_FOUND", "That file could not be opened."))?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("READ_FAILED", "That file could not be read."))?;
    if bytes.len() as u64 > MAX_READ_BYTES {
        return Err(error("FILE_TOO_LARGE", "That file is larger than 4 MB."));
    }
    Ok(bytes)
}

pub fn read(dir: &Dir, path: &str, offset: usize, limit: usize) -> WorkspaceResult<Value> {
    let relative = relative_path(path)?;
    let bytes = read_bytes(dir, &relative)?;
    let version = version_of(&bytes);
    let text =
        String::from_utf8(bytes).map_err(|_| error("NOT_TEXT", "That file is not UTF-8 text."))?;
    let characters = text.chars().count();
    let content: String = text.chars().skip(offset).take(limit).collect();
    let end = offset + content.chars().count();
    Ok(json!({
        "path": relative.to_string_lossy(),
        "version": version,
        "content": content,
        "offset": offset,
        "totalCharacters": characters,
        "truncated": end < characters
    }))
}

pub fn search(dir: &Dir, query: &str, path: &str) -> WorkspaceResult<Value> {
    let start = relative_path(path)?;
    check_components(dir, &start)?;
    let needle = query.to_lowercase();
    if needle.trim().is_empty() {
        return Err(error("INVALID_QUERY", "Enter text to search for."));
    }
    let mut matches = Vec::new();
    let mut files = 0usize;
    let mut seen = 0usize;
    let mut stack = vec![start];
    'walk: while let Some(folder) = stack.pop() {
        let target = if folder.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            folder.clone()
        };
        let Ok(entries) = dir.read_dir(&target) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_SEARCH_ENTRIES || files > MAX_SEARCH_FILES {
                break 'walk;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let child = folder.join(&name);
            if kind.is_dir() {
                if !SKIPPED_FOLDERS.contains(&name.to_string_lossy().as_ref()) {
                    stack.push(child);
                }
                continue;
            }
            if !kind.is_file()
                || entry
                    .metadata()
                    .map(|metadata| metadata.len() > MAX_SEARCH_FILE_BYTES)
                    .unwrap_or(true)
            {
                continue;
            }
            files += 1;
            let Ok(mut file) = dir.open(&child) else {
                continue;
            };
            let mut text = String::new();
            if file.read_to_string(&mut text).is_err() {
                continue;
            }
            for (number, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    matches.push(json!({
                        "path": child.to_string_lossy(),
                        "line": number + 1,
                        "text": line.chars().take(240).collect::<String>()
                    }));
                    if matches.len() >= MAX_SEARCH_MATCHES {
                        break 'walk;
                    }
                }
            }
        }
    }
    Ok(
        json!({ "matches": matches, "filesSearched": files, "complete": matches.len() < MAX_SEARCH_MATCHES && seen <= MAX_SEARCH_ENTRIES && files <= MAX_SEARCH_FILES }),
    )
}

const INDEXED_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "rst", "csv", "json", "yaml", "yml", "toml", "py", "rs", "ts", "tsx",
    "js", "jsx", "html", "css", "c", "h", "cpp", "java", "go", "sh", "sql",
];

/// Text files under a granted folder for indexing, bounded in count and
/// size; links and generated folders are skipped.
pub fn text_files(dir: &Dir) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut seen = 0usize;
    let mut stack = vec![PathBuf::new()];
    while let Some(folder) = stack.pop() {
        let target = if folder.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            folder.clone()
        };
        let Ok(entries) = dir.read_dir(&target) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_SEARCH_ENTRIES || found.len() >= MAX_SEARCH_FILES {
                return found;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let child = folder.join(&name);
            if kind.is_dir() && !kind.is_symlink() {
                if !SKIPPED_FOLDERS.contains(&name.to_string_lossy().as_ref()) {
                    stack.push(child);
                }
            } else if kind.is_file()
                && child
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        INDEXED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
                    })
                && entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() <= MAX_SEARCH_FILE_BYTES)
            {
                found.push(child);
            }
        }
    }
    found.sort();
    found
}

/// The current version of a path: its content digest, or `absent`.
pub fn current_version(dir: &Dir, path: &Path) -> WorkspaceResult<String> {
    check_components(dir, path)?;
    match dir.symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(version_of(&read_bytes(dir, path)?)),
        Ok(_) => Err(error("NOT_A_FILE", "That path is not a regular file.")),
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => Ok(ABSENT.into()),
        Err(_) => Err(error("PATH_UNAVAILABLE", "That path is not available.")),
    }
}

/// The preview shown before a write: a unified diff against the file as it
/// is now. Refuses when the model's expected version is out of date.
pub fn preview_write(
    dir: &Dir,
    path: &str,
    content: &str,
    expected_version: &str,
) -> WorkspaceResult<(String, String)> {
    let relative = relative_path(path)?;
    if relative.as_os_str().is_empty() {
        return Err(error("PATH_REJECTED", "Name a file to write."));
    }
    if protected(&relative) {
        return Err(error(
            "PROTECTED_PATH",
            "Agents cannot change version-control, editor, or CI configuration, which other programs run outside the sandbox.",
        ));
    }
    if content.len() > MAX_WRITE_BYTES {
        return Err(error("CONTENT_TOO_LARGE", "Writes are limited to 1 MB."));
    }
    let current = current_version(dir, &relative)?;
    if current != expected_version {
        return Err(error(
            "FILE_CHANGED",
            format!(
                "The file is at version {current}, not {expected_version}. Read it again before editing."
            ),
        ));
    }
    let before = if current == ABSENT {
        String::new()
    } else {
        String::from_utf8(read_bytes(dir, &relative)?)
            .map_err(|_| error("NOT_TEXT", "Only text files can be edited."))?
    };
    let name = relative.to_string_lossy();
    let diff = similar::TextDiff::from_lines(before.as_str(), content)
        .unified_diff()
        .context_radius(3)
        .header(
            &if current == ABSENT {
                "/dev/null".to_owned()
            } else {
                format!("a/{name}")
            },
            &format!("b/{name}"),
        )
        .to_string();
    // An approval covers exactly what it shows, so a diff is never
    // shortened; one too long to review is refused instead.
    if diff.chars().count() > MAX_DIFF_CHARS {
        return Err(error(
            "CHANGE_TOO_LARGE_TO_REVIEW",
            "This change is too large to show for approval. Write the file in smaller parts.",
        ));
    }
    let preview = diff;
    Ok((current, preview))
}

/// Writes atomically: a new file beside the target, synced, then renamed
/// over it, and only if the target is still at `expected_version`.
pub fn commit_write(
    dir: &Dir,
    path: &str,
    content: &str,
    expected_version: &str,
) -> WorkspaceResult<Value> {
    let relative = relative_path(path)?;
    if relative.as_os_str().is_empty() || content.len() > MAX_WRITE_BYTES || protected(&relative) {
        return Err(error("PATH_REJECTED", "That write is not allowed."));
    }
    if let Some(parent) = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        check_components(dir, parent)?;
        dir.create_dir_all(parent)
            .map_err(|_| error("WRITE_FAILED", "The folder could not be created."))?;
        check_components(dir, parent)?;
    }
    let current = current_version(dir, &relative)?;
    if current != expected_version {
        return Err(error(
            "FILE_CHANGED",
            "The file changed after it was previewed; nothing was written.",
        ));
    }
    let file_name = relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staging = relative.with_file_name(format!(
        ".{file_name}.juniper-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = dir
        .open_with(&staging, &options)
        .map_err(|_| error("WRITE_FAILED", "The file could not be written."))?;
    let written = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    if written.is_err() {
        let _ = dir.remove_file(&staging);
        return Err(error("WRITE_FAILED", "The file could not be written."));
    }
    // A last check right before the rename narrows the window in which an
    // outside edit could be lost.
    if current_version(dir, &relative)? != expected_version {
        let _ = dir.remove_file(&staging);
        return Err(error(
            "FILE_CHANGED",
            "The file changed while it was being written; nothing was replaced.",
        ));
    }
    dir.rename(&staging, dir, &relative).map_err(|_| {
        let _ = dir.remove_file(&staging);
        error("WRITE_FAILED", "The file could not be replaced.")
    })?;
    Ok(json!({
        "path": relative.to_string_lossy(),
        "previousVersion": expected_version,
        "version": version_of(content.as_bytes()),
        "bytes": content.len()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (PathBuf, Dir) {
        let path = std::env::temp_dir().join(format!("juniper-workspace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(path.join("docs")).expect("root");
        std::fs::write(path.join("docs/notes.md"), "alpha\nbeta\n").expect("file");
        let dir = open_root(path.to_str().expect("utf-8")).expect("open");
        (path, dir)
    }

    #[test]
    fn traversal_absolute_and_drive_paths_are_rejected() {
        for bad in [
            "../outside",
            "/etc/passwd",
            "docs/../../x",
            "a\\b",
            "C:/x",
            "docs/\u{7}",
        ] {
            let result = relative_path(bad);
            // `C:/x` is a normal component on Unix, which then stays inside the root.
            if bad == "C:/x" && cfg!(unix) {
                continue;
            }
            assert!(result.is_err(), "{bad} should be rejected");
        }
        assert_eq!(
            relative_path("./docs/notes.md").unwrap(),
            PathBuf::from("docs/notes.md")
        );
    }

    #[test]
    fn reads_lists_and_searches_inside_the_root() {
        let (path, dir) = root();
        let listing = list(&dir, "docs").expect("list");
        assert_eq!(listing["entries"][0]["name"], "notes.md");
        let read = read(&dir, "docs/notes.md", 0, 100).expect("read");
        assert_eq!(read["content"], "alpha\nbeta\n");
        assert!(read["version"].as_str().unwrap().starts_with("sha256:"));
        let found = search(&dir, "BETA", "").expect("search");
        assert_eq!(found["matches"][0]["line"], 2);
        std::fs::remove_dir_all(path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_or_out_of_the_root_are_not_followed() {
        let (path, dir) = root();
        let outside =
            std::env::temp_dir().join(format!("juniper-outside-{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "secret").expect("outside");
        std::os::unix::fs::symlink(&outside, path.join("escape.txt")).expect("link");
        std::os::unix::fs::symlink(path.join("docs"), path.join("linked-docs")).expect("dir link");
        assert_eq!(
            read(&dir, "escape.txt", 0, 10).unwrap_err().0,
            "LINK_REJECTED"
        );
        assert_eq!(
            read(&dir, "linked-docs/notes.md", 0, 10).unwrap_err().0,
            "LINK_REJECTED"
        );
        assert_eq!(
            preview_write(&dir, "linked-docs/new.md", "x", ABSENT)
                .unwrap_err()
                .0,
            "LINK_REJECTED"
        );
        let found = search(&dir, "secret", "").expect("search");
        assert!(found["matches"].as_array().unwrap().is_empty());
        std::fs::remove_file(outside).ok();
        std::fs::remove_dir_all(path).ok();
    }

    #[test]
    fn configuration_that_runs_code_outside_the_sandbox_cannot_be_written() {
        let (path, dir) = root();
        for target in [
            ".git/config",
            ".vscode/tasks.json",
            ".envrc",
            "src/.husky/pre-commit",
            ".GitHub/workflows/x.yml",
        ] {
            assert_eq!(
                preview_write(&dir, target, "x", ABSENT).unwrap_err().0,
                "PROTECTED_PATH",
                "{target}"
            );
            assert!(commit_write(&dir, target, "x", ABSENT).is_err(), "{target}");
        }
        assert!(preview_write(&dir, "docs/git-notes.md", "x", ABSENT).is_ok());
        std::fs::remove_dir_all(path).ok();
    }

    #[test]
    fn writes_are_previewed_version_bound_and_atomic() {
        let (path, dir) = root();
        let version = read(&dir, "docs/notes.md", 0, 100).unwrap()["version"]
            .as_str()
            .unwrap()
            .to_owned();
        let (_, diff) =
            preview_write(&dir, "docs/notes.md", "alpha\ngamma\n", &version).expect("preview");
        assert!(diff.contains("-beta") && diff.contains("+gamma"));
        assert_eq!(
            preview_write(&dir, "docs/notes.md", "x", "sha256:stale")
                .unwrap_err()
                .0,
            "FILE_CHANGED"
        );
        // Someone edits the file after the preview: the write is refused.
        std::fs::write(path.join("docs/notes.md"), "edited elsewhere\n").expect("edit");
        assert_eq!(
            commit_write(&dir, "docs/notes.md", "alpha\ngamma\n", &version)
                .unwrap_err()
                .0,
            "FILE_CHANGED"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("docs/notes.md")).unwrap(),
            "edited elsewhere\n"
        );
        let created = commit_write(&dir, "out/report.md", "# Report\n", ABSENT).expect("create");
        assert_eq!(created["previousVersion"], ABSENT);
        assert_eq!(
            std::fs::read_to_string(path.join("out/report.md")).unwrap(),
            "# Report\n"
        );
        let leftovers = std::fs::read_dir(path.join("out")).unwrap().count();
        assert_eq!(leftovers, 1, "no staging file remains");
        std::fs::remove_dir_all(path).ok();
    }
}
