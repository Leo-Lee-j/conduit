//! Loopback HTTP transport. The desktop entity owns all workspace state.
mod server;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot;

pub(crate) use server::Host;

pub(crate) const PORT: u16 = 47831;
pub(crate) const MAX_TEXT: usize = 256 * 1024;
pub(crate) const PAGE_SIZE: usize = 30;

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn secret() -> std::io::Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut bytes = [0u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(std::io::Error::other("Windows random generator failed"));
    }
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Fault {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
    pub http_status: u16,
}

impl Fault {
    pub fn new(http_status: u16, code: &'static str, message: &'static str) -> Self {
        Self {
            http_status,
            code,
            message,
            retryable: matches!(http_status, 429 | 503),
        }
    }

    pub fn unavailable() -> Self {
        Self::new(
            503,
            "host_unavailable",
            "The desktop is busy or disconnected.",
        )
    }

    pub fn missing() -> Self {
        Self::new(404, "not_found", "This resource is unavailable.")
    }

    pub fn conflict(message: &'static str) -> Self {
        Self::new(409, "state_conflict", message)
    }

    pub fn save_failed() -> Self {
        Self::new(
            500,
            "save_failed",
            "The desktop could not save the change. Check its status for details.",
        )
    }
}

pub(crate) type Reply = Result<Value, Fault>;

pub(crate) enum Read {
    System,
    Workspaces {
        offset: usize,
    },
    Sessions {
        workspace: String,
        archived: bool,
        offset: usize,
    },
    Session(u64),
    Metadata(u64),
    Approvals(u64),
    Messages {
        session: u64,
        before: Option<usize>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataPatch {
    pub title: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

pub(crate) enum Write {
    Create {
        workspace: String,
        engine: String,
    },
    Send {
        session: u64,
        version: String,
        text: String,
    },
    Cancel {
        session: u64,
        run: String,
    },
    Metadata {
        session: u64,
        version: String,
        patch: MetadataPatch,
    },
    Approval {
        session: u64,
        run: String,
        ticket: String,
        action: ApprovalAction,
    },
}

pub(crate) enum ApprovalAction {
    Decide {
        allow: bool,
    },
    Answer {
        answers: BTreeMap<String, Vec<String>>,
    },
}

pub(crate) enum Request {
    Read {
        device: Arc<str>,
        query: Read,
        reply: oneshot::Sender<Reply>,
    },
    Write {
        device: Arc<str>,
        operation: String,
        command: Write,
    },
}
