# 技术设计

Conduit 使用 Rust、GPUI 和 GPUI Kit 构建 Windows 原生界面，通过本机 CLI 的结构化协议执行任务。当前发布目标为 Windows x64。

## 模块

| 模块 | 职责 |
| --- | --- |
| `main.rs`、`app.rs`、`app/` | 窗口、应用状态、聊天、文件、Git、设置与更新界面 |
| `protocol.rs`、`protocol/` | 引擎请求、流式事件、审批与原生会话身份 |
| `engines.rs`、`cli_adapter.rs`、`cli_adapter/` | 引擎注册工厂、启动描述、运行时适配器与原生旁路通信 |
| `chat_runtime.rs`、`child.rs` | 后台任务、stdio、取消、Windows Job 与进程树清理 |
| `cli.rs`、`models.rs`、`providers.rs`、`ccs.rs` | CLI 定位、模型目录与 Provider 路由 |
| `history.rs`、`domain.rs`、`store.rs` | 历史导入、数据模型、兼容读取与原子保存 |
| `git.rs`、`git/`、`workspace_files.rs` | Git 查询与操作、文件树和有界文本读取 |
| `update.rs`、`update/` | GitHub 检查、签名协议、下载缓存与安装交接 |
| `bin/conduit-updater.rs` | 独立安装助手、备份恢复与重启 |
| `bin/release-manifest.rs` | 发布清单签名与密钥生成工具 |

`app/` 通过 `impl ConduitApp` 扩展同一应用状态。共享更新协议位于库 crate，供桌面程序、助手和发布工具复用；旧终端原型位于 `src/legacy/`，不参与构建。

## CLI 协议与生命周期

- Codex 使用 `app-server --stdio`；显式选择的 Provider 配置和模型同时通过启动参数 `-c` 与会话请求传入，避免恢复会话时被进程继承的其他 Provider 配置误拦截。
- Claude Code 使用 stream-json 与 stdio 控制协议。
- OpenCode 使用 ACP；自动命名与提交说明使用独立文本任务。

每轮对话有独立运行时、事件通道、审批队列和取消标记。后台线程读取协议，UI 线程合并事件、刷新视图并保存状态。Windows Job Objects 管理子进程树，确认进程退出后才释放任务。

聊天和辅助任务通过引擎注册表创建 `CliAdapter`。保存确认后，适配器返回最终参数、环境和首次输入方式；运行时统一处理 stdio、审批票据、取消和进程树清理。OpenCode 的原生问答通信由它的适配器负责，适配器资源在进程清理后释放。新 CLI 可独立实现该接口，具体边界、兼容范围和接入步骤见 [CLI 适配层与接入契约](cli-adapter-design.md)。

会话固定工作区、引擎和原生会话 ID；续接失败会显示错误，不自动另建丢失上下文的会话。不同会话可以并行，同引擎同原生 ID 防重。Git 写操作按仓库范围与聊天协调，详见 [会话并发](session-concurrency.md)。

审批仅处理 CLI 实际发出的请求；应用的权限模式不等于操作系统文件沙箱。CLI 自己负责认证、模型调用和工具执行。

## 界面与文件

GPUI 负责原生布局和渲染。聊天、文件预览与 Diff 共用主区标签，工具详情按需在右侧显示；网络、目录扫描、Git 与安装等待在后台执行。

源文件和 Diff 当前只读。文件读取有大小、行数和编码限制；文件引用和链接经过工作区边界校验。Markdown 与 Mermaid 原生渲染，不依赖浏览器或 Node.js。专题说明见 [文件引用](chat-file-mentions.md)、[Git](git-ui-design.md) 和 [Mermaid](mermaid.md)。

工作区会话侧栏使用 GPUI Kit 的 `v_virtual_list`：工作区标题、导入提示、会话和“显示更多”组成一条可变行高列表，只创建视口附近的行。`app/sidebar/cache.rs` 按工作区缓存会话摘要、排序和行尺寸；正文更新只检查变更会话的摘要，列表相关字段未变化时保留侧栏缓存。新增、删除、导入及运行状态变化更新相应工作区，排序或插入后按稳定会话 ID 保持滚动位置。默认条数、展开和折叠行为不变，滚动时关闭行内删除确认。

侧栏全文过滤复用后台搜索的完整匹配 ID 集合；搜索弹窗仍只展示前 40 个摘要，输入防抖为 150ms，过期查询和版本的结果不写回。虚拟化只限制 UI 行元素数量，会话正文仍随 `state.json` 完整载入；正文按需加载需要另行调整存储和搜索。相关回归在 `app/sidebar/tests.rs`，覆盖一万条会话、连续流式增量、滚动锚点、搜索上限和列表交互。

## 存储

默认目录为 `%LOCALAPPDATA%\Conduit`，可通过 `CONDUIT_DATA_DIR` 隔离。`state.json` 保存工作区、会话、模型选择、CLI 配置和更新偏好；新增字段使用默认值兼容旧状态。

写入先刷盘，再通过 Windows 原子替换更新文件。未知版本或损坏数据不会被默认配置覆盖。重启不重放任务，未完成运行显示为中断。同一数据目录通过互斥量限制单实例。

普通输入草稿在内存中隔离。应用更新重启前将草稿、附件和选择位置写入独立恢复文件，启动时恢复后再清理附件。

## 发布与更新

推送与 Cargo 版本一致的稳定标签触发 GitHub Actions：检查、构建、NSIS 打包、生成说明和签名清单，完整上传后发布 Release。

客户端后台比较 SemVer、验证 Ed25519 清单并下载安装包，校验大小和 SHA-256 后才允许安装。独立助手从缓存运行，等待旧进程退出，获取安装目录独占锁，备份后执行 NSIS 静默安装；失败恢复旧文件，成功重启并显示 changelog。

公钥随构建内置，私钥仅用于 CI 签名。安装与数据目录分离。详细配置、失败恢复和限制见 [自动发布与更新](auto-update.md)。

## 构建与验证

依赖通过 `Cargo.lock` 和 GPUI Kit 的固定 revision 锁定。主程序的 Windows 图标、版本资源和栈预留由 `build.rs` 配置；NSIS 安装包包含应用、更新助手和 MSVC CRT。

开发、静态检查与测试命令见 [参与贡献](../CONTRIBUTING.md)。历史验证结果保留在 [M0 记录](m0-validation.md)，不作为当前版本的完整验收结论。
