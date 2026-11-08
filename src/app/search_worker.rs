//! Query work runs on snapshots. Rendering only reads completed results.
use super::search::SearchHit;
use crate::domain::{ChatItemData, ItemKind, SessionRecord, Workspace};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub(super) struct SearchUi {
    pub hits: Vec<SearchHit>,
    pub errors: usize,
    pub cancel: Arc<AtomicBool>,
    pub matches: HashSet<u64>,
    pub result_query: String,
    pub result_generation: u64,
    pub generation: u64,
    pub revision: u64,
    pub requested: Instant,
    pub task: Option<gpui_kit::Task<()>>,
    pub running: bool,
    pub refresh_needed: bool,
    pub requested_query: String,
    pub cache: Cache,
}
impl Default for SearchUi {
    fn default() -> Self {
        Self {
            hits: Vec::new(),
            errors: 0,
            cancel: Arc::default(),
            matches: HashSet::new(),
            result_query: String::new(),
            result_generation: 0,
            generation: 0,
            revision: 0,
            requested: Instant::now(),
            task: None,
            running: false,
            refresh_needed: false,
            requested_query: String::new(),
            cache: Cache::default(),
        }
    }
}

impl Drop for SearchUi {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// KMP over Unicode-lowercased bytes, retaining only one query-sized offset
/// window. A case expansion still maps to the complete original character.
struct Matcher {
    query: Vec<u8>,
    failure: Vec<usize>,
}
impl Matcher {
    fn new(query: &str) -> Self {
        let query = query.to_lowercase().into_bytes();
        let mut failure = vec![0; query.len()];
        let mut matched = 0;
        for i in 1..query.len() {
            while matched > 0 && query[i] != query[matched] {
                matched = failure[matched - 1];
            }
            if query[i] == query[matched] {
                matched += 1;
            }
            failure[i] = matched;
        }
        Self { query, failure }
    }
    fn first(&self, text: &str) -> Option<Range<usize>> {
        if self.query.is_empty() {
            return None;
        }
        let mut positions = VecDeque::with_capacity(self.query.len());
        let mut matched = 0;
        for (index, ch) in text.char_indices() {
            for lower in ch.to_lowercase() {
                let mut encoded = [0; 4];
                for byte in lower.encode_utf8(&mut encoded).bytes() {
                    if positions.len() == self.query.len() {
                        positions.pop_front();
                    }
                    positions.push_back(index);
                    while matched > 0 && byte != self.query[matched] {
                        matched = self.failure[matched - 1];
                    }
                    if byte == self.query[matched] {
                        matched += 1;
                    }
                    if matched == self.query.len() {
                        return Some(*positions.front()?..index + ch.len_utf8());
                    }
                }
            }
        }
        None
    }
}

fn snippet_at(text: &str, found: Option<Range<usize>>) -> String {
    let (start, end) = if let Some(found) = found {
        let start = text[..found.start]
            .char_indices()
            .rev()
            .nth(39)
            .map_or(0, |(i, _)| i);
        let end = text[found.end..]
            .char_indices()
            .nth(120)
            .map_or(text.len(), |(i, _)| found.end + i);
        (start, end)
    } else {
        (
            0,
            text.char_indices().nth(160).map_or(text.len(), |(i, _)| i),
        )
    };
    let mut result = text[start..end].replace(['\r', '\n'], " ");
    if start > 0 {
        result.insert(0, '…');
    }
    if end < text.len() {
        result.push('…');
    }
    result
}
#[cfg(test)]
pub(super) fn snippet(text: &str, query: &str) -> String {
    snippet_at(text, Matcher::new(query).first(text))
}

pub(super) struct Results {
    pub hits: Vec<SearchHit>,
    pub errors: usize,
    pub matches: HashSet<u64>,
    pub versions: HashMap<u64, Version>,
}

impl Results {
    /// Return whether a surviving session needs another search. A first save can
    /// replace an in-memory version with a disk reference without a content revision.
    pub fn retain_current(&mut self, workspaces: &[Workspace]) -> bool {
        let current: HashMap<_, _> = workspaces
            .iter()
            .flat_map(|w| w.sessions.iter().map(move |s| (s.id, (w, s))))
            .collect();
        let mut refresh_needed = false;
        self.matches.retain(|id| {
            current.get(id).is_some_and(|(_, session)| {
                let valid = self
                    .versions
                    .get(id)
                    .is_some_and(|version| version.matches(session));
                refresh_needed |= !valid;
                valid
            })
        });
        self.hits.retain(|hit| self.matches.contains(&hit.session));
        for hit in &mut self.hits {
            let (workspace, session) = current[&hit.session];
            hit.archived = session.archived;
            hit.provider = session.provider;
            hit.workspace_name = workspace.name();
        }
        refresh_needed
    }
}

#[derive(Clone)]
enum BodyVersion {
    Memory(Vec<Weak<ChatItemData>>),
    Disk(Option<crate::transcript::Reference>),
}

#[derive(Clone)]
pub(super) struct Version {
    title: String,
    body: BodyVersion,
}

impl Version {
    fn new(session: &SessionRecord) -> Self {
        Self {
            title: session.title.clone(),
            // The saved reference describes content, independently of whether its
            // messages happen to be resident. Unsaved edits still use exact identities.
            body: if let Some(reference) = session.transcript.saved(&session.messages) {
                BodyVersion::Disk(Some(reference))
            } else if session.transcript.loaded {
                BodyVersion::Memory(session.messages.iter().map(|m| m.identity()).collect())
            } else {
                BodyVersion::Disk(None)
            },
        }
    }
    pub fn matches(&self, session: &SessionRecord) -> bool {
        if self.title != session.title {
            return false;
        }
        match &self.body {
            BodyVersion::Memory(items) => {
                session.transcript.loaded
                    && items.len() == session.messages.len()
                    && items
                        .iter()
                        .zip(&session.messages)
                        .all(|(a, b)| a.ptr_eq(&b.identity()))
            }
            BodyVersion::Disk(Some(reference)) => {
                session.transcript.saved(&session.messages).as_ref() == Some(reference)
            }
            BodyVersion::Disk(None) => {
                !session.transcript.loaded && session.transcript.reference.is_none()
            }
        }
    }
}

struct Cached {
    version: Version,
    matched: bool,
    snippet: Option<String>,
    failed: bool,
}

/// One query, one small result per session; never retains loaded transcript bodies.
#[derive(Default)]
pub(super) struct Cache {
    query: String,
    entries: HashMap<u64, Cached>,
    #[cfg(test)]
    pub scans: usize,
}

#[cfg(test)]
pub(super) fn collect(workspaces: &[Workspace], query: &str) -> Results {
    collect_cancellable(workspaces, query, &AtomicBool::new(false))
}

#[cfg(test)]
pub(super) fn collect_cancellable(
    workspaces: &[Workspace],
    query: &str,
    cancel: &AtomicBool,
) -> Results {
    collect_cached(workspaces, query, cancel, Cache::default()).0
}

pub(super) fn collect_cached(
    workspaces: &[Workspace],
    query: &str,
    cancel: &AtomicBool,
    mut cache: Cache,
) -> (Results, Cache) {
    if cache.query != query {
        cache.entries.clear();
        cache.query = query.to_owned();
    }
    let ids: HashSet<_> = workspaces
        .iter()
        .flat_map(|w| w.sessions.iter().map(|s| s.id))
        .collect();
    cache.entries.retain(|id, _| ids.contains(id));
    let matcher = Matcher::new(query);
    let mut hits = Vec::new();
    let mut matches = HashSet::new();
    let mut errors = 0;
    let mut versions = HashMap::new();
    for workspace in workspaces {
        let name = workspace.name();
        for session in &workspace.sessions {
            if cancel.load(Ordering::Acquire) {
                return (
                    Results {
                        hits,
                        matches,
                        errors,
                        versions,
                    },
                    cache,
                );
            }
            let cached = cache.entries.get(&session.id).filter(|entry| {
                entry.version.matches(session)
                    && !entry.failed
                    && (!entry.matched || entry.snippet.is_some() || hits.len() >= 40)
            });
            if let Some(entry) = cached {
                versions.insert(session.id, entry.version.clone());
                if entry.matched {
                    matches.insert(session.id);
                    if hits.len() < 40 {
                        hits.push(hit(
                            session,
                            &name,
                            entry.snippet.clone().unwrap_or_default(),
                        ));
                    }
                }
                continue;
            }
            #[cfg(test)]
            {
                cache.scans += 1;
            }
            let version = Version::new(session);
            versions.insert(session.id, version.clone());
            let title_match = matcher.first(&session.title).is_some();
            let mut failed = false;
            // No body I/O is needed for title-only matches past the snippet limit.
            let disk_messages;
            let messages = if session.transcript.loaded {
                &session.messages
            } else if (title_match || query.is_empty()) && hits.len() >= 40 {
                matches.insert(session.id);
                cache.entries.insert(
                    session.id,
                    Cached {
                        version,
                        matched: true,
                        snippet: None,
                        failed: false,
                    },
                );
                continue;
            } else {
                disk_messages = match crate::transcript::read(session) {
                    Ok(messages) => messages,
                    Err(_) => {
                        errors += 1;
                        failed = true;
                        Vec::new()
                    }
                };
                &disk_messages
            };
            let message = if !title_match && !query.is_empty() {
                messages
                    .iter()
                    .find_map(|m| matcher.first(&m.text).map(|range| (m, Some(range))))
            } else {
                None
            };
            if !query.is_empty() && !title_match && message.is_none() {
                cache.entries.insert(
                    session.id,
                    Cached {
                        version,
                        matched: false,
                        snippet: None,
                        failed,
                    },
                );
                continue;
            }
            matches.insert(session.id);
            if hits.len() == 40 {
                cache.entries.insert(
                    session.id,
                    Cached {
                        version,
                        matched: true,
                        snippet: None,
                        failed,
                    },
                );
                continue;
            }
            let source = message.or_else(|| {
                messages
                    .iter()
                    .find(|m| m.kind == ItemKind::User)
                    .or_else(|| messages.first())
                    .map(|m| (m, None))
            });
            let snippet = source.map_or_else(String::new, |(m, range)| snippet_at(&m.text, range));
            hits.push(hit(session, &name, snippet.clone()));
            cache.entries.insert(
                session.id,
                Cached {
                    version,
                    matched: true,
                    snippet: Some(snippet),
                    failed,
                },
            );
        }
    }
    (
        Results {
            hits,
            matches,
            errors,
            versions,
        },
        cache,
    )
}

fn hit(session: &SessionRecord, workspace_name: &str, snippet: String) -> SearchHit {
    SearchHit {
        session: session.id,
        provider: session.provider,
        workspace_name: workspace_name.to_owned(),
        title: if session.title.is_empty() {
            conduit::i18n::text_static("files.new_session").into()
        } else {
            session.title.clone()
        },
        archived: session.archived,
        snippet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_search_matches_and_cache_survive_load_unload_and_reload() {
        let _locale = crate::test_support::chinese();
        let fixture = crate::transcript::tests::Fixture::new();
        fixture.store(1).save_to(&fixture.state).unwrap();
        let mut store = crate::store::Store::load_from(&fixture.state).unwrap();
        let cancel = AtomicBool::new(false);
        let (mut results, mut cache) = collect_cached(
            &store.workspaces,
            "original reply",
            &cancel,
            Cache::default(),
        );
        for load in [true, false, true, false] {
            let session = &mut store.workspaces[0].sessions[0];
            if load {
                *session = crate::transcript::load(session.clone()).unwrap();
            } else {
                assert!(crate::transcript::unload(session));
            }
            assert!(!results.retain_current(&store.workspaces));
            assert_eq!(results.hits.len(), 1);
            assert_eq!(results.matches.len(), 1);
            (results, cache) = collect_cached(&store.workspaces, "original reply", &cancel, cache);
            assert_eq!(
                cache.scans, 1,
                "residency must not rescan unchanged content"
            );
        }
        // Loaded snapshots also use the durable version, and reject real unsaved changes.
        store.workspaces[0].sessions[0] =
            crate::transcript::load(store.workspaces[0].sessions[0].clone()).unwrap();
        let mut loaded_results = collect(&store.workspaces, "original reply");
        assert!(crate::transcript::unload(
            &mut store.workspaces[0].sessions[0]
        ));
        assert!(!loaded_results.retain_current(&store.workspaces));
        assert_eq!(loaded_results.hits.len(), 1);
        store.workspaces[0].sessions[0] =
            crate::transcript::load(store.workspaces[0].sessions[0].clone()).unwrap();
        store.workspaces[0].sessions[0].messages[1].text = "changed reply".into();
        assert!(loaded_results.retain_current(&store.workspaces));
        assert!(loaded_results.hits.is_empty());
        assert!(loaded_results.matches.is_empty());
    }

    #[test]
    fn first_save_during_search_requests_retry_without_accepting_stale_content() {
        let _locale = crate::test_support::chinese();
        let fixture = crate::transcript::tests::Fixture::new();
        let mut store = fixture.store(1);
        let mut results = collect(&store.workspaces, "original reply");
        store.save_to(&fixture.state).unwrap();
        assert!(crate::transcript::unload(
            &mut store.workspaces[0].sessions[0]
        ));
        assert!(results.retain_current(&store.workspaces));
        assert!(results.hits.is_empty());
        let mut retried = collect(&store.workspaces, "original reply");
        assert!(!retried.retain_current(&store.workspaces));
        assert_eq!(retried.hits.len(), 1);
        let session = &mut store.workspaces[0].sessions[0];
        *session = crate::transcript::load(session.clone()).unwrap();
        session.messages[1].text = "replaced content".into();
        crate::transcript::save(session, &crate::transcript::directory(&fixture.state)).unwrap();
        assert!(crate::transcript::unload(session));
        assert!(retried.retain_current(&store.workspaces));
        assert!(retried.hits.is_empty());
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn poll_retries_residency_invalidation_without_a_new_content_revision(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "search-residency-retry");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.writable = false;
                app.dirty = false;
                app.search_query = "residency-needle".into();
                app.session_mut(1).unwrap().messages = vec![crate::transcript::tests::message(
                    "user",
                    "residency-needle",
                    1,
                )];
                app.search_ui.revision = app.session_control.global_revision;
                app.search_ui.refresh_needed = true;
                app.search_ui.requested = Instant::now() - std::time::Duration::from_secs(1);
                let generation = app.search_ui.generation;
                app.poll(window, cx);
                assert!(app.search_ui.running);
                assert_eq!(app.search_ui.generation, generation + 1);
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(!app.search_ui.refresh_needed);
            assert!(!app.search_ui.running);
            assert_eq!(app.search_ui.hits.len(), 1);
        });
    }

