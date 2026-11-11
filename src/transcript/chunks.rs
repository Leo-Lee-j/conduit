//! Immutable pages of 64 messages and 64 KiB text blocks. A root is published
//! only after every child is durable; old roots remain independently readable.
use super::*;
use std::collections::HashSet;

const PAGE_MESSAGES: usize = 64;
const CHUNK_BYTES: usize = 64 * 1024;
const INLINE_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Text {
    Inline(String),
    Chunks { bytes: usize, hashes: Vec<String> },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Message {
    metadata: ChatItemData,
    text: Text,
    input: Text,
    detail: Text,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "format")]
enum Body {
    #[serde(rename = "conduit-transcript-pages-v1")]
    Pages { pages: Vec<String> },
    #[serde(rename = "conduit-transcript-messages-v1")]
    Messages { messages: Vec<Arc<Message>> },
}

pub(super) struct Prepared {
    pub reference: Reference,
    pub parts: Vec<Arc<Message>>,
    pub pages: Vec<String>,
    // Pins staged children until the root has a lease and is durably published.
    _guards: Vec<Arc<Lease>>,
}

struct Writer<'a> {
    directory: &'a Path,
    known: HashSet<String>,
    guards: Vec<Arc<Lease>>,
}

impl Writer<'_> {
    fn blob(&mut self, bytes: &[u8], extension: &str) -> Result<String> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        if extension == "chunk" && self.known.contains(&hash) {
            return Ok(hash);
        }
        let destination = self.directory.join(format!("{hash}.{extension}"));
        self.guards.push(acquire(destination.clone()));
        publish(bytes, &hash, &destination)?;
        if extension == "chunk" {
            self.known.insert(hash.clone());
        }
        Ok(hash)
    }

    fn text(&mut self, text: &str) -> Result<Text> {
        if text.len() <= INLINE_BYTES {
            return Ok(Text::Inline(text.into()));
        }
        let hashes = text
            .as_bytes()
            .chunks(CHUNK_BYTES)
            .map(|chunk| self.blob(chunk, "chunk"))
            .collect::<Result<Vec<_>>>()?;
        Ok(Text::Chunks {
            bytes: text.len(),
            hashes,
        })
    }

    fn message(&mut self, item: &ChatItem) -> Result<Arc<Message>> {
        // Copy metadata explicitly so large input/output strings are never cloned here.
        let metadata = ChatItemData {
            reasoning_summary: item.reasoning_summary,
            key: item.key.clone(),
            kind: item.kind.clone(),
            created_at: item.created_at,
            elapsed_ms: item.elapsed_ms,
            status: item.status.clone(),
            approval: item.approval.clone(),
            model: item.model.clone(),
            text: String::new(),
            input: String::new(),
            detail: String::new(),
        };
        Ok(Arc::new(Message {
            metadata,
            text: self.text(&item.text)?,
            input: self.text(&item.input)?,
            detail: self.text(&item.detail)?,
        }))
    }
}

