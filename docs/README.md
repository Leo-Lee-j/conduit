# 文档

[返回项目首页](../README.md)

文档按使用、开发与发布、实验扩展组织。同一行为只在一个专题中维护；历史审查、阶段进度和逐轮测试输出可从 Git 历史追溯，当前入口只保留操作说明、实现约束和未完成事项。

## 使用 Conduit

| 文档 | 内容 |
| --- | --- |
| [使用指南](usage.md) | 安装后的配置、对话、快捷键、文件、Git、终端与更新 |
| [设置与 AI 连接](settings.md) | CLI、模型服务、Skills、MCP 的管理和保存边界 |
| [文件与内容预览](files.md) | 文件引用、文件内查找、Markdown 和 Mermaid |
| [Git 工作流](git.md) | 提交范围、暂存、分支、日志、标签与失败恢复 |
| [集成终端](terminal.md) | 工作区路由、输入、进程生命周期及资源限制 |
| [通知中心](notifications.md) | 统一消息入口、历史、导航与审批操作 |

## 开发与发布

| 文档 | 内容 |
| --- | --- |
| [参与贡献](../CONTRIBUTING.md) | 开发环境、提交前检查和文档维护规则 |
| [架构](architecture.md) | 产品边界、模块职责与主要数据流 |
| [会话与存储](sessions.md) | 并发、发送确认、正文按需加载、历史导入及恢复 |
| [CLI 适配器](cli-adapters.md) | 接入契约、进程管理、OpenCode 特有行为 |
| [测试与验收](testing.md) | 测试选择、隔离环境、人工验收和结果记录 |
| [性能与指标](performance.md) | 有界工作、缓存、用量语义与测量边界 |
| [发布与更新](releasing.md) | 签名密钥、CI、安装包、更新与故障恢复 |

## 实验扩展

[插件文档](plugins/README.md)区分已接入的原生贡献机制、开发态 Shell 运行时和后续安装管理方案。Shell 生产准入尚未完成。

- [原生贡献契约](plugins/contributions.md)
- [Shell 开发运行时](plugins/runtime.md)
- [验证与准入](plugins/validation.md)
- [后续方案](plugins/roadmap.md)

## 工具与资源

- [Shell 原型运行与命令](../tools/shell-prototype/README.md)、[上游补丁维护](../tools/shell-prototype/patches/README.md)。
- [应用图标生成](../assets/app-icons/README.md)、[引擎与模型图标来源](../assets/icons/README.md)。资源来源和许可说明随资源保留。

文档应描述当前仓库行为。未实现方案必须标注状态；测试命令不代表已经执行，过去一次通过也不代表当前版本已完成验收。维护约定见[参与贡献](../CONTRIBUTING.md)。
