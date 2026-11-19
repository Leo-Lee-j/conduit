use super::{Transport, fail, text, valid_commit};
use anyhow::{Result, ensure};

const MAX_REFS: usize = 8 * 1024 * 1024;

pub(super) fn resolve(client: &impl Transport, repo: &str, revision: &str) -> Result<String> {
    let url = format!("https://github.com/{repo}.git/info/refs?service=git-upload-pack");
    let bytes = client.get(&url, MAX_REFS)?;
    select(client, &bytes, revision)
}

// Git smart HTTP v0 advertises the public refs without using the GitHub REST quota.
fn select(client: &impl Transport, mut bytes: &[u8], revision: &str) -> Result<String> {
    let candidates = if revision == "HEAD" || revision.starts_with("refs/") {
        vec![format!("{revision}^{{}}"), revision.to_owned()]
    } else {
        vec![
            format!("refs/heads/{revision}"),
            format!("refs/tags/{revision}^{{}}"),
            format!("refs/tags/{revision}"),
        ]
    };
    let mut best = None;
    while !bytes.is_empty() {
        client.check()?;
        let size = bytes
            .get(..4)
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|hex| usize::from_str_radix(hex, 16).ok())
            .ok_or_else(|| fail("invalid_response"))?;
        if size == 0 {
            bytes = &bytes[4..];
            continue;
        }
        ensure!((4..=bytes.len()).contains(&size), text("invalid_response"));
        let (packet, rest) = bytes.split_at(size);
        bytes = rest;
        let line = packet[4..]
            .split(|b| *b == b'\0' || *b == b'\n')
            .next()
            .unwrap_or_default();
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        let Some((sha, name)) = line.split_once(' ') else {
            continue;
        };
        if let Some(rank) = candidates.iter().position(|candidate| candidate == name)
            && valid_commit(sha)
        {
            if rank == 0 {
                return Ok(sha.to_ascii_lowercase());
            }
            if best.is_none_or(|(previous, _)| rank < previous) {
                best = Some((rank, sha));
            }
        }
    }
    best.map(|(_, sha)| sha.to_ascii_lowercase())
        .ok_or_else(|| fail("not_found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::catalog::tests::Fake;

    #[test]
    fn handles_slash_refs_and_peels_annotated_tags() {
        let branch = "a".repeat(40);
        let tag = "b".repeat(40);
        let peeled = "c".repeat(40);
        let mut data = String::from("001e# service=git-upload-pack\n0000");
        for (sha, reference) in [
            (&branch, "refs/heads/feature/test"),
            (&tag, "refs/tags/v1"),
            (&peeled, "refs/tags/v1^{}"),
        ] {
            let packet = format!("{sha} {reference}\n");
            data.push_str(&format!("{:04x}{packet}", packet.len() + 4));
        }
        data.push_str("0000");
        let client = Fake::new();
        assert_eq!(
            select(&client, data.as_bytes(), "feature/test").unwrap(),
            branch
        );
        assert_eq!(select(&client, data.as_bytes(), "v1").unwrap(), peeled);
        assert!(select(&client, b"0100incomplete", "HEAD").is_err());
    }
}
