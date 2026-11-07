# CLI、Model、Skills 与 MCP 设置界面

更新日期：2026-11-07。

## 参考与范围

参考 `farion1231/cc-switch` 官方仓库的主界面、Apps 截图，以及 `ProviderCard.tsx`、`UnifiedSkillsPanel.tsx`、`UnifiedMcpPanel.tsx`：使用明确的管理页标题、统一的引擎切换、顶部主要操作、紧凑服务卡片和带搜索筛选的扩展列表。通过现有 GPUI 控件实现，沿用项目明暗主题和语义颜色。

保留「CLI 引擎」「模型与服务」「Skills」「MCP 服务」四个独立入口。设置继续作为工作区内的标签页打开，与会话标签共用顶部 tab 栏；保留工作区侧栏、右侧详情面板及其宽度和选择状态。

## 页面结构

| 页面 | 固定区域 | 正文 |
| --- | --- | --- |
| CLI 引擎 | 标题、Claude Code / Codex / OpenCode 切换 | 引擎状态、协议、启停；启动路径、文件选择与恢复自动查找；合并后的更新操作、进度、停止与更新说明；按需展开路径优先级 |
| 模型与服务 | 标题、重新读取配置、添加服务、引擎切换 | 服务卡片；CLI 默认配置与按需展开的模型目录；配置来源 |
| Skills | 标题、创建与文件夹导入、引擎切换、范围、搜索、筛选与刷新 | 虚拟列表、状态与来源标签、编辑、停用或恢复、按需展开详情 |
| MCP 服务 | 标题、添加服务、引擎切换、范围、搜索、筛选与刷新 | 虚拟列表、传输与目标地址、状态与来源、编辑、启停、详情中的连接测试与取消 |

四页复用 `management.rs` 的页头、引擎控件和状态标签。根据扣除工作区侧栏与详情面板后的 tab 可用宽度收紧导航、页头与操作按钮，扩展搜索在窄 tab 内单独一行；低于 650px 的窗口压缩页头说明，扩展列表保留独立滚动空间。标题与主要操作保留在页头，CLI 与模型正文继续使用原生设置表单的滚动和焦点行为。

## 状态与交互

- 默认服务以主题强调色边框、淡色背景和「新会话默认」标签标识。服务名称、来源、端点、模型数量与引用信息分层显示，操作根据空间同排或换行。
- CCS 服务保留来源说明，在 cc-switch 中编辑；本地服务保留编辑、删除和引用保护。OpenCode 继续管理原生模型目录。
- Skills/MCP 保留引擎与工作区范围、独立搜索和筛选状态；长目录使用虚拟列表。无条目、读取中、取消和错误均有明确反馈。
- 扩展扫描与 MCP 连接结果按设置标签页的活动状态接收，修复从弹窗迁移后仍检查活动弹窗、导致后台结果被取消的问题。
- 保留只读限制、导入和配置编辑器、启停确认、连接测试，以及后台任务取消与过期上下文隔离。
- 搜索分组索引随新布局调整。聚焦模型分组时，页头提供「查看完整页面」，避免隐藏原生页面标题后丢失返回入口。
- 路径、服务和模型的保存方式沿用现有逻辑；状态栏继续区分自动保存、表单保存、只读和错误。

## CLI 检查更新

CLI 引擎页的「版本与更新」区位于启动路径之前，保留一个更新按钮，移除独立的版本检测与安装按钮。默认「检查更新」，发现新版本后原位切换为「一键更新」；无新版本时显示「已经是最新版本」约 2 秒，再恢复「检查更新」。检查期间提供取消入口；失败或取消后可重试。按钮状态按引擎与实际启动路径隔离，路径变更清除旧结果。结果区分「发现可用更新」「已是最新版本」「当前版本高于官方稳定版」，同时展示当前与最新版本，并保留兼容性基准提示。网络失败仍保留已读到的本机版本。

后台读取所选可执行文件的 `--version`，查询官方 GitHub 仓库的 latest release；元数据请求限时 15 秒、最多读取 1 MiB，拒绝草稿、预发布与无法识别的版本。版本比较遵循 SemVer（忽略构建元数据），路径变更与取消后的结果不会发布为有效更新。检查支持只读模式，官方更新说明按引擎打开；检查本身不修改安装或设置。

## CLI 一键更新