    #[test]
    fn streaming_rescans_only_changed_session_and_publishes_stable_cold_matches() {
        let _locale = crate::test_support::chinese();
        let fixture = crate::transcript::tests::Fixture::new();
        fixture.store(65).save_to(&fixture.state).unwrap();
        let mut store = crate::store::Store::load_from(&fixture.state).unwrap();
        store.workspaces[0].sessions[0] =
            crate::transcript::load(store.workspaces[0].sessions[0].clone()).unwrap();
        let cancel = AtomicBool::new(false);
        let (_, mut cache) = collect_cached(
            &store.workspaces,
            "original reply",
            &cancel,
            Cache::default(),
        );
        assert_eq!(cache.scans, 65);
        for _ in 0..12 {
            store.workspaces[0].sessions[0].messages[0]
                .text
                .push_str(" delta");
            let (mut results, next) =
                collect_cached(&store.workspaces, "original reply", &cancel, cache);
            cache = next;
            // A live body changes while the snapshot is being searched.
            store.workspaces[0].sessions[0].messages[0]
                .text
                .push_str(" later");
            results.retain_current(&store.workspaces);
            assert_eq!(results.matches.len(), 64);
            assert!(
                !results
                    .matches
                    .contains(&store.workspaces[0].sessions[0].id)
            );
            assert!(!results.hits.is_empty());
        }
        assert_eq!(cache.scans, 77);
        let (mut results, _) = collect_cached(&store.workspaces, "original reply", &cancel, cache);
        store.workspaces[0].sessions.remove(1);
        results.retain_current(&store.workspaces);
        assert_eq!(results.matches.len(), 64);
    }
    #[test]
    fn matching_preserves_unicode_expansion_and_bounds_snippet_allocations() {
        let _locale = crate::test_support::chinese();
        assert_eq!(Matcher::new("i").first("İ"), Some(0..2));
        assert_eq!(Matcher::new("中").first("a中文"), Some(1..4));
        let text = format!("{}needle{}", "前".repeat(100_000), "后".repeat(100_000));
        let preview = snippet(&text, "NEEDLE");
        assert!(preview.contains("needle"));
        assert!(preview.chars().count() <= 168);
        assert!(preview.starts_with('…') && preview.ends_with('…'));
    }
    #[test]
    fn cold_search_is_complete_beyond_snippet_limit_without_warming_sessions() {
        let _locale = crate::test_support::chinese();
        let fixture = crate::transcript::tests::Fixture::new();
        fixture.store(65).save_to(&fixture.state).unwrap();
        let store = crate::store::Store::load_from(&fixture.state).unwrap();
        let results = collect(&store.workspaces, "original reply");
        assert_eq!(results.matches.len(), 65);
        assert_eq!(results.hits.len(), 40);
        assert_eq!(results.errors, 0);
        assert!(
            store.workspaces[0]
                .sessions
                .iter()
                .all(|session| !session.transcript.loaded && session.messages.is_empty())
        );
        assert!(
            collect_cancellable(&store.workspaces, "original", &AtomicBool::new(true))
                .matches
                .is_empty()
        );
        let reference = store.workspaces[0].sessions[0]
            .transcript
            .reference
            .as_ref()
            .unwrap();
        std::fs::remove_file(reference.directory.join(format!("{}.json", reference.hash))).unwrap();
        let results = collect(&store.workspaces, "original reply");
        assert_eq!(results.errors, 65);
        assert!(results.matches.is_empty());
    }
}
