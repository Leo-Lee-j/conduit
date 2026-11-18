//! Select the repository's hosting service. Only the repository's own host is ever probed.
use super::*;
use anyhow::ensure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    Github,
    Gitlab,
}

impl Forge {
    pub fn name(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Gitlab => "GitLab",
        }
    }
    pub fn request_name(self, number: u64) -> String {
        match self {
            Self::Github => format!("PR #{number}"),
            Self::Gitlab => format!("MR !{number}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub forge: Forge,
    pub host: String,
    pub project: String,
}

fn valid_project(project: &str) -> bool {
    project.split('/').count() >= 2
        && project.len() <= 4096
        && project.split('/').all(|part| {
            !part.is_empty()
                && !matches!(part, "." | "..")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

fn valid_host(host: &str) -> bool {
    host.len() <= 255
        && url::Url::parse(&format!("https://{host}/")).is_ok_and(|url| {
            url.host_str().is_some()
                && url[url::Position::BeforeHost..url::Position::AfterPort]
                    .eq_ignore_ascii_case(host)
        })
}

pub fn parse_remote(remote: &str) -> Result<Target> {
    ensure!(
        !remote.contains(['\n', '\r', '\0', '\\']),
        "Invalid remote URL"
    );
    let parsed = if remote.contains("://") {
        url::Url::parse(remote)?
    } else {
        let (host, project) = remote.split_once(':').context("Unsupported remote URL")?;
        ensure!(!host.contains('/'), "Invalid SSH remote");
        url::Url::parse(&format!("ssh://{host}/{project}"))?
    };
    ensure!(
        matches!(parsed.scheme(), "https" | "http" | "ssh")
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "Unsupported remote URL"
    );
    let hostname = parsed.host_str().context("Missing remote hostname")?;
    let host = if parsed.scheme() != "ssh" {
        parsed[url::Position::BeforeHost..url::Position::AfterPort].to_owned()
    } else {
        // An SSH port is not the web/API port; API requests use HTTPS on the default port.
        hostname.to_owned()
    };
    let project = parsed.path().trim_start_matches('/').trim_end_matches('/');
    let project = project.strip_suffix(".git").unwrap_or(project);
    ensure!(valid_project(project), "Invalid repository path");
    let forge = if hostname
        .split('.')
        .any(|part| part.eq_ignore_ascii_case("gitlab"))
    {
        Forge::Gitlab
    } else {
        // Preserve GitHub Enterprise behavior for arbitrary existing hostnames.
        Forge::Github
    };
    Ok(Target {
        forge,
        host,
        project: project.into(),
    })
}

fn config(git: &Path, workspace: &Path, key: &str, cancel: &AtomicBool) -> Result<String> {
    match query(git, &["config", "--get", key], workspace, cancel) {
        Ok(value) => Ok(value.trim().into()),
        Err(error) if error.to_string().starts_with("1: ") => Ok(String::new()),
        Err(error) => Err(error),
    }
}

pub fn target(workspace: &Path, cancel: &AtomicBool) -> Result<Target> {
    let git = crate::git::resolve_git().context(conduit::i18n::text("github.missing_git"))?;
    let branch = query(
        &git,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        workspace,
        cancel,
    )?;
    let branch = branch.trim();
    let remotes = query(&git, &["remote"], workspace, cancel)?;
    let remotes: Vec<_> = remotes.lines().collect();
    let mut remote = String::new();
    for key in [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".into(),
        format!("branch.{branch}.remote"),
    ] {
        remote = config(&git, workspace, &key, cancel)?;
        if !remote.is_empty() {
            break;
        }
    }
    if remote.is_empty() {
        remote = if remotes.contains(&"origin") {
            "origin".into()
        } else if remotes.len() == 1 {
            remotes[0].into()
        } else {
            bail!(conduit::i18n::text("forge.remote_required"));
        };
    }
    ensure!(
        remotes.contains(&remote.as_str()),
        "Configured remote is unavailable"
    );
    let urls = query(
        &git,
        &["remote", "get-url", "--all", &remote],
        workspace,
        cancel,
    )?;
    let urls: Vec<_> = urls.lines().collect();
    ensure!(urls.len() == 1, "Multiple remote URLs are ambiguous");
    let mut target = parse_remote(urls[0])?;
    // SSH host aliases or a separate web/API host: `git config conduit.forgeHost host[:port]`.
    let host = config(&git, workspace, "conduit.forgeHost", cancel)?;
    if !host.is_empty() {
        ensure!(valid_host(&host), "Invalid conduit.forgeHost");
        target.host = host;
    }
    match config(&git, workspace, "conduit.forge", cancel)?.as_str() {
        "" | "auto" => {
            // Self-hosted GitLab often lacks a `gitlab` label in its hostname.
            if target.forge == Forge::Github
                && !target.host.eq_ignore_ascii_case("github.com")
                && api::is_gitlab(&target.host)
            {
                target.forge = Forge::Gitlab;
            }
        }
        "github" => target.forge = Forge::Github,
        "gitlab" => target.forge = Forge::Gitlab,
        _ => bail!(conduit::i18n::text("forge.invalid_provider")),
    }
    Ok(target)
}

pub fn select(workspace: &Path, forge: Option<Forge>, cancel: &AtomicBool) -> Result<()> {
    let git = crate::git::resolve_git().context(conduit::i18n::text("github.missing_git"))?;
    let value = match forge {
        None => "auto",
        Some(Forge::Github) => "github",
        Some(Forge::Gitlab) => "gitlab",
    };
    query(
        &git,
        &["config", "--local", "conduit.forge", value],
        workspace,
        cancel,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_provider_selection_and_remote_precedence_survive_reload() {
        let git = crate::git::resolve_git().expect("Git is required for repository tests");
        let root = std::env::temp_dir().join(format!(
            "conduit-forge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let cancel = AtomicBool::new(false);
        let run = |args: &[&str]| query(&git, args, &root, &cancel).unwrap();
        run(&["init", "-b", "topic"]);
        run(&[
            "remote",
            "add",
            "origin",
            "https://code.example.test/team/sub/repo.git",
        ]);
        assert_eq!(target(&root, &cancel).unwrap().forge, Forge::Github);
        select(&root, Some(Forge::Gitlab), &cancel).unwrap();
        assert_eq!(
            run(&["config", "--local", "--get", "conduit.forge"]).trim(),
            "gitlab"
        );
        assert_eq!(target(&root, &cancel).unwrap().forge, Forge::Gitlab);
        run(&[
            "remote",
            "add",
            "fork",
            "ssh://git@code.example.test:2222/me/repo.git",
        ]);
        run(&["config", "branch.topic.pushRemote", "fork"]);
        assert_eq!(target(&root, &cancel).unwrap().project, "me/repo");
        run(&["config", "conduit.forgeHost", "api.code.example.test:8443"]);
        assert_eq!(
            target(&root, &cancel).unwrap().host,
            "api.code.example.test:8443"
        );
        for invalid in ["a/b", "user@host", "host?x"] {
            run(&["config", "conduit.forgeHost", invalid]);
            assert!(target(&root, &cancel).is_err(), "{invalid}");
        }
        run(&["config", "--unset", "conduit.forgeHost"]);
        run(&["config", "--unset", "branch.topic.pushRemote"]);
        select(&root, None, &cancel).unwrap();
        assert_eq!(target(&root, &cancel).unwrap().project, "team/sub/repo");
        assert_eq!(target(&root, &cancel).unwrap().forge, Forge::Github);
        run(&[
            "remote",
            "set-url",
            "--add",
            "origin",
            "https://other.test/a/b.git",
        ]);
        assert!(target(&root, &cancel).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn self_hosted_remotes_preserve_subgroups_and_web_ports() {
        for remote in [
            "git@gitlab.example.test:team/sub/repo.git",
            "ssh://git@gitlab.example.test:2222/team/sub/repo.git",
            "https://gitlab.example.test/team/sub/repo.git",
        ] {
            let target = parse_remote(remote).unwrap();
            assert_eq!(target.forge, Forge::Gitlab);
            assert_eq!(target.host, "gitlab.example.test");
            assert_eq!(target.project, "team/sub/repo");
        }
        let target = parse_remote("https://code.example.test:8443/team/sub/repo.git").unwrap();
        assert_eq!(target.host, "code.example.test:8443");
        assert_eq!(target.forge, Forge::Github);
        for invalid in [
            "C:\\work",
            "file:///tmp/repo",
            "https://user:secret@host/a/b",
            "https://host/a/b?token=secret",
            "git@host:a/b\nother",
            "https://host/a/%2fescape",
        ] {
            assert!(parse_remote(invalid).is_err(), "{invalid}");
        }
    }
}
