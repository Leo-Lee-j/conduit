# Session Transcript Lazy-Loading: Phase 2 Checkpoint

Last updated: 2026-11-05

## Scope

Keep transcript bodies out of startup/sidebar state, load a body in the background when a session is selected, bound resident bodies, and preserve full-text search across cold sessions. A cold body must never be mistaken for an empty session or overwritten after a failed read/save.

## Implemented, Not Yet Accepted

- `src/transcript.rs` adds content-addressed JSON body files, SHA-256 validation, metadata references, attachment references, save receipts, and deferred body cleanup.
- `SessionRecord` now carries transcript state. Sidebar metadata, initial sorting, and blank-session deletion checks use summaries without requiring resident bodies.
- `Store` reads v1/v2. Production `Store::load()` preserves the original as `.v2.json`, writes the bodies, then migrates `state.json` to v3. V3 startup loads metadata only. The body is durable before the manifest is replaced.
- `src/app/transcripts.rs` adds asynchronous selection loading, visible error/retry state, a read concurrency limit, and count/estimated-memory-based eviction. The selected, running, queued, and unsaved sessions are pinned.
- History batches load/merge matching existing bodies on a background executor and persist the merged body reference. Failed merges retain the original.
- Search reads cold bodies on a worker, keeps complete match IDs and at most 40 preview hits, supports cancellation, and reports body-read errors.
- Attachment cleanup and body-dependent operations were adapted. The old storage paragraph in `docs/technical-design.md` still needs revision.

## Verification So Far

Integration follow-up (2026-11-05): all 5 transcript UI tests now pass within the full 165-test `app::` regression. The missing-body fixture removes the body before creating the app, then explicitly selects session 2, so an asynchronous startup read cannot finish before deletion. The failure/retry assertions remain intact. Final all-target Clippy with `ui-tests` and formatting checks pass. This resolves the test blocker below; the wider phase-2 review and documentation items remain separate work.

- `cargo fmt --all` ran and the subsequent format check passed.
- `cargo check --locked --bin conduit` and a tests/ui-tests check passed before the latest parallel workspace changes.
- A focused run executed 32 tests: 31 passed. The sole failure was `missing_body_has_error_retry_and_blocks_send_without_looking_empty`: its fixture selected session 1, while the test removed session 2's body and never selected session 2. Explicitly switch to session 2 and await its load before asserting the failure UI.
- Storage, migration, integrity, failed-save, cold-search, import-merge, and eviction tests passed in that run.
- Parallel work subsequently added `src/plugins.rs`, `src/plugins/`, and file-preview code. No final build has been run against this updated worktree. An earlier all-tests check encountered a type error in concurrent `file_preview.rs`; recheck its current state before attributing or changing it.

## Next Session

1. Inspect `git status --short` and the newly added plugin/file-preview files. Preserve concurrent work and confirm those modules are complete.
2. Completed in the integration follow-up: make missing-body setup precede startup reads, select session 2 explicitly, and verify error, send guard, delete confirmation, and retry.
3. Run `cargo fmt --all --check`, `cargo check --locked --bin conduit`, focused store/history tests, and `cargo clippy --locked --all-targets --features ui-tests -- -D warnings`.
4. Review startup selection and async completion, rapid switch/delete/import, import merge, running-session pinning, persistence acknowledgements, cleanup, search completeness, and attachment reachability; then run the relevant UI regressions.
5. Update `docs/technical-design.md` with v3 layout, migration backup, memory limits, and recovery behavior. Review the final diff and verification. No commit has been made.

## Risks to Recheck

- Many existing UI/queue paths read `session.messages` directly. Verify their call paths only operate on loaded sessions.
- A missing/corrupt v3 body reports an error and must never silently reset to an empty conversation. Check all write paths preserve the reference.
- Lease-based content-addressed cleanup may retain old versions temporarily. Verify orphan cleanup after restart and the retention policy for migration backups.
- `Store::load_from` parses without migrating; `Store::load` performs the backed-up v1/v2 upgrade. Confirm all production startup paths call `Store::load`.
- Do not mark phase 2 complete until the updated worktree passes its checks and relevant UI regressions.
