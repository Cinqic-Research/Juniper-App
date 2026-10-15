//! Skills: reusable procedures an agent can load on demand. Juniper reads a
//! documented subset of the `SKILL.md` format (YAML-style front matter with
//! `name`, `description`, and optional `license`, `version`, `author`,
//! `allowed-tools`, then a Markdown body). A skill is guidance only: it
//! cannot grant permissions, enable tools or plugins, run scripts, or change
//! the user's goal. Imported skills start disabled and are shown in full
//! before they can be enabled.

use super::store::{self, db_error};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_SKILL_BYTES: usize = 64 * 1024;
const MAX_RESOURCES: usize = 10;
const MAX_RESOURCE_BYTES: usize = 64 * 1024;

const BUNDLED: &[(&str, &str)] = &[
    (
        "cited-research-brief",
        include_str!("../../../config/skills/cited-research-brief/SKILL.md"),
    ),
    (
        "project-document-summary",
        include_str!("../../../config/skills/project-document-summary/SKILL.md"),
    ),
    (
        "draft-and-save",
        include_str!("../../../config/skills/draft-and-save/SKILL.md"),
    ),
    (
        "code-change-review",
        include_str!("../../../config/skills/code-change-review/SKILL.md"),
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub license: Option<String>,
    pub author: Option<String>,
    /// Tools the skill says it needs. Informational: the agent's own
    /// permissions decide what runs.
    pub requires: Vec<String>,
    pub body: String,
    pub resources: Vec<Resource>,
    pub source: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bundled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Resource {
    pub path: String,
    pub text: String,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// Parses a `SKILL.md` document.
pub fn parse(text: &str, source: &str) -> Result<Skill, String> {
    if text.len() > MAX_SKILL_BYTES {
        return Err("SKILL_INVALID: SKILL.md is larger than 64 KB.".into());
    }
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or("SKILL_INVALID: SKILL.md must start with front matter between --- lines.")?;
    let end = rest
        .find("\n---")
        .ok_or("SKILL_INVALID: The front matter is not closed with ---.")?;
    let front = &rest[..end];
    let body = rest[end + 4..]
        .trim_start_matches(['\r', '\n'])
        .trim_end()
        .to_owned();
    let mut fields = std::collections::BTreeMap::new();
    let mut requires = Vec::new();
    for line in front.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            requires.push(item.trim().trim_matches(['"', '\'']).to_owned());
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or("SKILL_INVALID: Front matter lines must be `key: value`.")?;
        let value = value.trim().trim_matches(['"', '\'']).to_owned();
        fields.insert(key.trim().to_owned(), value);
    }
    if let Some(inline) = fields
        .get("allowed-tools")
        .filter(|value| !value.is_empty())
    {
        requires.extend(
            inline
                .trim_matches(['[', ']'])
                .split([',', ' '])
                .map(|tool| tool.trim().trim_matches(['"', '\'']).to_owned())
                .filter(|tool| !tool.is_empty()),
        );
    }
    let name = fields.get("name").cloned().unwrap_or_default();
    if !valid_name(&name) {
        return Err(
            "SKILL_INVALID: `name` must be 1–64 lowercase letters, digits, or hyphens.".into(),
        );
    }
    let description = fields.get("description").cloned().unwrap_or_default();
    if description.is_empty() || description.len() > 1024 {
        return Err("SKILL_INVALID: `description` is required (up to 1,024 characters).".into());
    }
    if body.is_empty() {
        return Err("SKILL_INVALID: The skill has no procedure.".into());
    }
    let mut skill = Skill {
        id: name.clone(),
        name,
        description,
        version: fields.get("version").cloned().unwrap_or_else(|| "1".into()),
        license: fields.get("license").cloned(),
        author: fields.get("author").cloned(),
        requires,
        body,
        resources: Vec::new(),
        source: source.to_owned(),
        digest: String::new(),
        enabled: false,
        bundled: false,
    };
    skill.digest = digest(&skill);
    Ok(skill)
}

pub fn digest(skill: &Skill) -> String {
    let canonical = json!({
        "name": skill.name,
        "description": skill.description,
        "version": skill.version,
        "requires": skill.requires,
        "body": skill.body,
        "resources": skill.resources,
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(canonical.to_string().as_bytes())
    )
}

/// Reads a skill folder: `SKILL.md` plus Markdown or text references.
/// Links, nested archives, oversized files, and executable scripts are
/// refused rather than imported.
pub fn read_folder(folder: &Path) -> Result<Skill, String> {
    let dir =
        super::workspace::open_root(&folder.to_string_lossy()).map_err(|(_, message)| message)?;
    let text = super::workspace::read_bytes(&dir, Path::new("SKILL.md"))
        .map_err(|(_, message)| format!("SKILL_INVALID: {message}"))?;
    let text =
        String::from_utf8(text).map_err(|_| "SKILL_INVALID: SKILL.md is not UTF-8.".to_owned())?;
    let mut skill = parse(
        &text,
        &format!(
            "imported folder “{}”",
            folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        ),
    )?;
    for entry in dir
        .read_dir(".")
        .map_err(|_| "SKILL_INVALID: The folder could not be read.".to_owned())?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry
            .file_type()
            .map_err(|_| "SKILL_INVALID: The folder could not be read.".to_owned())?;
        if kind.is_symlink() {
            return Err(format!(
                "SKILL_INVALID: “{name}” is a link; links are not imported."
            ));
        }
        match (name.as_str(), kind.is_dir()) {
            ("SKILL.md" | "LICENSE" | "LICENSE.md" | "LICENSE.txt", false) => {}
            ("references", true) => {
                for item in dir
                    .read_dir("references")
                    .map_err(|_| "SKILL_INVALID: references/ could not be read.".to_owned())?
                    .flatten()
                {
                    let item_name = item.file_name().to_string_lossy().into_owned();
                    let item_kind = item.file_type().map_err(|_| "SKILL_INVALID".to_owned())?;
                    let allowed = item_kind.is_file()
                        && !item_kind.is_symlink()
                        && (item_name.ends_with(".md") || item_name.ends_with(".txt"));
                    if !allowed {
                        return Err(format!(
                            "SKILL_INVALID: references/{item_name} is not a Markdown or text file."
                        ));
                    }
                    let bytes = super::workspace::read_bytes(
                        &dir,
                        &Path::new("references").join(&item_name),
                    )
                    .map_err(|(_, message)| format!("SKILL_INVALID: {message}"))?;
                    if bytes.len() > MAX_RESOURCE_BYTES || skill.resources.len() >= MAX_RESOURCES {
                        return Err(
                            "SKILL_INVALID: Skill references are limited to 10 files of 64 KB."
                                .into(),
                        );
                    }
                    skill.resources.push(Resource {
                        path: format!("references/{item_name}"),
                        text: String::from_utf8(bytes)
                            .map_err(|_| "SKILL_INVALID: A reference is not UTF-8.".to_owned())?,
                    });
                }
            }
            ("scripts", _) => {
                return Err("SKILL_UNSUPPORTED: Skills with scripts are not supported in this beta; Juniper never runs skill code.".into());
            }
            _ => {
                return Err(format!(
                    "SKILL_INVALID: Unexpected item “{name}” in the skill folder."
                ));
            }
        }
    }
    skill.digest = digest(&skill);
    Ok(skill)
}

/// Installs or refreshes the bundled skills, keeping each one's enabled flag.
pub fn install_bundled(connection: &Connection) -> Result<(), String> {
    for (id, text) in BUNDLED {
        let mut skill = parse(text, "bundled with Juniper")?;
        debug_assert_eq!(&skill.id, id);
        skill.bundled = true;
        let enabled: Option<bool> = connection
            .query_row(
                "SELECT enabled FROM skills WHERE id = ?1",
                [&skill.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        save(connection, &skill, enabled.unwrap_or(true), true)?;
    }
    Ok(())
}

fn save(
    connection: &Connection,
    skill: &Skill,
    enabled: bool,
    bundled: bool,
) -> Result<(), String> {
    let payload = serde_json::to_string(skill).map_err(db_error)?;
    connection
        .execute(
            "INSERT INTO skills(id, name, payload, digest, enabled, bundled, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, payload = excluded.payload, digest = excluded.digest, enabled = excluded.enabled, bundled = excluded.bundled, updated_at = excluded.updated_at",
            params![skill.id, skill.name, payload, skill.digest, enabled, bundled, store::now()],
        )
        .map_err(db_error)?;
    Ok(())
}

pub fn list(connection: &Connection) -> Result<Vec<Skill>, String> {
    let mut statement = connection
        .prepare("SELECT payload, enabled, bundled, digest FROM skills ORDER BY bundled DESC, name")
        .map_err(db_error)?;
    statement
        .query_map([], |row| {
            let payload: String = row.get(0)?;
            Ok((
                payload,
                row.get::<_, bool>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(db_error)?
        .filter_map(Result::ok)
        .map(|(payload, enabled, bundled, digest)| {
            let mut skill: Skill = serde_json::from_str(&payload).map_err(db_error)?;
            skill.enabled = enabled;
            skill.bundled = bundled;
            skill.digest = digest;
            Ok(skill)
        })
        .collect()
}

pub fn get(connection: &Connection, id: &str) -> Result<Skill, String> {
    list(connection)?
        .into_iter()
        .find(|skill| skill.id == id)
        .ok_or_else(|| "SKILL_NOT_FOUND: That skill is not installed.".into())
}

/// The procedure text an agent loads, with its references appended.
pub fn body(connection: &Connection, id: &str) -> Result<String, String> {
    let skill = get(connection, id)?;
    if !skill.enabled {
        return Err("That skill is turned off.".into());
    }
    let mut text = skill.body.clone();
    for resource in &skill.resources {
        text.push_str(&format!(
            "\n\n## Reference: {}\n{}",
            resource.path, resource.text
        ));
    }
    Ok(text.chars().take(40_000).collect())
}

/// Imports a skill disabled; enabling it is a separate, informed step.
pub fn import(connection: &Connection, skill: Skill) -> Result<Skill, String> {
    if let Ok(existing) = get(connection, &skill.id)
        && existing.bundled
    {
        return Err("SKILL_CONFLICT: A bundled skill already uses that name.".into());
    }
    save(connection, &skill, false, false)?;
    get(connection, &skill.id)
}

pub fn set_enabled(connection: &Connection, id: &str, enabled: bool) -> Result<(), String> {
    let changed = connection
        .execute(
            "UPDATE skills SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            params![enabled, store::now(), id],
        )
        .map_err(db_error)?;
    if changed == 0 {
        return Err("SKILL_NOT_FOUND: That skill is not installed.".into());
    }
    Ok(())
}

pub fn remove(connection: &Connection, id: &str) -> Result<(), String> {
    let skill = get(connection, id)?;
    if skill.bundled {
        return Err("SKILL_BUNDLED: Bundled skills can be turned off but not removed.".into());
    }
    connection
        .execute("DELETE FROM skills WHERE id = ?1", [id])
        .map_err(db_error)?;
    Ok(())
}

/// The skill as a `SKILL.md` document for export.
pub fn export(skill: &Skill) -> String {
    let mut front = vec![
        format!("name: {}", skill.name),
        format!("description: {}", skill.description.replace('\n', " ")),
        format!("version: {}", skill.version),
    ];
    if let Some(license) = &skill.license {
        front.push(format!("license: {license}"));
    }
    if let Some(author) = &skill.author {
        front.push(format!("author: {author}"));
    }
    if !skill.requires.is_empty() {
        front.push(format!("allowed-tools: {}", skill.requires.join(", ")));
    }
    format!("---\n{}\n---\n\n{}\n", front.join("\n"), skill.body)
}

pub fn summary(skill: &Skill) -> Value {
    json!({
        "id": skill.id,
        "name": skill.name,
        "description": skill.description,
        "version": skill.version,
        "license": skill.license,
        "author": skill.author,
        "requires": skill.requires,
        "source": skill.source,
        "digest": skill.digest,
        "enabled": skill.enabled,
        "bundled": skill.bundled,
        "body": skill.body,
        "resources": skill.resources.iter().map(|resource| &resource.path).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skills_parse_and_round_trip() {
        for (id, text) in BUNDLED {
            let skill = parse(text, "bundled").expect("bundled skill parses");
            assert_eq!(&skill.id, id);
            let again = parse(&export(&skill), "bundled").expect("export parses");
            assert_eq!(skill.digest, again.digest, "{id} round-trips");
        }
    }

    #[test]
    fn malformed_and_unsafe_skills_are_refused() {
        assert!(parse("no front matter", "x").is_err());
        assert!(parse("---\nname: Bad Name\ndescription: d\n---\nbody", "x").is_err());
        assert!(parse("---\nname: ok\n---\nbody", "x").is_err());
        assert!(parse("---\nname: ok\ndescription: d\n---\n", "x").is_err());
        assert!(
            parse(
                &format!("---\nname: ok\ndescription: d\n---\n{}", "x".repeat(70_000)),
                "x"
            )
            .is_err()
        );
        let skill = parse(
            "---\nname: ok\ndescription: d\nallowed-tools: web.search, web.fetch\n---\nDo it.",
            "x",
        )
        .unwrap();
        assert_eq!(skill.requires, vec!["web.search", "web.fetch"]);
    }

    #[cfg(unix)]
    #[test]
    fn skill_folders_with_scripts_links_or_strays_are_refused() {
        let root = std::env::temp_dir().join(format!("juniper-skill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("references")).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            "---\nname: demo\ndescription: d\n---\nSteps.",
        )
        .unwrap();
        std::fs::write(root.join("references/guide.md"), "Guide").unwrap();
        let skill = read_folder(&root).expect("valid folder");
        assert_eq!(skill.resources.len(), 1);
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        assert!(
            read_folder(&root)
                .unwrap_err()
                .contains("SKILL_UNSUPPORTED")
        );
        std::fs::remove_dir_all(root.join("scripts")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("references/passwd.md")).unwrap();
        assert!(read_folder(&root).is_err());
        std::fs::remove_file(root.join("references/passwd.md")).unwrap();
        std::fs::write(root.join("payload.bin"), [0u8; 4]).unwrap();
        assert!(read_folder(&root).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn imported_skills_start_disabled_and_cannot_replace_bundled_ones() {
        let database =
            std::env::temp_dir().join(format!("juniper-skills-{}.db", uuid::Uuid::new_v4()));
        crate::storage::initialize(&database).unwrap();
        let connection = store::open(&database).unwrap();
        install_bundled(&connection).unwrap();
        let imported = import(
            &connection,
            parse("---\nname: mine\ndescription: d\n---\nSteps.", "x").unwrap(),
        )
        .unwrap();
        assert!(!imported.enabled);
        assert!(
            import(
                &connection,
                parse("---\nname: draft-and-save\ndescription: d\n---\nx", "x").unwrap()
            )
            .is_err()
        );
        assert!(
            body(&connection, "mine").is_err(),
            "a disabled skill does not load"
        );
        set_enabled(&connection, "mine", true).unwrap();
        assert_eq!(body(&connection, "mine").unwrap(), "Steps.");
        assert!(remove(&connection, "draft-and-save").is_err());
        std::fs::remove_file(database).ok();
    }
}
