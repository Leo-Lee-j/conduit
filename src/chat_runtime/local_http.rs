//! Bounded HTTP JSON request reader for opt-in local model fixtures.
use serde_json::Value;
use std::{io::Read, net::TcpStream, time::Duration};

pub(super) fn read_request(socket: &mut TcpStream) -> Option<Value> {
    socket.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut bytes = vec![];
    let mut buffer = [0_u8; 8192];
    loop {
        let count = socket.read(&mut buffer).ok()?;
        if count == 0 || bytes.len() > 2 * 1024 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..count]);
        let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else {
            continue;
        };
        let header = String::from_utf8_lossy(&bytes[..index]);
        let size: usize = header
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .map(str::to_owned)
            })?
            .parse()
            .ok()?;
        if bytes.len() >= index + 4 + size {
            return serde_json::from_slice(&bytes[index + 4..index + 4 + size]).ok();
        }
    }
}
