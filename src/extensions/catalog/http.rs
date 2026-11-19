use super::{fail, text};
use anyhow::{Result, ensure};
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) trait Transport {
    fn get(&self, url: &str, limit: usize) -> Result<Vec<u8>>;
    fn check(&self) -> Result<()> {
        Ok(())
    }
    fn json(&self, url: &str, limit: usize) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.get(url, limit)?).map_err(|_| fail("invalid_response"))
    }
}
pub struct Client<'a> {
    agent: ureq::Agent,
    cancel: &'a AtomicBool,
    deadline: Instant,
}
impl<'a> Client<'a> {
    pub fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .timeout_connect(Some(Duration::from_secs(8)))
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .into(),
            cancel,
            deadline: Instant::now() + Duration::from_secs(120),
        }
    }
    pub fn check(&self) -> Result<()> {
        ensure!(!self.cancel.load(Ordering::Acquire), text("cancelled"));
        ensure!(Instant::now() < self.deadline, text("timeout"));
        Ok(())
    }
}
impl Transport for Client<'_> {
    fn check(&self) -> Result<()> {
        Client::check(self)
    }
    fn get(&self, address: &str, limit: usize) -> Result<Vec<u8>> {
        self.check()?;
        let url = url::Url::parse(address).map_err(|_| fail("invalid_source"))?;
        ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443)
                && matches!(
                    url.host_str(),
                    Some(
                        "skills.sh"
                            | "www.skills.sh"
                            | "api.github.com"
                            | "github.com"
                            | "codeload.github.com"
                            | "raw.githubusercontent.com"
                            | "registry.modelcontextprotocol.io"
                    )
                ),
            text("invalid_source")
        );
        let mut response = self
            .agent
            .get(address)
            .header("User-Agent", concat!("Conduit/", env!("CARGO_PKG_VERSION")))
            .header(
                "Accept",
                match url.host_str() {
                    Some("github.com") => "application/x-git-upload-pack-advertisement",
                    Some("www.skills.sh") => "text/html",
                    Some("codeload.github.com" | "raw.githubusercontent.com") => "*/*",
                    _ => "application/json",
                },
            )
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(401) => fail("authentication"),
                ureq::Error::StatusCode(403 | 429) => fail("rate_limit"),
                ureq::Error::StatusCode(404) => fail("not_found"),
                _ => fail("network"),
            })?;
        ensure!(response.status().is_success(), text("network"));
        let mut bytes = Vec::new();
        let mut reader = response.body_mut().as_reader();
        let mut chunk = [0u8; 8192];
        loop {
            self.check()?;
            let size = reader.read(&mut chunk).map_err(|_| fail("network"))?;
            if size == 0 {
                break;
            }
            ensure!(bytes.len() + size <= limit, text("response_limit"));
            bytes.extend_from_slice(&chunk[..size]);
        }
        Ok(bytes)
    }
}
