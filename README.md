<p align="center">
  <img src="assets/app-icons/128x128@2x.png" alt="Conduit logo" width="128" height="128">
</p>

<h1 align="center">Conduit</h1>

<p align="center">
  A native Windows workspace for AI coding agents.<br>
  Conversations, files, Git, and a terminal — together in one app.
</p>

<p align="center">
  <strong>English</strong> · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/Leo-Lee-j/conduit/releases"><img src="https://img.shields.io/github/v/release/Leo-Lee-j/conduit?color=20cbb5" alt="GitHub release"></a>
  <a href="https://github.com/Leo-Lee-j/conduit/actions/workflows/ci.yml"><img src="https://github.com/Leo-Lee-j/conduit/actions/workflows/ci.yml/badge.svg" alt="Windows CI"></a>
  <img src="https://img.shields.io/badge/platform-Windows_x64-0078D4" alt="Windows x64">
</p>

<p align="center">
  <a href="https://github.com/Leo-Lee-j/conduit/releases/latest"><strong>Download for Windows</strong></a> ·
  <a href="docs/usage.md">User guide (中文)</a> ·
  <a href="https://github.com/Leo-Lee-j/conduit/issues">Report an issue</a> ·
  <a href="CONTRIBUTING.md">Contribute</a>
</p>

Conduit brings **Claude Code, Codex, OpenCode, and Qoder CLI** into a shared desktop workspace. Keep conversations organized by project, follow tool activity, inspect changes, and commit your work without losing track of the task.

Conduit is under active development and currently supports **Windows x64**. The interface supports **English and 简体中文**, switchable in **Settings → General → Language**. New installations default to English; existing installations retain Chinese until you choose another language. Detailed guides are currently in Chinese.

## Why Conduit?

- **Choose your agent.** Use Claude Code, Codex, OpenCode, or Qoder CLI, with streaming replies, tool approvals, cancellation, and per-session model settings.
- **Keep projects organized.** Run sessions in parallel, search your history, pin important conversations, and resume native CLI sessions. Existing Claude Code and Codex terminal history is imported by workspace.
- **Review what changed.** Browse files with syntax highlighting, read Markdown and Mermaid diagrams, and inspect unified or side-by-side diffs from a conversation turn. File and diff previews are currently read-only.
- **Take changes through Git.** Stage selected files, manage branches, generate commit messages, commit, and push. Commits include only the staged changes in the files you select.
- **Keep working while an agent runs.** Queue a follow-up message, switch sessions, or use workspace terminal tabs with PowerShell, cmd, and terminal programs.
- **Make the workspace yours.** Customize themes, fonts, and keyboard shortcuts. Follow approvals and completed tasks in the notification center, and install app updates from within Conduit.

## Get started

### What you need

- **Windows x64** for the current desktop release.
- At least one supported CLI — **Claude Code, Codex, OpenCode, or Qoder CLI** — installed and authenticated. Confirm it works in a terminal first.
- **Git** installed if you want to use the Git features.

Conduit uses the selected CLI's authentication and execution capabilities. It does not include a model service; model access and usage are handled through your configured CLI and provider.

### Install and start a conversation

1. Download `Conduit-<version>-windows-x64-setup.exe` from [GitHub Releases](https://github.com/Leo-Lee-j/conduit/releases/latest) and run the installer.
2. Open Conduit and add a local project folder as a workspace.
3. Select an engine and model. If Conduit cannot find your CLI, set its executable path in Settings.
4. Describe your task and press **Enter**. Use **Shift+Enter** for a new line, and type **@** to attach a workspace file.

Review requested tool actions as they appear, inspect the resulting changes, and use the Git panel when you are ready to commit. See the [user guide (中文)](docs/usage.md) for configuration and daily workflows.

## Your workspace and data

Conduit stores conversation history, tool output, and settings locally in `%LOCALAPPDATA%\Conduit`. The selected CLI handles model requests; Conduit also makes requests for features such as model discovery and update checks. Local storage does not mean that model execution is offline.

Sessions can run concurrently. Sessions in the same folder share its files; use separate Git worktrees when tasks need isolated working copies.

## Documentation

The English and [Chinese](README.zh-CN.md) project pages cover the same essentials. Detailed guides are currently in Chinese.

| Guide | What you will find |
| --- | --- |
| [User guide](docs/usage.md) | Setup, conversations, shortcuts, and updates |
| [Settings and AI connections](docs/settings.md) | CLI paths, model providers, Skills, and MCP |
| [Plugins and icon themes](docs/plugins/github-catalog.md) | GitHub discovery, installation, and official plugins |
| [Files and previews](docs/files.md) | File references, search, Markdown, and Mermaid |
| [Git workflows](docs/git.md) | Staging, commits, branches, and recovery |
| [Integrated terminal](docs/terminal.md) | Workspace shells and terminal behavior |
| [All documentation](docs/README.md) | User guides, architecture, development, and releases |

## Contributing

Bug reports, documentation improvements, and pull requests are welcome. [Open an issue](https://github.com/Leo-Lee-j/conduit/issues) with your Conduit version, steps to reproduce, and expected and actual behavior. Discuss substantial features or architecture changes before implementing them.

To build from source, follow [CONTRIBUTING.md](CONTRIBUTING.md) for the Windows Rust toolchain, build commands, and required checks. The [architecture guide](docs/architecture.md) describes how the project fits together.

## Acknowledgements

Built with **Rust**, **GPUI**, and [GPUI Kit](https://github.com/longbridge/gpui-kit). Thanks to the projects behind Claude Code, Codex, OpenCode, and Qoder CLI, and to [LobeHub Icons](https://github.com/lobehub/lobe-icons) for the engine and model marks.

See [application icon generation](assets/app-icons/README.md) and [third-party icon attribution](assets/icons/README.md) for asset details.

## License

This repository does not currently include a project-level `LICENSE` file. Third-party icon license notices are included with the [icon assets](assets/icons/README.md).