pub(super) fn prepare(
    session: &SessionRecord,
    directory: &Path,
    previous: Option<&Receipt>,
) -> Result<Prepared> {
    fs::create_dir_all(directory)?;
    let previous = previous.filter(|r| r.reference.directory == directory);
    let mut writer = Writer {
        directory,
        known: HashSet::new(),
        guards: Vec::new(),
    };
    let mut reusable = HashMap::new();
    if let Some(previous) = previous {
        for (identity, part) in previous.messages.iter().zip(&previous.parts) {
            reusable.insert(identity.as_ptr(), part);
            for text in [&part.text, &part.input, &part.detail] {
                if let Text::Chunks { hashes, .. } = text {
                    writer.known.extend(hashes.iter().cloned());
                }
            }
        }
    }
    let parts = session
        .messages
        .iter()
        .map(|item| {
            reusable
                .get(&item.identity().as_ptr())
                .map_or_else(|| writer.message(item), |part| Ok((*part).clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut page_hashes = Vec::new();
    let root = if parts.len() <= PAGE_MESSAGES {
        Body::Messages {
            messages: parts.clone(),
        }
    } else {
        let pages = parts
            .chunks(PAGE_MESSAGES)
            .enumerate()
            .map(|(index, page)| {
                let start = index * PAGE_MESSAGES;
                if let Some(previous) = previous
                    && let Some(hash) = previous.pages.get(index)
                    && previous
                        .parts
                        .len()
                        .saturating_sub(start)
                        .min(PAGE_MESSAGES)
                        == page.len()
                    && previous
                        .messages
                        .get(start..start + page.len())
                        .is_some_and(|identities| {
                            identities
                                .iter()
                                .zip(&session.messages[start..start + page.len()])
                                .all(|(a, b)| a.ptr_eq(&b.identity()))
                        })
                {
                    return Ok(hash.clone());
                }
                let bytes = serde_json::to_vec(&Body::Messages {
                    messages: page.to_vec(),
                })?;
                writer.blob(&bytes, "json")
            })
            .collect::<Result<Vec<_>>>()?;
        page_hashes = pages.clone();
        Body::Pages { pages }
    };
    let bytes = serde_json::to_vec(&root)?;
    let hash = writer.blob(&bytes, "json")?;
    let reference = Reference {
        hash: hash.clone(),
        message_count: parts.len(),
        last_activity_at: session
            .messages
            .iter()
            .rev()
            .find(|m| m.created_at > 0)
            .map_or(0, |m| m.created_at),
        has_user: session.messages.iter().any(|m| m.kind == ItemKind::User),
        bytes: resident_bytes(&session.messages),
        attachments: attachment_paths(&session.messages),
        directory: directory.to_owned(),
        lease: Some(acquire(directory.join(format!("{hash}.json")))),
    };
    Ok(Prepared {
        reference,
        parts,
        pages: page_hashes,
        _guards: writer.guards,
    })
}

fn checked(directory: &Path, hash: &str, extension: &str) -> Result<Vec<u8>> {
    validate_hash(hash)?;
    let path = directory.join(format!("{hash}.{extension}"));
    let bytes = fs::read(&path).with_context(|| {
        conduit::i18n::format(
            "transcript.chunks.unable_to_read_session_messages",
            &[("arg0", format!("{}", path.display()))],
        )
    })?;
    if format!("{:x}", Sha256::digest(&bytes)) != hash {
        bail!(conduit::i18n::format(
            "transcript.chunks.session_message_validation_failed",
            &[("arg0", format!("{}", path.display()))]
        ));
    }
    Ok(bytes)
}

fn read_text(directory: &Path, text: &Text) -> Result<String> {
    match text {
        Text::Inline(text) => Ok(text.clone()),
        Text::Chunks { bytes, hashes } => {
            if hashes.len() != bytes.div_ceil(CHUNK_BYTES) {
                bail!(conduit::i18n::text(
                    "transcript.chunks.session_message_chunk_count_mismatch"
                ));
            }
            let mut output = Vec::new();
            for (index, hash) in hashes.iter().enumerate() {
                let chunk = checked(directory, hash, "chunk")?;
                let expected = if index + 1 == hashes.len() {
                    bytes - index * CHUNK_BYTES
                } else {
                    CHUNK_BYTES
                };
                if chunk.len() != expected {
                    bail!(conduit::i18n::text(
                        "transcript.chunks.session_message_chunk_length_mismatch"
                    ));
                }
                output.extend_from_slice(&chunk);
            }
            Ok(
                String::from_utf8(output).context(conduit::i18n::text_static(
                    "transcript.chunks.session_message_character_encoding_is_corrupt",
                ))?,
            )
        }
    }
}

pub(super) struct Loaded {
    pub messages: Vec<ChatItem>,
    pub parts: Vec<Arc<Message>>,
    pub pages: Vec<String>,
}

pub(super) fn read(reference: &Reference) -> Result<Loaded> {
    let bytes = checked(&reference.directory, &reference.hash, "json")?;
    // v3 legacy roots remain readable; their next modification writes the new format.
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'[') {
        let messages: Vec<ChatItem> = serde_json::from_slice(&bytes)?;
        if messages.len() != reference.message_count {
            bail!(conduit::i18n::text(
                "transcript.chunks.session_messages_do_not_match_the_summary"
            ));
        }
        return Ok(Loaded {
            messages,
            parts: Vec::new(),
            pages: Vec::new(),
        });
    }
    let mut page_hashes = Vec::new();
    let parts = match serde_json::from_slice::<Body>(&bytes)? {
        Body::Messages { messages } => messages,
        Body::Pages { pages } => {
            page_hashes = pages.clone();
            if pages.len() != reference.message_count.div_ceil(PAGE_MESSAGES) {
                bail!(conduit::i18n::text(
                    "transcript.chunks.session_message_page_count_mismatch"
                ));
            }
            let mut parts = Vec::new();
            for (index, hash) in pages.iter().enumerate() {
                let bytes = checked(&reference.directory, hash, "json")?;
                let Body::Messages { messages } = serde_json::from_slice(&bytes)? else {
                    bail!(conduit::i18n::text(
                        "transcript.chunks.invalid_session_message_page_type"
                    ));
                };
                let expected = if index + 1 == pages.len() {
                    reference.message_count - index * PAGE_MESSAGES
                } else {
                    PAGE_MESSAGES
                };
                if messages.len() != expected {
                    bail!(conduit::i18n::text(
                        "transcript.chunks.session_message_page_length_mismatch"
                    ));
                }
                parts.extend(messages);
            }
            parts
        }
    };
    if parts.len() != reference.message_count {
        bail!(conduit::i18n::text(
            "transcript.chunks.session_messages_do_not_match_the_summary"
        ));
    }
    let messages = parts
        .iter()
        .map(|part| {
            let mut item = part.metadata.clone();
            item.text = read_text(&reference.directory, &part.text)?;
            item.input = read_text(&reference.directory, &part.input)?;
            item.detail = read_text(&reference.directory, &part.detail)?;
            Ok(item.into())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Loaded {
        messages,
        parts,
        pages: page_hashes,
    })
}

/// Called with the lease registry locked. Fail closed if a live root is missing
/// or damaged: collection must not destroy children needed for later recovery.
pub(super) fn reachable(
    directory: &Path,
    roots: impl Iterator<Item = PathBuf>,
) -> Result<HashSet<PathBuf>> {
    let mut marked = HashSet::new();
    let mut pending: Vec<_> = roots.collect();
    while let Some(path) = pending.pop() {
        if !marked.insert(path.clone()) || path.extension().is_some_and(|e| e == "chunk") {
            continue;
        }
        let hash =
            path.file_stem()
                .and_then(|s| s.to_str())
                .context(conduit::i18n::text_static(
                    "transcript.chunks.invalid_message_path",
                ))?;
        let bytes = checked(directory, hash, "json")?;
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'[') {
            continue;
        }
        match serde_json::from_slice::<Body>(&bytes)? {
            Body::Pages { pages } => {
                for hash in pages {
                    validate_hash(&hash)?;
                    pending.push(directory.join(format!("{hash}.json")));
                }
            }
            Body::Messages { messages } => {
                for message in messages {
                    for text in [&message.text, &message.input, &message.detail] {
                        if let Text::Chunks { hashes, .. } = text {
                            for hash in hashes {
                                validate_hash(hash)?;
                                marked.insert(directory.join(format!("{hash}.chunk")));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(marked)
}
