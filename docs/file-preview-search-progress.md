# File Preview Ctrl+F Progress

Updated: 2026-11-05

## Goal

Add search to the file preview: Ctrl+F toggles the find bar, case-insensitive fuzzy matching is enabled by default, an input suffix icon switches between exact and fuzzy matching, highlights and match counts track the active result, Enter/F3 moves next, Shift+Enter/Shift+F3 moves previous, and Esc closes the bar. A single-line selection should prefill the query. Each document tab should retain its own query and position. Support plain text, Markdown, numbered code blocks, and Mermaid diagrams.

## Completed

- Added `src/app/file_preview.rs` and `src/app/file_preview/search.rs` for per-document preview/find state, shortcuts, debounced background search with stale-result protection, highlights, scrolling, and UTF-8-safe case folding/literal/fuzzy matching.
- Added `src/app/file_preview/tests.rs` with source-preview interaction coverage and Markdown/code/diagram coverage.
- Updated `src/app/files.rs` to delegate preview content to a separate `FilePreview` entity.
- Updated `src/app/numbered_code.rs` and `src/app/typography.rs` to support search highlights and reveal while preserving the existing chat Markdown path.
- Registered shortcuts in `src/app.rs` and documented usage in `docs/usage.md`.
- Earlier preview scrollbar placement, wrapping, and horizontal-overflow fixes remain in the workspace.
- Ctrl+F toggles the find bar open/closed, including while a search is pending. Closing and reopening reuses completed results and restores the active match when the query and source are unchanged; edits, reloads, and cancelled searches invalidate the cached results. Opening focuses/selects the query; closing returns focus to the preview.
- Replaced the outer focus ring with a single 1px focused input border, removed the empty-query status hint, and moved the fuzzy toggle into `Input::suffix` as a compact filter icon with selected/toggled state, an accessible name, and state-specific tooltips. Clicking the suffix keeps typing focus in the input.
- Preserved leading/trailing query whitespace and enforced the same 256-original-character limit in both the matcher and UI, including Unicode case expansions.
- Completed the interaction follow-up with five additional UI tests and two additional matcher tests.

## Verification Status

Continuation (2026-11-05): all 12 file-preview tests pass (5 matcher tests and 7 UI tests). The previously pending interaction checklist is complete:

- Exact/fuzzy toggle, previous/next buttons, wraparound, no-result/empty/oversized query handling, and disabled navigation controls.
- Rapid query changes, reload during a pending search, closing before completion, and reopening after cancellation, in both source and Markdown previews.
- Ctrl+F open/close toggling, active-match preservation after close/reopen, highlight cleanup/restoration, focus return, and replacing the selected query. The UI test also checks suffix placement, absence of the outer focus ring and empty-query status, and input focus after toggling fuzzy matching.
- Single-line selection prefill, ignoring multiline selections, and copying from the search input independently of the document selection.
- Reloading inactive source/Markdown tabs, both with find open and closed, while preserving the foreground tab's query, results, and focus.
- UTF-8 offsets, case-expanding characters, CRLF, exact whitespace, query limits, and exact/truncated result-limit boundaries.

UI refinement verification (2026-11-05):

- All 12 file-preview tests pass with the Ctrl+F toggle and suffix control changes.
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings`, `cargo fmt --all --check`, and the changed-file diff check pass.
- The application regression reports 182 passed and 7 failed. The failures are in the concurrently edited notification-card test (a 0.25px alignment difference) and six right-plugin-view tests (missing view elements); both groups also fail in isolation. Full output is in `target/file-find-toggle-app-tests.log`. The file-preview, file-navigation, Markdown, numbered-code, and Mermaid tests pass.

Regression results before the UI refinement:

- Full main-binary regression: 416 passed, 0 failed, 16 ignored by default. This includes all 180 application tests, covering shared Markdown, numbered-code, Mermaid, and file navigation.
- All 27 library tests and the release-manifest test passed; the updater target has no tests.
- `cargo fmt --all --check`, `cargo clippy --locked --all-targets --features ui-tests -- -D warnings`, and `git diff --check` passed.

The earlier integration checkpoint passed 165 application tests after changing the query helper to use real input events and fixing Mermaid custom-block reveal inside the externally scrolled preview. Its original two failing UI tests are resolved; the coverage remains in this regression.

## Regression Commands

PowerShell commands from the repository root:

```powershell
cargo rustc --locked --bin conduit --features ui-tests --profile test -- -C extra-filename=-file-find-toggle
& .\target\debug\deps\conduit-file-find-toggle.exe app::file_preview:: --test-threads=1 --nocapture
& .\target\debug\deps\conduit-file-find-toggle.exe app:: --test-threads=1
cargo test --locked --lib --bin conduit-updater --bin release-manifest -- --test-threads=1
cargo clippy --locked --all-targets --features ui-tests -- -D warnings
cargo fmt --all --check
git diff --check
```

The workspace contains uncommitted changes from multiple concurrent tasks. Do not reset, revert, or overwrite unrelated files while continuing.