检查发现新版本后，同一按钮变为「一键更新」，后台先复核所选程序的当前版本与官方稳定版；已是最新或高于稳定版时跳过安装。更新期间显示阶段、用时和最近输出，可停止更新。成功后重新运行原路径的 `--version`，只有版本达到目标才显示完成，并短暂显示「已经是最新版本」后恢复检查入口；进程退出成功但版本未变化会报告失败。

- npm：通过所选 `.cmd` 内容与相邻的包清单，或 `node_modules` 内的原生可执行文件识别所属安装目录；三个引擎均使用对应官方包更新到检查所得的精确版本，显式传入原 `--prefix`。直接运行 Node 与 npm 脚本，保留带空格、中文及 shell 特殊字符的路径参数。
- 原生 Claude Code：执行所选程序的 `update`；原生 OpenCode：执行 `upgrade <目标版本>`。独立 Codex 可执行文件、无法确认的启动器，以及已识别的 WinGet / Scoop / Chocolatey / pnpm / Yarn / Bun 安装来源显示原渠道更新说明，避免在其他位置安装副本或混用包管理器。
- 安装限制为可写实例，检测和检查更新仍允许只读。运行中的会话、自动命名、Commit Message 生成和模型查询会暂时阻止更新；安装期间阻止新 CLI 任务、修改启动路径和并发安装。
- 更新子进程复用 Windows Job 管理，最长 10 分钟；输出保留有界尾部并节流进度消息。停止或退出应用时终止进程树，并明确提示安装可能未完成，可重新检测或重试；不会将取消结果显示为成功。更新后使扩展与 OpenCode 模型目录缓存失效。

## 验证

- 单按钮交互调整：`cargo test --locked --features ui-tests --bin conduit settings -- --test-threads=1`，25 项通过，覆盖无更新提示自动恢复、可更新状态保留、失败重试、过期路径结果隔离，以及移除独立版本检测与安装按钮。
- 单按钮桌面验收：`settings-cli-single-update-available.png`、`settings-cli-single-update-latest.png`、`settings-cli-single-update-reset.png` 分别记录可更新、短暂最新提示与恢复检查。默认可执行文件被运行中的实例占用，本轮通过 `cargo rustc --locked --bin conduit -- -o G:/office/product/conduit/target/debug/conduit-cli-update-preview.exe` 构建预览程序并验收，保留现有窗口。

- 完整应用范围：`cargo test --locked --features ui-tests --bin conduit app:: -- --test-threads=1`，286 项通过。
- 设置相关用例包含于应用回归，覆盖版本更新结果、网络失败、取消、重复任务拦截、只读状态和路径变更后的结果隔离。新增 4 项安装后端测试和 2 项 GPUI 交互测试，使用临时安装目录验证来源识别、原 npm prefix 与参数边界、输出限制、失败退出、停止与超时、安装后版本复核，以及任务互斥和 tab 内按钮边界，不升级真实 CLI。
- `cargo fmt --all --check`、`cargo clippy --locked --all-targets --features ui-tests -- -D warnings`、`cargo build --locked --bin conduit`、`git diff --check`。
- CLI 范围 `cargo test --locked --features ui-tests --bin conduit cli -- --test-threads=1`：36 项通过、3 项依赖本机环境的测试默认忽略；另单独运行 `cli::updates::tests::installed_cli_updates_report_official_versions --ignored --nocapture`，三个已安装引擎的只读联网检查通过。
- 自动化覆盖明暗主题、850 × 500 / 1280 × 900、页头按钮与列表边界、引擎切换、只读状态、搜索分组返回、编辑器与扩展上下文隔离，以及设置 tab 保留外层侧栏和详情面板的位置、宽度。
- 桌面截图使用 `target/ui-acceptance/cc-switch-data` 和隔离 profile，以合成服务、Skill 与 MCP 定义检查视觉。证据位于 `target/ui-acceptance/out/settings-tab-*.png`；更新检查的明暗主题截图为 `settings-cli-update-light.png` 与 `settings-cli-update-narrow-dark.png`，均通过桌面实际点击检查；一键更新按钮在浅色与窄窗口深色下的截图为 `settings-cli-install-light.png` 和 `settings-cli-install-narrow-dark.png`，不提交构建产物。

此前全仓默认测试的库测试 33 项、主程序测试 365 项通过（16 项忽略）；`conduit-updater` 测试可执行文件因 Windows 提升权限要求（os error 740）未启动，不能将该轮记为全仓全部通过。
