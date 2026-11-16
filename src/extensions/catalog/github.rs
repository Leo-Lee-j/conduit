use super::{Item, Page, Payload, Skill, digest, fail, http::Transport, receipts, text};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAX_FILES: usize = 2000;

pub(super) fn repository_name(value: &str) -> Result<String> {
    let parts: Vec<_> = value.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.len() <= 100
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))),
        text("invalid_repository")
    );
    Ok(value.to_owned())
}
fn source(query: &str) -> Result<Skill> {
    let (query, path) = query
        .trim()
        .split_once('#')
        .map_or((query.trim(), None), |(q, p)| (q, Some(p)));
    let (query, revision) = query.split_once('@').unwrap_or((query, "HEAD"));
    let repository = if query.starts_with("https://") {
        let url = url::Url::parse(query)?;
        ensure!(
            url.host_str() == Some("github.com")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.port_or_known_default() == Some(443),
            text("invalid_repository")
        );
        url.path()
            .trim_matches('/')
            .trim_end_matches(".git")
            .to_owned()
    } else {
        query.to_owned()
    };
    repository_name(&repository)?;
    ensure!(
        !revision.is_empty() && revision.len() <= 200 && !revision.chars().any(char::is_control),
        text("invalid_source")
    );
    if let Some(path) = path {
        relative_path(path)?;
    }
    Ok(Skill {
        repository,
        revision: revision.into(),
        path: path.map(str::to_owned),
        skill_id: None,
    })
}
fn api(repository: &str, segments: &[&str]) -> String {
    let mut url = url::Url::parse("https://api.github.com/repos/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .extend(repository.split('/'))
        .extend(segments);
    url.into()
}
fn raw(repository: &str, commit: &str, path: &str) -> String {
    let mut url = url::Url::parse("https://raw.githubusercontent.com/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .extend(repository.split('/'))
        .push(commit)
        .extend(path.split('/'));
    url.into()
}
fn commit(client: &impl Transport, skill: &Skill) -> Result<String> {
    repository_name(&skill.repository)?;
    let value = client.json(
        &api(&skill.repository, &["commits", &skill.revision]),
        512 * 1024,
    )?;
    let sha = value["sha"]
        .as_str()
        .ok_or_else(|| fail("invalid_response"))?;
    ensure!(
        sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        text("invalid_response")
    );
    Ok(sha.to_lowercase())
}
#[derive(Clone, Debug, Deserialize)]
pub struct File {
    pub path: String,
    mode: String,
    #[serde(default)]
    size: usize,
}
#[derive(Deserialize)]
struct Tree {
    tree: Vec<File>,
    truncated: bool,
}
fn tree(client: &impl Transport, repository: &str, commit: &str) -> Result<Vec<File>> {
    let url = format!("{}?recursive=1", api(repository, &["git", "trees", commit]));
    let value: Tree = serde_json::from_value(client.json(&url, 8 * 1024 * 1024)?)
        .map_err(|_| fail("invalid_response"))?;
    ensure!(!value.truncated, text("truncated_tree"));
    ensure!(value.tree.len() <= 40000, text("response_limit"));
    Ok(value.tree)
}
fn definitions(files: &[File], directory: Option<&str>) -> Vec<String> {
    files
        .iter()
        .filter(|file| matches!(file.mode.as_str(), "100644" | "100755"))
        .filter_map(|file| {
            let path = if file.path == "SKILL.md" {
                ""
            } else {
                file.path.strip_suffix("/SKILL.md")?
            };
            if directory
                .is_some_and(|d| !d.is_empty() && path != d && !path.starts_with(&format!("{d}/")))
            {
                return None;
            }
            Some(path.to_owned())
        })
        .collect()
}
fn metadata(
    client: &impl Transport,
    repository: &str,
    commit: &str,
    path: &str,
) -> Result<(String, String, String)> {
    if !path.is_empty() {
        relative_path(path)?;
    }
    let file = if path.is_empty() {
        "SKILL.md".into()
    } else {
        format!("{path}/SKILL.md")
    };
    let bytes = client.get(&raw(repository, commit, &file), 2 * 1024 * 1024)?;
    let markdown = String::from_utf8(bytes).map_err(|_| fail("invalid_skill"))?;
    crate::extensions::edit::validate(Path::new("SKILL.md"), &markdown)?;
    let (name, description) = crate::extensions::skill_metadata(&markdown);
    let name = name.ok_or_else(|| fail("invalid_skill"))?;
    crate::extensions::edit::skill_path(Path::new("skills"), &name)?;
    ensure!(description.len() <= 4096, text("invalid_skill"));
    Ok((name, description, markdown))
}
pub(super) fn discover(client: &impl Transport, query: &str) -> Result<Page> {
    let source = source(query)?;
    let commit = commit(client, &source)?;
    let files = tree(client, &source.repository, &commit)?;
    let paths = definitions(&files, source.path.as_deref());
    ensure!(!paths.is_empty(), text("no_skills"));
    ensure!(paths.len() <= 60, text("too_many_skills"));
    let mut items = Vec::new();
    let mut invalid = false;
    for path in paths {
        client.check()?;
        match metadata(client, &source.repository, &commit, &path) {
            Ok((name, description, _)) => {
                let mut item =
                    super::skill_item(&source.repository, &name, Some(&path), &description);
                if let Payload::Skill(skill) = &mut item.payload {
                    skill.revision = commit.clone();
                }
                items.push(item);
            }
            Err(_) => invalid = true,
        }
    }
    ensure!(!items.is_empty(), text("invalid_skill"));
    Ok(Page {
        items,
        warning: invalid.then(|| text("skipped_skills")),
        ..Default::default()
    })
}

#[derive(Clone, Debug)]
pub struct PreparedSkill {
    pub item: Item,
    pub repository: String,
    pub commit: String,
    pub directory: String,
    pub name: String,
    pub markdown: String,
    pub files: Vec<File>,
    pub bytes: usize,
}
pub fn prepare_skill(client: &super::Client<'_>, item: &Item) -> Result<PreparedSkill> {
    prepare(client, item)
}
fn prepare(client: &impl Transport, item: &Item) -> Result<PreparedSkill> {
    let Payload::Skill(source) = &item.payload else {
        return Err(fail("invalid_source"));
    };
    let commit = commit(client, source)?;
    let files = tree(client, &source.repository, &commit)?;
    let paths = match &source.path {
        Some(path) => vec![path.clone()],
        None => definitions(&files, None),
    };
    ensure!(paths.len() <= 100, text("too_many_skills"));
    let mut selected = None;
    for path in paths {
        let Ok((name, _, markdown)) = metadata(client, &source.repository, &commit, &path) else {
            client.check()?;
            continue;
        };
        if source.skill_id.as_ref().is_none_or(|id| id == &name) {
            ensure!(selected.is_none(), text("ambiguous_skill"));
            selected = Some((path, name, markdown));
        }
    }
    let (directory, name, markdown) = selected.ok_or_else(|| fail("skill_not_found"))?;
    let selected = files
        .into_iter()
        .filter(|file| directory.is_empty() || file.path.starts_with(&format!("{directory}/")))
        .collect::<Vec<_>>();
    let bytes = validate_files(&selected, &directory)?;
    let files = selected
        .into_iter()
        .filter(|f| f.mode != "040000")
        .collect();
    Ok(PreparedSkill {
        item: item.clone(),
        repository: source.repository.clone(),
        commit,
        directory,
        name,
        markdown,
        files,
        bytes,
    })
}
fn relative_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 240
            && !path.contains(['\\', ':', '%', '?', '#'])
            && path.split('/').count() <= 24,
        text("invalid_path")
    );
    for part in path.split('/') {
        let base = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        let reserved = matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
            || (base.len() == 4
                && (base.starts_with("com") || base.starts_with("lpt"))
                && base.as_bytes()[3].is_ascii_digit());
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !reserved
                && !part.chars().any(|c| c.is_control() || "<>\"|*".contains(c)),
            text("invalid_path")
        );
    }
    Ok(())
}
fn local_path<'a>(path: &'a str, directory: &str) -> Result<&'a str> {
    let relative = if directory.is_empty() {
        path
    } else {
        path.strip_prefix(&format!("{directory}/"))
            .ok_or_else(|| fail("invalid_path"))?
    };
    relative_path(relative)?;
    Ok(relative)
}
fn validate_files(files: &[File], directory: &str) -> Result<usize> {
    ensure!(files.len() <= MAX_FILES, text("package_limit"));
    let mut paths = HashSet::new();
    let mut total = 0usize;
    for file in files {
        let relative = local_path(&file.path, directory)?;
        ensure!(
            matches!(file.mode.as_str(), "100644" | "100755" | "040000"),
            text("linked_file")
        );
        ensure!(
            paths.insert(relative.to_lowercase()),
            text("path_collision")
        );
        total = total
            .checked_add(file.size)
            .ok_or_else(|| fail("package_limit"))?;
        ensure!(total <= MAX_BYTES, text("package_limit"));
    }
    ensure!(
        files
            .iter()
            .any(|f| local_path(&f.path, directory).ok() == Some("SKILL.md") && f.mode != "040000"),
        text("invalid_skill")
    );
    Ok(total)
}

