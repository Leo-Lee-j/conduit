# File Preview Ctrl+F Progress

Updated: 2026-11-05

## Goal

Add search to the file preview: case-insensitive fuzzy matching by default, an exact-match toggle, highlights and match counts, scrolling to the active result, Enter/F3 next, Shift+Enter/Shift+F3 previous, and Esc to close. A single-line selection should prefill the query. Each document tab should retain its own query and position. Support plain text, Markdown, numbered code blocks, and Mermaid diagrams.

## Completed

- Added `src/app/file_preview.rs` and `src/app/file_preview/search.rs` for per-document preview/find state, shortcuts, debounced background search with stale-result protection, highlights, scrolling, and UTF-8-safe case folding/literal/fuzzy matching.
- Added `src/app/file_preview/tests.rs` with source-preview interaction coverage and Markdown/code/diagram coverage.
- Updated `src/app/files.rs` to delegate preview content to a separate `FilePreview` entity.
- Updated `src/app/numbered_code.rs` and `src/app/typography.rs` to support search highlights and reveal while preserving the existing chat Markdown path.
- Registered shortcuts in `src/app.rs` and documented usage in `docs/usage.md`.
- Earlier preview scrollbar placement, wrapping, and horizontal-overflow fixes remain in the workspace.

## Verification Status

- A separately named test binary built successfully to avoid contention with another session's locked test binary.
- All 3 matcher unit tests passed.
- All 6 existing file-preview/navigation tests passed, including copy, wrapping, Markdown, and tab switching.
- The 2 new UI tests are not yet passing. The current known cause is the test helper in `src/app/file_preview/tests.rs`: it uses `InputState::set_value`, which suppresses the Change event, so search is not triggered. First change the helper to simulate real typing or use an API that emits the normal change event, then rerun. The current failures do not yet establish a production Markdown bug.

## Next Steps

1. Fix the `query()` test helper in `src/app/file_preview/tests.rs`, rerun both new UI tests, and diagnose any remaining failures.
2. Verify exact/fuzzy toggle, stale results after rapid typing, repeated Ctrl+F, selected-text prefill, source reload while a tab is inactive, highlight cleanup/reopen, and focus behavior.
3. Run relevant shared Markdown, numbered-code, and Mermaid tests.
4. Run formatting, Clippy, and diff checks. Preserve unrelated concurrent worktree changes.

PowerShell commands from the repository root:

```powershell
cargo rustc --locked --bin conduit --features ui-tests --profile test -- -C extra-filename=-file-find-regression
& .\target\debug\deps\conduit-file-find-regression.exe app::file_preview:: --test-threads=1 --nocapture
& .\target\debug\deps\conduit-file-find-regression.exe app::files::tests:: --test-threads=1 --nocapture
```

The workspace contains uncommitted changes from multiple concurrent tasks. Do not reset, revert, or overwrite unrelated files while continuing.
