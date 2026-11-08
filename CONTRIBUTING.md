# Contributing to Conduit

**English** · [简体中文](CONTRIBUTING.zh-CN.md) · [Project home](README.md)

Thanks for helping improve Conduit. Contributions can be code, bug reports, documentation, translations, or feedback from using the app. You do not need a Rust development environment to improve the docs or report a problem.

## Find a way to contribute

- **Report a bug:** search [existing issues](https://github.com/Leo-Lee-j/conduit/issues) first, then open an issue with a reproducible example.
- **Suggest a feature:** describe the task you want to accomplish and what is missing from the current workflow. Discuss substantial features, new dependencies, or architecture changes before starting implementation.
- **Improve the docs:** fix unclear steps, broken links, or outdated descriptions. Keep the English and Chinese project and contribution guides in sync when shared content changes.
- **Send a fix:** choose a focused problem, add an appropriate regression test, and explain the resulting behavior in your pull request.

## Write a useful issue

For a bug report, include:

- Your Conduit version, Windows version, and the affected CLI and its version, if relevant.
- The steps to reproduce, what you expected, and what happened.
- Relevant error text, a short log excerpt, or a screenshot. For visual issues, include your display scaling and theme.
- Whether you can reproduce it in a small example workspace.

Remove credentials, private code, and conversation data before sharing logs or screenshots. A complete data-directory dump is not needed for a useful report.

## Set up a development environment

Conduit currently targets **Windows x64**. For Rust changes, you need:

- Git, Rust with the MSVC toolchain, and the `rustfmt` and `clippy` components.
- Visual Studio Build Tools with the C++ build tools and Windows SDK.
- The Rust toolchain version used by [Windows CI](.github/workflows/ci.yml). Use that workflow as the source of truth; dependency versions are pinned by `Cargo.lock` and the GPUI Kit revision in `Cargo.toml`.

Install and authenticate Claude Code, Codex, or OpenCode only when manually testing that integration. Tests that require installed CLIs or real accounts are opt-in; see the [testing guide (中文)](docs/testing.md).

### Clone and run

Fork the repository if you do not have write access. The commands below clone upstream; substitute your fork's URL when contributing from a fork. Run them in PowerShell:

```powershell
git clone https://github.com/Leo-Lee-j/conduit.git
cd conduit
git switch -c fix/describe-your-change
$env:CONDUIT_DATA_DIR = Join-Path (Get-Location).Path 'target/dev-data'
cargo run --locked
```

`CONDUIT_DATA_DIR` keeps this development instance's Conduit data separate from your everyday installation. It does not isolate the selected CLI's configuration or the files in an opened workspace. Use a disposable project or a separate Git worktree when testing file changes or Git operations.

To open Settings directly, keep the same data-directory setting and run:

```powershell
cargo run --locked -- --settings
```

The environment variable applies to commands launched from the current PowerShell session. Keep the terminal and its output available while investigating startup failures.

## Make a focused change

Follow the conventions of the module you are editing. Keep a pull request focused on one problem or a related set of changes, and avoid unrelated formatting or dependency updates.

Put regression tests in the existing behavior module or a neighboring `tests.rs`. Test the behavior that could break, including relevant failure or cancellation paths. Keep explicit integration tests and performance probes separate from ordinary regression tests.

Update the relevant documentation when behavior changes. The [architecture guide (中文)](docs/architecture.md) is a useful starting point for finding the responsible module.

## Validate your changes

Run commands from the repository root. While developing, start with tests for the affected module; the [testing guide (中文)](docs/testing.md) lists filters and manual checks.

Before submitting Rust changes, run:

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo build --locked --bin conduit --bin conduit-updater
git diff --check
```

For changes to the application UI, also run:

```powershell
cargo clippy --locked --all-targets --features ui-tests -- -D warnings
cargo test --locked --features ui-tests --bin conduit app:: -- --test-threads=1
```

These checks match the build and test coverage in [CI](.github/workflows/ci.yml). Review the test summary: a filter that runs zero tests does not verify a change. Report failures, skipped tests, interruptions, and untested areas accurately. Do not enable every ignored test with a blanket `--ignored`; choose explicitly any test that needs a real account, model usage, or extra software.

For UI changes, exercise the affected flow in the native app and check the relevant themes, window sizes, DPI settings, and keyboard behavior. Attach screenshots when they help explain the change; automated tests do not replace visual verification.

For **documentation-only changes**, check links, code examples, language navigation, and consistency with the repository, then run `git diff --check`. No Cargo build or test run is required.

## Keep documentation useful

The [documentation index (中文)](docs/README.md) links to user guides, development and release information, and experimental extensions.

- Maintain one current guide per topic and language. Put detailed commands in the guide responsible for that workflow, and link to it from other pages.
- Keep English and Chinese project and contribution guides aligned. Update language links and remove outdated language labels when translating a page.
- Describe what is implemented today. Label proposals and incomplete validation clearly, and remove obsolete constraints after a fix.
- Prefer relative links for repository content. When moving a file, update incoming links, image references, and related script comments.
- Keep reusable methods and constraints in the docs. Save individual test logs, screenshots, and benchmark output under ignored `target/` or tool artifact directories.
- Preserve asset source, generation, and license information in the resource directory. Do not commit secrets, raw conversations, or build output.

Before cleaning temporary files, confirm their ownership and whether they are in use. Manage registered worktrees with Git.

## Open a pull request

Push your branch to your fork or an authorized repository branch, then open a pull request against `main`.

A useful pull request description includes:

- **Problem:** the trigger and the behavior that needs to change; link the issue if there is one.
- **Result:** what users will experience, with important implementation choices or compatibility concerns.
- **Validation:** the commands and manual steps actually performed, their results, and any remaining gaps.
- **Documentation or visuals:** updated guides and screenshots where relevant.

Keep the title and description aligned with the final patch. When review leads to more changes, rerun the affected checks and update the validation notes. Do not report an earlier revision's test results as verification of new changes.

## Further reading

Detailed guides are currently in Chinese:

- [Architecture](docs/architecture.md): modules, product boundaries, and data flow.
- [Testing and acceptance](docs/testing.md): targeted regressions and native Windows checks.
- [Performance](docs/performance.md): measurement methods, bounded work, and caches.
- [Releases and updates](docs/releasing.md): packaging, signatures, and recovery.
- [Plugin development](docs/plugins/README.md): supported contributions and experimental runtime work.

## Translations

See the [internationalization guide](docs/internationalization.md) for adding languages and interface text. Register a locale in `assets/languages.json`, translate its catalog in `locales/`, and run `python scripts/check-i18n.py`. Keep message keys and named parameters stable. Verify live switching without losing drafts, and check narrow layouts in both languages.