pub fn install_skill(
    client: &super::Client<'_>,
    skill: &PreparedSkill,
    destination: &Path,
    ledger: &Path,
) -> Result<PathBuf> {
    install(client, skill, destination, ledger)
}
fn install(
    client: &impl Transport,
    skill: &PreparedSkill,
    destination: &Path,
    ledger: &Path,
) -> Result<PathBuf> {
    let target = crate::extensions::edit::skill_path(destination, &skill.name)?
        .parent()
        .unwrap()
        .to_owned();
    client.check()?;
    fs::create_dir_all(destination)?;
    // Lock lives beside the destination, so concurrent app instances cannot publish the same skill.
    let lock_path = destination.join(format!(".conduit-install-{}.lock", skill.name));
    let _lock = Guard::lock(&lock_path)?;
    ensure!(!target.try_exists()?, text("already_exists"));
    let staging = destination.join(format!(
        ".conduit-download-{}-{}-{}",
        skill.name,
        std::process::id(),
        super::now()
    ));
    fs::create_dir(&staging)?;
    let _staging = Guard {
        path: staging.clone(),
        directory: true,
        handle: None,
    };
    let folder = staging.join(&skill.name);
    fs::create_dir(&folder)?;
    let mut total = 0usize;
    let mut hashes = Vec::new();
    validate_files(&skill.files, &skill.directory)?;
    for file in &skill.files {
        client.check()?;
        let relative = local_path(&file.path, &skill.directory)?;
        let bytes = client.get(
            &raw(&skill.repository, &skill.commit, &file.path),
            (MAX_BYTES - total).min(file.size),
        )?;
        ensure!(bytes.len() == file.size, text("content_changed"));
        total += bytes.len();
        if relative == "SKILL.md" {
            ensure!(bytes == skill.markdown.as_bytes(), text("content_changed"));
        }
        let out = folder.join(relative);
        fs::create_dir_all(out.parent().unwrap())?;
        fs::write(&out, &bytes)?;
        hashes.push((relative.to_owned(), digest(&bytes)));
    }
    client.check()?;
    let receipt = receipts::Receipt::skill(skill, &target, hashes);
    // Prepare provenance first. A failure after publication leaves a recoverable receipt.
    receipts::write(ledger, &receipt)?;
    let result = crate::extensions::edit::import_skill(&folder, destination);
    if result.is_err() {
        let _ = receipts::remove(ledger, &receipt);
    }
    result
}
struct Guard {
    path: PathBuf,
    directory: bool,
    handle: Option<fs::File>,
}
impl Guard {
    fn lock(path: &Path) -> Result<Self> {
        let handle = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| fail("install_busy"))?;
        Ok(Self {
            path: path.into(),
            directory: false,
            handle: Some(handle),
        })
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        drop(self.handle.take());
        if self.directory {
            let _ = fs::remove_dir_all(&self.path);
        } else {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::catalog::tests::Fake;
    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn github_urls_have_no_empty_path_segment_and_encode_refs() {
        assert_eq!(
            api("owner/repo", &["commits", "feature/test"]),
            "https://api.github.com/repos/owner/repo/commits/feature%2Ftest"
        );
        assert_eq!(
            raw("owner/repo", SHA, "skills/example/SKILL.md"),
            format!("https://raw.githubusercontent.com/owner/repo/{SHA}/skills/example/SKILL.md")
        );
    }
    fn fixture() -> (Fake, Item) {
        let mut client = Fake::new();
        let body = "---\nname: named-skill\ndescription: test\n---\nUse references/readme.txt";
        client.add(
            &api("owner/repo", &["commits", "HEAD"]),
            format!(r#"{{"sha":"{SHA}"}}"#),
        );
        client.add(
            &format!("{}?recursive=1", api("owner/repo", &["git", "trees", SHA])),
            serde_json::to_vec(&serde_json::json!({
                "truncated":false,"tree":[
                    {"path":"skills/different/SKILL.md","mode":"100644","size":body.len()},
                    {"path":"skills/different/references","mode":"040000"},
                    {"path":"skills/different/references/readme.txt","mode":"100644","size":5}
                ]
            }))
            .unwrap(),
        );
        client.add(&raw("owner/repo", SHA, "skills/different/SKILL.md"), body);
        client.add(
            &raw("owner/repo", SHA, "skills/different/references/readme.txt"),
            "hello",
        );
        (
            client,
            crate::extensions::catalog::skill_item("owner/repo", "named-skill", None, ""),
        )
    }
    #[test]
    fn installs_complete_pinned_skill_resolves_frontmatter_and_refuses_overwrite() {
        let (client, item) = fixture();
        let skill = prepare(&client, &item).unwrap();
        assert_eq!(skill.directory, "skills/different");
        assert_eq!(skill.name, "named-skill");
        let root = std::env::temp_dir().join(format!(
            "conduit-catalog-install-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target = install(&client, &skill, &root.join("skills"), &root.join("ledger")).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("references/readme.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            skill.markdown
        );
        assert!(install(&client, &skill, &root.join("skills"), &root.join("ledger")).is_err());
        assert_eq!(fs::read_dir(root.join("skills")).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_traversal_links_case_collisions_and_truncated_trees() {
        for path in [
            "../x",
            "a/../b",
            "/absolute",
            "a\\b",
            "con.txt",
            "a:b",
            "trailing.",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
        let (mut client, item) = fixture();
        client.add(
            &format!("{}?recursive=1", api("owner/repo", &["git", "trees", SHA])),
            r#"{"truncated":true,"tree":[]}"#,
        );
        assert!(prepare(&client, &item).is_err());
        let file = |path: &str, mode: &str| File {
            path: path.into(),
            mode: mode.into(),
            size: 1,
        };
        assert!(
            validate_files(
                &[file("SKILL.md", "100644"), file("skill.MD", "100644")],
                ""
            )
            .is_err()
        );
        assert!(validate_files(&[file("SKILL.md", "100644"), file("link", "120000")], "").is_err());
        assert!(source("https://evil.invalid/owner/repo").is_err());
        assert_eq!(
            source("owner/repo@feature/name#skills/one")
                .unwrap()
                .revision,
            "feature/name"
        );
    }
    #[test]
    fn failed_download_leaves_no_partial_skill_or_receipt() {
        let (mut client, item) = fixture();
        let skill = prepare(&client, &item).unwrap();
        client.add(
            &raw("owner/repo", SHA, "skills/different/references/readme.txt"),
            "bad",
        );
        let root = std::env::temp_dir().join(format!(
            "conduit-catalog-fail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(install(&client, &skill, &root.join("skills"), &root.join("ledger")).is_err());
        assert_eq!(fs::read_dir(root.join("skills")).unwrap().count(), 0);
        assert!(!root.join("ledger").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
