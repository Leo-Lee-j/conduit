//! Forge API tokens without requiring gh/glab: a token saved in Conduit (DPAPI),
//! environment variables, Git Credential Manager, then an installed forge CLI.
use super::*;
use forge::Forge;
use std::{collections::BTreeMap, fmt};

const MAX_TOKEN_BYTES: usize = 4096;

/// No usable credential was found, or every candidate was rejected by the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequired {
    pub forge: Forge,
    pub host: String,
    pub rejected: bool,
}

impl fmt::Display for AuthRequired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&conduit::i18n::format(
            if self.rejected {
                "forge.auth_rejected"
            } else {
                "forge.auth_required"
            },
            &[
                ("forge", self.forge.name().into()),
                ("host", self.host.clone()),
            ],
        ))
    }
}

impl std::error::Error for AuthRequired {}

/// Return the sign-in requirement carried by a forge error, if any.
pub fn required(error: &anyhow::Error) -> Option<AuthRequired> {
    error
        .chain()
        .find_map(|e| e.downcast_ref::<AuthRequired>().cloned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Saved,
    Environment,
    GitCredential,
    Cli,
}

fn slug(forge: Forge) -> &'static str {
    match forge {
        Forge::Github => "github",
        Forge::Gitlab => "gitlab",
    }
}

fn key(forge: Forge, host: &str) -> String {
    format!("{}:{}", slug(forge), host.to_ascii_lowercase())
}

fn store_path() -> PathBuf {
    crate::store::Store::path().with_file_name("forge-tokens.json")
}

fn read_store(path: &Path) -> Result<BTreeMap<String, String>> {
    match std::fs::read_to_string(path) {
        Ok(json) => Ok(serde_json::from_str(&json).context("Invalid forge token store")?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error.into()),
    }
}

fn write_store(path: &Path, tokens: &BTreeMap<String, String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(tokens)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_BYTES
        && token.bytes().all(|b| b.is_ascii_graphic())
}

fn save_in(path: &Path, forge: Forge, host: &str, token: &str) -> Result<()> {
    let token = token.trim();
    anyhow::ensure!(
        valid_token(token),
        "{}",
        conduit::i18n::text("forge.token_invalid")
    );
    let mut tokens = read_store(path)?;
    tokens.insert(
        key(forge, host),
        crate::providers::credentials::protect(token)?,
    );
    write_store(path, &tokens)
}

fn remove_in(path: &Path, forge: Forge, host: &str) -> Result<()> {
    let mut tokens = read_store(path)?;
    if tokens.remove(&key(forge, host)).is_some() {
        write_store(path, &tokens)?;
    }
    Ok(())
}

fn saved_in(path: &Path, forge: Forge, host: &str) -> Result<Option<String>> {
    read_store(path)?
        .get(&key(forge, host))
        .map(|protected| crate::providers::credentials::unprotect(protected))
        .transpose()
}

/// Save a personal access token for one forge host, encrypted for the current Windows user.
pub fn save(forge: Forge, host: &str, token: &str) -> Result<()> {
    save_in(&store_path(), forge, host, token)
}

/// Forget the token Conduit saved for one forge host. Other sources are untouched.
pub fn remove(forge: Forge, host: &str) -> Result<()> {
    remove_in(&store_path(), forge, host)
}

pub fn has_saved(forge: Forge, host: &str) -> bool {
    read_store(&store_path()).is_ok_and(|tokens| tokens.contains_key(&key(forge, host)))
}

fn environment(forge: Forge, host: &str) -> Option<String> {
    // Same variables and host scoping as gh / glab.
    let names: &[&str] = match forge {
        Forge::Github if host.eq_ignore_ascii_case("github.com") => &["GH_TOKEN", "GITHUB_TOKEN"],
        Forge::Github => &["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"],
        Forge::Gitlab => &["GITLAB_TOKEN", "GITLAB_ACCESS_TOKEN"],
    };
    names
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| value.trim().to_owned())
        .find(|value| valid_token(value))
}

pub(super) fn parse_credential(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::trim)
        .filter(|token| valid_token(token))
        .map(str::to_owned)
}

fn git_credential(host: &str, workspace: &Path, cancel: &AtomicBool) -> Option<String> {
    // Reuse the token Git Credential Manager stored when the user pushed over HTTPS.
    // Read-only: never approve or reject, so a failed API call cannot erase Git's credential.
    let git = crate::git::resolve_git()?;
    let input = format!("protocol=https\nhost={host}\n\n");
    let output = query_with_input(
        &git,
        &[
            "-c",
            "credential.interactive=never",
            "-c",
            "core.askPass=",
            "credential",
            "fill",
        ],
        workspace,
        cancel,
        Some(input.as_bytes()),
    )
    .ok()?;
    parse_credential(&output)
}

