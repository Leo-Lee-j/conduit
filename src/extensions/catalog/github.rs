use super::{Item, Page, Payload, Skill, digest, fail, http::Transport, receipts, text};
use anyhow::{Result, ensure};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

mod archive;
mod refs;

const MAX_BYTES: usize = 20 * 1024 * 1024;
const MAX_FILES: usize = 2000;
const PAGE_SIZE: usize = 20;
const MAX_TREE_FILES: usize = 40_000;

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
pub(super) fn source(query: &str) -> Result<(Skill, bool)> {
    let (query, path_override) = query
        .trim()
        .split_once('#')
        .map_or((query.trim(), None), |(q, p)| (q, Some(p.to_owned())));
    let (query, revision) = if query.starts_with("https://") {
        (query, "HEAD")
    } else {
        query.split_once('@').unwrap_or((query, "HEAD"))
    };
    let mut revision = revision.to_owned();
    let mut path = path_override;
    let mut exact = false;
    let repository = if query.starts_with("https://") {
        let url = url::Url::parse(query)?;
        ensure!(
            url.host_str() == Some("github.com")
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443),
            text("invalid_repository")
        );
        let mut segments = url.path().trim_matches('/').split('/');
        let owner = segments.next().unwrap_or_default();
        let repo = segments.next().unwrap_or_default();
        if let Some(kind) = segments.next() {
            ensure!(matches!(kind, "tree" | "blob"), text("invalid_repository"));
            let reference = segments.next().ok_or_else(|| fail("invalid_source"))?;
            revision = decode_url_segment(reference)?;
            let mut directory = segments
                .map(decode_url_segment)
                .collect::<Result<Vec<_>>>()?;
            // A URL path segment must not become a new path component after decoding.
            ensure!(
                !directory.iter().any(|part| part.contains('/')),
                text("invalid_path")
            );
            if kind == "blob" {
                ensure!(
                    directory.pop().as_deref() == Some("SKILL.md"),
                    text("invalid_source")
                );
                exact = true;
            }
            // GitHub view fragments (e.g. #L10) do not identify the skill path.
            path = (exact || !directory.is_empty()).then(|| directory.join("/"));
        }
        format!("{owner}/{}", repo.strip_suffix(".git").unwrap_or(repo))
    } else {
        query.strip_suffix(".git").unwrap_or(query).to_owned()
    };
    repository_name(&repository)?;
    ensure!(
        !revision.is_empty() && revision.len() <= 200 && !revision.chars().any(char::is_control),
        text("invalid_source")
    );
    if let Some(path) = path.as_deref().filter(|path| !path.is_empty()) {
        relative_path(path)?;
    }
    Ok((
        Skill {
            repository,
            revision,
            path,
            skill_id: None,
        },
        exact,
    ))
}
fn decode_url_segment(segment: &str) -> Result<String> {
    percent_decode_str(segment)
        .decode_utf8()
        .map(|decoded| decoded.into_owned())
        .map_err(|_| fail("invalid_source"))
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
    let value = client.json(
        &api(&skill.repository, &["commits", &skill.revision]),
        512 * 1024,
    )?;
    let sha = value["sha"]
        .as_str()
        .ok_or_else(|| fail("invalid_response"))?;
    ensure!(valid_commit(sha), text("invalid_response"));
    Ok(sha.to_lowercase())
}
fn valid_commit(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())
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
    ensure!(value.tree.len() <= MAX_TREE_FILES, text("response_limit"));
    Ok(value.tree)
}

