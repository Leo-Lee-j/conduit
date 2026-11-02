# Conduit

Windows 原生 AI 编程工作台，将 Claude Code、Codex 和 OpenCode 的对话、工作区与 Git 操作放在同一个界面中。

使用 Rust、GPUI 和 GPUI Kit 构建。目前支持 Windows x64，持续开发中。

## 功能

- **多引擎对话**：流式回复、工具审批、取消任务、原生会话续接，以及按会话选择模型与思考强度。
- **工作区与历史**：多会话并行、搜索、自动命名，以及 Claude / Codex 终端历史导入。
- **文件与 Git**：文件预览、语法高亮、Diff、暂存、分支操作、提交与推送，支持 AI 生成提交说明。
- **阅读体验**：Markdown、Mermaid 图表、主题与字体设置。
- **应用更新**：后台下载、更新日志，以及安装并重启。

## 安装与使用

在 [Releases](https://github.com/Leo-Lee-j/conduit/releases) 下载 Windows x64 安装包。

1. 安装并登录至少一个受支持的 CLI：Claude Code、Codex 或 OpenCode。
2. 打开 Conduit，添加本地工作区。
3. 选择引擎和模型，输入任务开始对话。

Conduit 复用 CLI 的认证与执行能力，不自带模型服务。详细配置见 [使用指南](docs/usage.md)。

## 从源码运行

需要 Rust MSVC 工具链，以及带 C++ 工具和 Windows SDK 的 Visual Studio Build Tools。

```powershell
git clone https://github.com/Leo-Lee-j/conduit.git
cd conduit
cargo run --locked
```

构建、测试与贡献流程见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 文档与反馈

- [使用指南](docs/usage.md)
- [自动发布与更新](docs/auto-update.md)
- [技术设计](docs/technical-design.md)
- [完整文档](docs/README.md)

欢迎通过 [Issues](https://github.com/Leo-Lee-j/conduit/issues) 反馈问题，或提交 Pull Request。