fn cli_token(forge: Forge, host: &str, workspace: &Path, cancel: &AtomicBool) -> Option<String> {
    let (cli, args): (PathBuf, Vec<&str>) = match forge {
        Forge::Github => (executable()?, vec!["auth", "token", "--hostname", host]),
        Forge::Gitlab => (
            gitlab::executable()?,
            vec!["config", "get", "token", "--host", host],
        ),
    };
    let token = query(&cli, &args, workspace, cancel).ok()?;
    let token = token.trim();
    valid_token(token).then(|| token.to_owned())
}

/// Lazily yields candidate tokens in priority order, skipping duplicates.
pub(super) struct Tokens<'a> {
    forge: Forge,
    host: String,
    workspace: &'a Path,
    cancel: &'a AtomicBool,
    next: usize,
    tried: Vec<String>,
}

impl<'a> Tokens<'a> {
    pub fn new(forge: Forge, host: &str, workspace: &'a Path, cancel: &'a AtomicBool) -> Self {
        Self {
            forge,
            host: host.to_owned(),
            workspace,
            cancel,
            next: 0,
            tried: vec![],
        }
    }

    pub fn required(&self) -> AuthRequired {
        AuthRequired {
            forge: self.forge,
            host: self.host.clone(),
            rejected: !self.tried.is_empty(),
        }
    }

    pub fn next(&mut self) -> Result<Option<(String, Source)>> {
        while self.next < 4 {
            let source = [
                Source::Saved,
                Source::Environment,
                Source::GitCredential,
                Source::Cli,
            ][self.next];
            self.next += 1;
            if self.cancel.load(Ordering::Acquire) {
                bail!(conduit::i18n::text("github.cancelled"));
            }
            let token = match source {
                // A corrupt or foreign-user store must surface instead of silently falling through.
                Source::Saved => saved_in(&store_path(), self.forge, &self.host)?,
                Source::Environment => environment(self.forge, &self.host),
                Source::GitCredential => git_credential(&self.host, self.workspace, self.cancel),
                Source::Cli => cli_token(self.forge, &self.host, self.workspace, self.cancel),
            };
            if let Some(token) = token
                && !self.tried.contains(&token)
            {
                self.tried.push(token.clone());
                return Ok(Some((token, source)));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_tokens_are_scoped_per_forge_and_host_and_encrypted() {
        let root = std::env::temp_dir().join(format!(
            "conduit-forge-auth-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = root.join("forge-tokens.json");
        save_in(
            &path,
            Forge::Gitlab,
            "GitLab.Example.test",
            " glpat-secret \n",
        )
        .unwrap();
        assert_eq!(
            saved_in(&path, Forge::Gitlab, "gitlab.example.test")
                .unwrap()
                .as_deref(),
            Some("glpat-secret")
        );
        assert!(
            saved_in(&path, Forge::Github, "gitlab.example.test")
                .unwrap()
                .is_none()
        );
        assert!(
            saved_in(&path, Forge::Gitlab, "other.test")
                .unwrap()
                .is_none()
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("glpat-secret")
        );
        for invalid in ["", "  ", "two words", "a\nb"] {
            assert!(
                save_in(&path, Forge::Gitlab, "h", invalid).is_err(),
                "{invalid:?}"
            );
        }
        remove_in(&path, Forge::Gitlab, "gitlab.example.test").unwrap();
        assert!(
            saved_in(&path, Forge::Gitlab, "gitlab.example.test")
                .unwrap()
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn credential_output_yields_only_a_valid_password() {
        assert_eq!(
            parse_credential("protocol=https\nhost=github.com\nusername=me\npassword=gho_abc\n")
                .as_deref(),
            Some("gho_abc")
        );
        assert!(parse_credential("protocol=https\nhost=github.com\n").is_none());
        assert!(parse_credential("password=\n").is_none());
    }

    #[test]
    fn auth_errors_survive_context_wrapping() {
        let error = anyhow::Error::new(AuthRequired {
            forge: Forge::Github,
            host: "github.com".into(),
            rejected: false,
        })
        .context("loading");
        assert_eq!(required(&error).unwrap().host, "github.com");
        assert!(required(&anyhow::anyhow!("other")).is_none());
    }
}