struct Snapshot {
    commit: String,
    files: Vec<File>,
    archive: Option<archive::Archive>,
}
fn snapshot(client: &impl Transport, skill: &Skill) -> Result<Snapshot> {
    repository_name(&skill.repository)?;
    let commit = if valid_commit(&skill.revision) {
        Ok(skill.revision.to_ascii_lowercase())
    } else {
        commit(client, skill)
    };
    let pinned = match commit {
        Ok(commit) => {
            if let Ok(files) = tree(client, &skill.repository, &commit) {
                return Ok(Snapshot {
                    commit,
                    files,
                    archive: None,
                });
            }
            commit
        }
        Err(_) => refs::resolve(client, &skill.repository, &skill.revision)?,
    };
    client.check()?;
    let (files, archive) = archive::load(client, &skill.repository, &pinned)?;
    Ok(Snapshot {
        commit: pinned,
        files,
        archive: Some(archive),
    })
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
            if directory.is_some_and(|d| {
                path != d
                    && (d.is_empty()
                        || !path
                            .strip_prefix(d)
                            .is_some_and(|tail| tail.starts_with('/')))
            }) {
                return None;
            }
            Some(path.to_owned())
        })
        .collect()
}
fn metadata(
    client: &impl Transport,
    archive: Option<&mut archive::Archive>,
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
    let bytes = read_file(client, archive, repository, commit, &file, 2 * 1024 * 1024)?;
    let markdown = String::from_utf8(bytes).map_err(|_| fail("invalid_skill"))?;
    crate::extensions::edit::validate(Path::new("SKILL.md"), &markdown)?;
    let (name, description) = crate::extensions::skill_metadata(&markdown);
    let name = name.ok_or_else(|| fail("invalid_skill"))?;
    crate::extensions::edit::skill_path(Path::new("skills"), &name)?;
    ensure!(description.len() <= 4096, text("invalid_skill"));
    Ok((name, description, markdown))
}
fn read_file(
    client: &impl Transport,
    archive: Option<&mut archive::Archive>,
    repository: &str,
    commit: &str,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>> {
    match archive {
        Some(archive) => archive.read(client, path, limit),
        None => client.get(&raw(repository, commit, path), limit),
    }
}
pub(super) fn discover(client: &impl Transport, query: &str, cursor: Option<&str>) -> Result<Page> {
    let (mut source, exact) = source(query)?;
    let mut offset = 0;
    if let Some(cursor) = cursor {
        let (commit, start) = cursor
            .split_once(':')
            .ok_or_else(|| fail("invalid_source"))?;
        ensure!(valid_commit(commit), text("invalid_source"));
        offset = start
            .parse::<usize>()
            .ok()
            .filter(|offset| *offset <= MAX_TREE_FILES)
            .ok_or_else(|| fail("invalid_source"))?;
        source.revision = commit.to_owned();
    }
    let Snapshot {
        commit,
        files,
        mut archive,
    } = snapshot(client, &source)?;
    let mut paths = definitions(&files, source.path.as_deref());
    if exact {
        paths.retain(|path| Some(path.as_str()) == source.path.as_deref());
    }
    paths.sort_unstable();
    paths.dedup();
    ensure!(!paths.is_empty(), text("no_skills"));
    let end = (offset + PAGE_SIZE).min(paths.len());
    let next = (end < paths.len()).then(|| format!("{commit}:{end}"));
    let mut items = Vec::new();
    let mut invalid = false;
    for path in paths.iter().skip(offset).take(PAGE_SIZE) {
        client.check()?;
        match metadata(client, archive.as_mut(), &source.repository, &commit, path) {
            Ok((name, description, _)) => {
                let mut item =
                    super::skill_item(&source.repository, &name, Some(path), &description);
                if let Payload::Skill(skill) = &mut item.payload {
                    skill.revision = commit.clone();
                }
                items.push(item);
            }
            Err(_) => invalid = true,
        }
    }
    Ok(Page {
        items,
        next,
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
    archive: Option<archive::Archive>,
}
pub fn prepare_skill(client: &super::Client<'_>, item: &Item) -> Result<PreparedSkill> {
    prepare(client, item)
}
fn prepare(client: &impl Transport, item: &Item) -> Result<PreparedSkill> {
    let Payload::Skill(source) = &item.payload else {
        return Err(fail("invalid_source"));
    };
    let Snapshot {
        commit,
        files,
        mut archive,
    } = snapshot(client, source)?;
    let mut paths = match &source.path {
        Some(path) => vec![path.clone()],
        None => definitions(&files, None),
    };
    // skills.sh normally uses the skill's directory slug. Check those paths first,
    // without fetching hundreds of unrelated definitions from large repositories.
    let preferred = source.skill_id.as_deref().filter(|_| source.path.is_none());
    if let Some(id) = preferred {
        paths.sort_by_key(|path| path.rsplit('/').next() != Some(id));
    }
    let mut selected = None;
    let mut selected_preferred = false;
    for path in paths {
        let matches_slug = preferred.is_some_and(|id| path.rsplit('/').next() == Some(id));
        if selected_preferred && !matches_slug {
            break;
        }
        let Ok((name, _, markdown)) =
            metadata(client, archive.as_mut(), &source.repository, &commit, &path)
        else {
            client.check()?;
            continue;
        };
        if source.skill_id.as_ref().is_none_or(|id| id == &name) {
            ensure!(selected.is_none(), text("ambiguous_skill"));
            selected = Some((path, name, markdown));
            selected_preferred = matches_slug;
        }
    }
    let (directory, name, markdown) = selected.ok_or_else(|| fail("skill_not_found"))?;
    let prefix = format!("{directory}/");
    let selected = files
        .into_iter()
        .filter(|file| directory.is_empty() || file.path.starts_with(&prefix))
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
        archive,
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
    let mut archive = skill.archive.clone();
    validate_files(&skill.files, &skill.directory)?;
    for file in &skill.files {
        client.check()?;
        let relative = local_path(&file.path, &skill.directory)?;
        let bytes = read_file(
            client,
            archive.as_mut(),
            &skill.repository,
            &skill.commit,
            &file.path,
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
    use std::io::{Cursor, Write};
    use zip::{ZipWriter, write::SimpleFileOptions};
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
        for kind in ["tree", "blob"] {
            let file = if kind == "blob" { "/SKILL.md" } else { "" };
            let url = format!(
                "https://github.com/owner/repo/{kind}/feature%2Ftest/skills/my%20skill{file}?plain=1#L10"
            );
            let (skill, exact) = source(&url).unwrap();
            assert_eq!(exact, kind == "blob");
            assert_eq!(skill.repository, "owner/repo");
            assert_eq!(skill.revision, "feature/test");
            assert_eq!(skill.path.as_deref(), Some("skills/my skill"));
        }
        assert!(source("https://user@github.com/owner/repo").is_err());
        assert!(source("https://github.com/owner/repo/tree/main/a%2Fb").is_err());
        assert!(source("https://github.com/owner/repo/blob/main/README.md").is_err());
        assert!(super::super::is_github_reference("owner/repo#skills/task"));
        assert!(!super::super::is_github_reference("react testing"));
    }
    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "conduit-catalog-archive-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn archive_fixture(link: bool) -> Fake {
        let mut client = Fake::new();
        let packet = format!("{SHA} HEAD\0symref=HEAD:refs/heads/main\n");
        client.add(
            "https://github.com/owner/repo.git/info/refs?service=git-upload-pack",
            format!(
                "001e# service=git-upload-pack\n0000{:04x}{packet}0000",
                packet.len() + 4
            ),
        );
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..105 {
            let skill = format!("skill-{i:03}");
            zip.start_file(format!("repo-{SHA}/skills/{skill}/SKILL.md"), options)
                .unwrap();
            write!(
                zip,
                "---\nname: {skill}\ndescription: Test\n---\nTest skill\n"
            )
            .unwrap();
        }
        zip.start_file(
            format!("repo-{SHA}/skills/skill-104/references/readme.txt"),
            options,
        )
        .unwrap();
        zip.write_all(b"hello").unwrap();
        if link {
            zip.add_symlink(
                format!("repo-{SHA}/skills/skill-104/link"),
                "../outside",
                options,
            )
            .unwrap();
        }
        client.add(
            &format!("https://codeload.github.com/owner/repo/zip/{SHA}"),
            zip.finish().unwrap().into_inner(),
        );
        client
    }
    #[test]
    fn without_rest_paginates_large_repos_and_installs_from_the_pinned_archive() {
        let client = archive_fixture(false);
        let mut next = None;
        let mut names = Vec::new();
        loop {
            let page = discover(&client, "owner/repo", next.as_deref()).unwrap();
            names.extend(page.items.iter().map(|item| item.name.clone()));
            next = page.next;
            if next.is_none() {
                break;
            }
        }
        assert_eq!(names.len(), 105);
        assert_eq!(names.last().unwrap(), "skill-104");
        let item = super::super::skill_item("owner/repo", "skill-104", None, "");
        let skill = prepare(&client, &item).unwrap();
        assert_eq!(skill.commit, SHA);
        assert_eq!(skill.files.len(), 2);
        let root = test_root();
        let target = install(
            &Fake::new(),
            &skill,
            &root.join("skills"),
            &root.join("ledger"),
        )
        .unwrap();
        assert_eq!(
            fs::read(target.join("references/readme.txt")).unwrap(),
            b"hello"
        );
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn archive_fallback_rejects_symlinks_in_selected_skill() {
        let client = archive_fixture(true);
        let item = super::super::skill_item("owner/repo", "skill-104", None, "");
        assert_eq!(
            prepare(&client, &item).unwrap_err().to_string(),
            text("linked_file")
        );
    }
    #[test]
    #[ignore = "requires network; performs public archive fallback without calling GitHub REST"]
    fn public_skill_install_with_rest_blocked() {
        struct NoRest<'a>(super::super::Client<'a>);
        impl Transport for NoRest<'_> {
            fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
                ensure!(
                    !url.starts_with("https://api.github.com/"),
                    "simulated REST rate limit"
                );
                self.0.get(url, limit)
            }
            fn check(&self) -> Result<()> {
                self.0.check()
            }
        }
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let client = NoRest(super::super::Client::new(&cancel));
        let page = discover(
            &client,
            "https://github.com/vercel-labs/agent-skills/tree/main/skills/web-design-guidelines",
            None,
        )
        .unwrap();
        let skill = prepare(&client, &page.items[0]).unwrap();
        let root = test_root();
        let result = install(
            &Fake::new(),
            &skill,
            &root.join("skills"),
            &root.join("ledger"),
        );
        let verified = result.as_ref().is_ok_and(|path| {
            fs::read_to_string(path.join("SKILL.md")).is_ok_and(|content| content == skill.markdown)
        });
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        result.unwrap();
        assert!(verified);
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
                .0
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
