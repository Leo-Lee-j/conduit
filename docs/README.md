# 文档

## 使用与开发

- [使用指南](usage.md)：CLI 配置、工作区、会话、Git 和本地数据。
- [参与贡献](../CONTRIBUTING.md)：开发环境、运行、测试与 Pull Request。
- [自动发布与更新](auto-update.md)：签名密钥、GitHub Actions、安装包和恢复。
- [技术设计](technical-design.md)：模块、协议、进程与存储。
- [产品范围](product-design.md)：功能定位与边界。

## 专题

- [OpenCode 接入](opencode-cli.md)
- [Git 界面](git-ui-design.md)
- [CLI 与模型服务设置](cli-model-services-design.md)
- [文件引用](chat-file-mentions.md)
- [会话并发](session-concurrency.md)
- [Mermaid 预览](mermaid.md)
- [Token 指标](chat-token-metrics-review.md)

## 设计与验证记录

以下文档记录特定阶段的评估或验收，日期和结果仅代表当时状态。

- [性能设计审查](performance-design-review-2026-11-05.md)：聊天分组与工具统计、搜索进展、正文写放大，以及隔离探针和定向回归结果。
- [通知中心实现进度与验收](notification-center-evaluation.md)：首版已实现；第 11 节记录集成回归结果及剩余人工验收项，前文保留设计依据。
- [工作台界面插件扩展性评估](ui-plugin-extensibility-evaluation.md)
- [工作台插件分阶段设计](plugins/README.md)：界面贡献、Shell 候选 / 进程备选、安装管理与验收。
- [P1-4 底栏、统一命令与完整示例进度](plugins/p1-4-progress-2026-11-05.md)：开发开关、三处联动、验证结果与 P1-5 交接。
- [P1-5 实例生命周期与接口验收](plugins/p1-5-progress-2026-11-05.md)：有界缓存、异步版本屏障、摘要刷新、原生退出清理及 P2 前置边界。
- [P2 前置：G2/G5 宿主控制与 G1 兼容](plugins/p2-host-controls-2026-11-05.md)：公开授权加载、资源限额、故障恢复、Fast 兼容补丁及下一步契约。
- [P2 前置：G3 定向命令与统一停止](plugins/p2-targeted-commands-2026-11-05.md)：具体视图路由、有界异步结果、上下文失效与多视图取消。
- [P2-A 开发加载适配器](plugins/p2-development-loader-2026-11-05.md)：清单绑定、会话快照、P1 实例 / ticket、逐插件授权、惰性激活与停止验证。
- [P2-A 真实桌面开发接入](plugins/p2-desktop-integration-2026-11-05.md)：真实导航、右侧与底栏、工作区摘要、关闭 / 更新停止及 Windows 验证。
- [P2-A G6 浮层所有权与清理](plugins/p2-overlay-cleanup-2026-11-05.md)：停止时按运行时撤销浮层、焦点恢复和更新前同步清理；下一步 G4 输入与焦点、G5 性能验收。
- [P2-A G4 输入、焦点与主题](plugins/p2-input-focus-2026-11-05.md)：组合输入接口、插件编辑、主题切换、浮层 Esc 和窄窗状态回归；真实 Windows IME 仍待验收。
- [GPUI Shell 复用评估](plugins/gpui-shell-reuse-assessment.md)：官网与锁定版本对照、三处扩展分工及接入验证条件。
- [发布与更新评估](github-release-and-updater-evaluation.md)
- [OpenCode 接入评估](opencode-cli-evaluation.md)
- [M0 验证记录](m0-validation.md)
- [UI 验收步骤](m0-manual-acceptance-guide.md)
- [UI 自动预验记录](m0-ui-acceptance-run.md)
