<p align="center">
  <img src="assets/app-icons/128x128@2x.png" alt="Conduit 标志" width="128" height="128">
</p>

<h1 align="center">Conduit</h1>

<p align="center">
  面向 AI 编程助手的 Windows 原生工作台。<br>
  在同一个应用中管理对话、文件、Git 与终端。
</p>

<p align="center">
  <a href="README.md">English</a> · <strong>简体中文</strong>
</p>

<p align="center">
  <a href="https://github.com/Leo-Lee-j/conduit/releases"><img src="https://img.shields.io/github/v/release/Leo-Lee-j/conduit?color=20cbb5" alt="GitHub 发布版本"></a>
  <a href="https://github.com/Leo-Lee-j/conduit/actions/workflows/ci.yml"><img src="https://github.com/Leo-Lee-j/conduit/actions/workflows/ci.yml/badge.svg" alt="Windows CI"></a>
  <img src="https://img.shields.io/badge/platform-Windows_x64-0078D4" alt="Windows x64">
</p>

<p align="center">
  <a href="https://github.com/Leo-Lee-j/conduit/releases/latest"><strong>下载 Windows 版</strong></a> ·
  <a href="docs/usage.md">使用指南</a> ·
  <a href="https://github.com/Leo-Lee-j/conduit/issues">反馈问题</a> ·
  <a href="CONTRIBUTING.zh-CN.md">参与贡献</a>
</p>

Conduit 将 **Claude Code、Codex 和 OpenCode** 带入同一个桌面工作区。按项目组织对话，跟进工具执行，检查代码修改，再完成提交，让任务从开始到交付都有清晰的上下文。

项目正在持续开发，目前支持 **Windows x64**。应用界面与详细文档以中文为主。

## 为什么使用 Conduit？

- **自由选择编程助手。** 使用 Claude Code、Codex 或 OpenCode，支持流式回复、工具审批、取消任务，以及按会话选择模型。
- **让项目与对话井然有序。** 并行运行多个会话，搜索历史，置顶重要对话，并续接原生 CLI 会话。已有的 Claude Code 和 Codex 终端历史会按工作区导入。
- **看清每一次修改。** 浏览带语法高亮的文件，阅读 Markdown 与 Mermaid 图表，按轮次查看统一或并排 Diff。文件与 Diff 预览目前为只读。
- **在工作台完成 Git 流程。** 勾选文件暂存、管理分支、生成提交说明、提交与推送。提交只包含所选文件中已暂存的修改。
- **等待回复时继续工作。** 排队下一条消息、切换会话，或在工作区终端中运行 PowerShell、cmd 和终端程序。
- **调整为自己的使用习惯。** 自定义主题、字体和快捷键，通过通知中心跟进审批与任务结果，并在应用内安装更新。

## 开始使用

### 准备条件

- 当前桌面版需要 **Windows x64**。
- 安装并登录至少一个受支持的 CLI：**Claude Code、Codex 或 OpenCode**，先确认它可以在终端中正常运行。
- 如需使用 Git 功能，请在本机安装 **Git**。

Conduit 复用所选 CLI 的认证与执行能力，不自带模型服务。模型访问与用量由你配置的 CLI 和模型服务商处理。

### 安装并开始对话

1. 在 [GitHub Releases](https://github.com/Leo-Lee-j/conduit/releases/latest) 下载 `Conduit-<version>-windows-x64-setup.exe`，运行安装包。
2. 打开 Conduit，将本地项目文件夹添加为工作区。
3. 选择引擎和模型。如果未找到 CLI，可在设置中指定它的可执行文件路径。
4. 输入任务，按 **Enter** 发送；使用 **Shift+Enter** 换行，输入 **@** 引用工作区文件。

按提示处理工具审批，检查任务产生的修改，准备好后在 Git 面板中提交。配置方法与日常操作见[使用指南](docs/usage.md)。

## 工作区与数据

对话历史、工具输出和设置保存在本地 `%LOCALAPPDATA%\Conduit`。模型请求由所选 CLI 处理；模型目录获取、应用更新检查等功能也会发起网络请求。本地保存数据不代表模型离线运行。

多个会话可以并行执行。同一文件夹中的会话共享工作区文件；需要隔离任务时，请使用不同的 Git worktree。

## 文档

[英文首页](README.md)与本页提供相同的核心信息，详细指南目前以中文维护。

| 指南 | 内容 |
| --- | --- |
| [使用指南](docs/usage.md) | 配置、对话、快捷键与更新 |
| [设置与 AI 连接](docs/settings.md) | CLI 路径、模型服务、Skills 与 MCP |
| [文件与内容预览](docs/files.md) | 文件引用、搜索、Markdown 与 Mermaid |
| [Git 工作流](docs/git.md) | 暂存、提交、分支与失败恢复 |
| [集成终端](docs/terminal.md) | 工作区 Shell 与终端行为 |
| [完整文档](docs/README.md) | 使用指南、架构、开发与发布 |

## 参与贡献

欢迎反馈问题、改进文档或提交 Pull Request。[提交 Issue](https://github.com/Leo-Lee-j/conduit/issues) 时，请附上 Conduit 版本、复现步骤、预期结果与实际行为。较大的功能或架构调整，请先讨论范围。

如需从源码构建，请参考 [贡献指南](CONTRIBUTING.zh-CN.md) 中的 Windows Rust 开发环境、构建命令与必要检查。[架构文档](docs/architecture.md)介绍了项目的模块与协作方式。

## 致谢

Conduit 使用 **Rust**、**GPUI** 和 [GPUI Kit](https://github.com/longbridge/gpui-kit) 构建。感谢 Claude Code、Codex、OpenCode 背后的项目，以及提供引擎与模型标志的 [LobeHub Icons](https://github.com/lobehub/lobe-icons)。

资源说明见[应用图标生成](assets/app-icons/README.md)与[第三方图标来源](assets/icons/README.md)。

## 许可证

仓库目前尚未包含项目级 `LICENSE` 文件。第三方图标的许可声明随[图标资源](assets/icons/README.md)保留。
