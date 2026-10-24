# Chat M0 UI 自动预验记录

日期：2026-10-22。本记录是**自动预验**：用隔离数据、Win32 窗口截图与 UI Automation 驱动界面后的人工（Agent）目检，**不代替** [验证记录](m0-validation.md#手工-ui-验收清单) 中要求的人工验收签署。人眼、输入法、真实按键手感、DPI 与多屏等仍须人工完成。

## 方法与环境

- 构建：`cargo build --locked`，Debug 二进制 `target/debug/conduit.exe`；基线提交 `712a4f8`，本记录同时包含设置弹窗窄窗修复。
- 隔离数据：`scripts/ui-acceptance.ps1 -Action seed` 生成 `target/ui-acceptance/data*/state.json`，包含一个临时工作区、Claude 与 Codex 各一个合成会话；不使用 `%LOCALAPPDATA%\Conduit`，不写入正式数据。
- 驱动方式：`Start-Process` 启动 → `PrintWindow` 抓取整窗 PNG → Agent 读图目检；UI Automation 读取按钮/输入框并可 `Invoke` 按名点击；导航与滚动使用窗口坐标点击与滚轮事件。执行期间窗口会短暂出现在桌面上。
- 自动化脚本：[scripts/ui-acceptance.ps1](../scripts/ui-acceptance.ps1)。截图仅用于当轮目检，不纳入仓库。

## 结果

| # | 清单项 | 自动预验 | 备注 |
| --- | --- | --- | --- |
| 1 | Ctrl+Enter | 未覆盖 | 需要真实按键与输入法组合，交人工 |
| 2 | 中文输入法 | 未覆盖 | Windows TSF 输入无法由本脚本可靠模拟 |
| 3 | Emoji 与复制 | 部分 | Emoji、行内代码、围栏代码块、列表渲染正常；鼠标选择与剪贴板一致性未覆盖 |
| 4 | 滚动跟随 | 部分 | 长回复静态渲染与底部定位正常；上滚不被拉回、回到底部继续跟随的动态行为未覆盖 |
| 5 | 审批弹窗 | 未覆盖 | 需要真实 CLI 触发审批；本轮未执行，交人工（生产参数下可能不产生审批，按清单记录为不可复现） |
| 6 | 焦点恢复 | 未覆盖 | 需要真实按键输入验证 |
| 7 | 最小窗口与 DPI | 部分 | 850×500 下 Chat 与设置页无重叠、按钮可见；**发现并修复设置弹窗底部裁切**（见下）；100%–200% 缩放与多屏未覆盖 |
| 8 | 多工作区 | 未覆盖 | 需要两个真实运行任务并行与滚动交互 |
| 9 | 快速关闭 | 未覆盖 | 冒烟脚本使用强制结束，不是正常关窗验收；运行中任务关闭未覆盖 |
| 10 | 设置页 | 部分 | 三页可进入；路径来源、实际启动路径正确；点击“检测版本”返回“版本 2.1.301 · 与本项目已验证版本一致”；取消检测与文件选择对话框未覆盖 |
| 11 | 启动冒烟 | 通过 | `scripts/smoke.ps1 -View chat`、`-View settings` 均 PASS；隔离数据、窗口响应；不覆盖输入与点击 |
| 12 | 模型选择与同步 | 通过（界面项） | 来源列表、CLI 当前模型、CCS 9 Provider / 23 模型、当前 Provider 标记（Claude：anyrouter；Codex：公司）与配置一致；选择器只并入当前 Provider 模型；页面未出现 Token、端点密钥等敏感字段 |

补充：设置页与模型同步的“重新读取”“检测版本”可点击且结果落位；工具卡片可打开右侧执行详情。

## 本轮发现并修复

设置弹窗内容固定 `560px` 高，而对话框层会按“视口高度 − 边缘留白”裁切超出部分：850×500 最小窗口下底部的“完成”按钮与说明被裁掉。已改为高度随视口自适应（`视口高度 - 96px`，限制在 360–560px），正文区域自行滚动；修复后窄窗“完成”可见，常窗保持 560px 不变。

## 复现命令

```powershell
cargo build --locked
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action seed
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name chat-markdown -DataDir target/ui-acceptance/data-claude
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name chat-model-picker -DataDir target/ui-acceptance/data-claude -ClickButton '选择模型'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-models -AppArgs '--settings' -ClickAt '251,446'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-models-codex -AppArgs '--settings' -ClickAt '251,446' -ScrollAt '700,500,20'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-detect -AppArgs '--settings' -ClickAt '243,338;933,462' -SettleMs 5000
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-narrow -AppArgs '--settings' -Width 850 -Height 500
# 面板拖拽（侧栏）/ 详情面板拖拽（先点“详情”再拖边界）
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name sidebar-resize -DataDir target/ui-acceptance/data-claude -DragAt '256,400,360,400'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name detail-resize -DataDir target/ui-acceptance/data-claude -ClickAt '1162,60' -DragAt '908,400,820,400'
# 真实审批弹窗（需 Codex 已登录；先在隔离工作区种子一个空会话，见记录正文）
Set-Clipboard -Value 'Use a shell command to write the file C:\Users\<you>\conduit-approval-probe.txt with the text ok, then reply exactly APPROVAL_TEST_DONE. Do not use apply_patch.'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -DataDir target/ui-acceptance/data-approval -Name approval-pending -ClickAt '600,655' -SendKeys '^v' -SendClickAt '1120,732' -WaitSeconds 50
./scripts/smoke.ps1 -View chat
./scripts/smoke.ps1 -View settings
```

`ui-acceptance.ps1` 以 UTF-8 BOM 保存；用 Windows PowerShell 5.1 执行时改为 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\ui-acceptance.ps1 ...`，否则会因中文编码或执行策略解析失败（2026-10-22 已修复并双版本验证）。脚本支持 UIA 按名点击、窗口坐标点击/拖拽/滚动、剪贴板文本注入、分阶段点击与等待，用于在隔离数据下生成截图与 UIA 证据；它不改变生产数据，也不替代人工签署。

坐标参数与 1216×808 窗口（100% 缩放）对应；不同缩放或窗口尺寸下需按实际窗口重新定位。审批流示例中的发送按钮坐标、审批按钮坐标、探针路径均为本机 100% 缩放下的实测值，换机器需重测。
## 第二轮：2026-10-22 UI 改进与生产参数审批自动预验

本轮按反馈完成五项界面改进，并用同一套隔离脚本复验：

| 改进 | 实现 |
| --- | --- |
| 自定义标题栏 | 移除 Windows 原生标题栏，改用 gpui-kit `TitleBar`：品牌区 + 窗口拖动/双击最大化 + 最小化/最大化/关闭命中区；侧栏品牌行并入标题栏，工作区“+”移到分区标题 |
| 审批卡片 | 宽度改为与对话文字列一致（最大 840px 居中），重做为警示图标 + 标题/队列序号 + 说明 + 可滚动详情 + 右对齐“拒绝 / 允许本次” |
| 左右面板 | 侧栏与执行详情改为可拖拽调宽（侧栏 200–420px，详情 240–480px）；窄窗时详情仍以覆盖层显示 |
| 最新消息与底部工具栏 | “最新消息”移到输入区右上角；底部新增横向预留工具栏（当前显示状态提示），供后续功能扩展 |
| 设置弹窗 | 垂直居中；模型目录与 cc-switch 读取改到后台线程，打开不再阻塞首帧；入场动画改为轻量 fade |

### 生产参数审批链路自动预验（新增）

此前只有测试专用 `untrusted + read-only` 夹具；本轮用**生产参数**（`on-request` + `workspace-write`）在隔离工作区跑真实 Codex 0.163.0 任务，脚本通过 UI Automation、剪贴板输入与合成点击完成：

| 场景 | 结果 |
| --- | --- |
| 请求越界写文件 | 收到真实 `item/commandExecution/requestApproval`，界面显示“请求执行命令”审批卡片 |
| 点击“拒绝” | 状态由“等待你的审批”变为“已提交拒绝本次操作”；模型回复确认未写入；探针文件不存在；本轮完成 |
| 点击“允许本次” | 状态变为“已提交允许本次操作”；探针文件内容为 `ok`，模型回复 `APPROVAL_TEST_DONE`；本轮完成 |

以上为脚本点击，不代替人工点击手感与视觉确认；生产默认权限模式下的 Claude 审批仍依赖本机 CLI 配置。

### 设置弹窗打开时序

在 Debug 构建、隔离数据下重复 3 次测量：窗口句柄出现 593–698 ms，设置弹窗“完成”按钮可被 UI Automation 检出 789–861 ms（均从进程启动起算）。模型目录读取已移出 UI 线程，首帧不再等待配置与 cc-switch 查询。
## 第三轮：2026-10-22 反馈修复

针对使用反馈做了四项修正，均已完成编译、单测与启动冒烟，具体手感需人工复验：

| 反馈 | 处理 | 验证 |
| --- | --- | --- |
| 标题栏拖动“一帧一帧”、跟手延迟 | Debug 构建下窗口移动会触发整窗重绘，而依赖未优化导致重绘过慢。为 dev profile 的依赖启用 `opt-level = 2`、应用本体 `opt-level = 1`（保留 debug 信息），不改变 release 行为 | 全量重建通过、`cargo test` 36 passed、`clippy -D warnings` 通过、双视图冒烟通过；帧率/手感需真实鼠标复验 |
| 审批卡片默认收起命令内容，可展开 | 新增审批详情折叠状态：默认只显示标题、说明与操作按钮；点“展开详情/收起”切换，滚动区域只在展开时构建 | 折叠态渲染正常；展开路径编译通过，自动预验脚本在此轮前台/剪贴板限制下未能稳定复现，需人工点一次确认 |
| 设置弹窗直接呈现、无动画 | 之前的淡入是为规避同步读取卡顿；模型目录读取已在后台线程，恢复组件默认的滑入入场 | 最终帧渲染正常；动画流畅度需人工观察 |
| AI 内容重复呈现 | 根因：流式事件含隐藏 thinking 块，文本块的 stream index 与 assistant 快照的可视块序号错位，同一文本被存成 `...-1` 与 `...-0` 两条。协议改为按文本块序号对齐流式增量与快照；加载旧数据时清理相邻重复助手文本与已由助手文本承载的失败通知；另加失败信息去重与相邻重复块合并 | `cargo test` 38 passed（含 `claude_hidden_thinking_block_does_not_duplicate_visible_text`、`restored_duplicate_assistant_blocks_and_failure_notices_are_cleaned`、`failure_echoed_as_assistant_text_is_detected`） |

## 第四轮：2026-10-22 历史整理（重命名 / 归档 / 删除 / 搜索）

实现 P1 历史整理并通过界面预验：

| 功能 | 实现 |
| --- | --- |
| 搜索 | 侧栏顶部搜索框（Ctrl+V 输入可用），按标题与消息正文实时过滤；无匹配的工作区整块隐藏 |
| 操作菜单 | 每个会话行右侧“⋯”打开重命名/归档/删除菜单 |
| 重命名 | 对话框预填当前标题，取消/保存；标题为空时拒绝并提示 |
| 归档 | 菜单切换归档；默认从列表隐藏，工作区“已归档”按钮可显示并带“已归档”标记；归档状态写入 `state.json`（本轮以持久化文件核对） |
| 删除 | 二次确认对话框，提示不可恢复 |

数据层：`SessionRecord.archived` 为 v2 可选字段（缺省 false），保存/加载与搜索匹配均有单测；`cargo test` 40 passed。删除确认按钮的自动化点击链未稳定复现，删除逻辑与对话框视觉已验证，最终删除动作建议人工点一次确认。

## 第五轮：2026-10-22 侧栏信息架构与搜索弹窗

按反馈调整侧栏入口与搜索形态：

| 反馈 | 实现 |
| --- | --- |
| 去掉侧栏 Claude Code / Codex 新建入口，保留新建会话为图标，顶部改为一行图标功能 | 侧栏顶部改为 [＋ 新建会话] [搜索] 图标行；新建会话使用当前“新建会话引擎” |
| 引擎切换移到 chat 左下角 | 输入区底部工具栏左侧“新建会话引擎”Claude Code / Codex 切换，不改变已打开会话；无会话时输入区用该引擎新建；切到 Claude 后新建，`state.json` 中新增会话 provider=Claude |
| 工作区右侧菜单 + 新建会话图标 | 工作区行新增“+”与“⋯”；菜单含“移除工作区”（仅移除应用路径，二次确认）；实测确认后 state workspaces 由 1→0，磁盘文件未动 |
| 搜索移到顶部图标旁，点击弹出搜索窗口 | 弹窗输入框 + 命中会话列表；标题与摘要高亮命中词，点击结果切换会话 |

自动预验发现并修复：搜索高亮把字符数当作字节数映射，CJK 文本下范围越界导致进程 panic（窗口空白）。改为逐字节映射并在摘要中复用同一范围计算；新增 Unicode 与多字节摘要测试（42 passed）。

仍待人工确认：图标 tooltip/点击手感、引擎切换后新建会话是否符合使用习惯、搜索弹窗长列表滚动。

## 第六轮：2026-10-22 输入与引擎交互细节

| 反馈 | 实现 |
| --- | --- |
| 工作区右侧菜单向右弹出 | 工作区菜单锚点改为 `Anchor::LeftCenter`，菜单出现在行右侧 |
| 每个工作区默认展示最近 N 条会话，设置可调 | `UiSettings.sessions_per_workspace`（默认 5，范围 1–30）；侧栏超出显示“显示更多（还有 X 条）”并可展开/收起；设置 → 通用 → 界面提供步进器，点击“+”后 `state.json` 持久化为 6 |
| Enter 发送、Ctrl/Shift+Enter 换行 | 在 `ConduitComposer > Input` 与 `ConduitComposer` 绑定 Enter→SendPrompt、Ctrl/Shift+Enter→Newline；合成按键验证 Ctrl+Enter 插入换行（两行草稿），Enter 在新会话中发送成功并生成用户消息 |
| 去掉快捷键提示，引擎下拉移到该位置、向上弹出并同步右上角模型选择器 | 输入区左下角引擎下拉（`Anchor::BottomLeft` 向上弹出）；切换后切到该工作区该引擎最近的未归档会话，右上角模型选择器随会话同步（切换后模型选择器显示 GPT-5.6-Sol + Codex） |

预验中发现并修复：工作区行“+”按钮点击冒泡到行选择，导致新建会话后选中被覆盖；已停止该按钮的点击传播。`cargo test` 43 passed。

仍待人工确认：Enter/Ctrl+Enter 在真实中文输入法组合态下的行为（合成按键不等同 IME）、模型选择器与引擎下拉的手感、会话数上限在长列表下的滚动表现。

## 第七轮：2026-10-23 界面细节反馈

按最新反馈调整界面细节，全部为自动预验（截图 + 像素几何测量），不替代人工验收：

| 反馈 | 实现 |
| --- | --- |
| 工作区与会话行的右侧菜单不在一条垂直线上 | 会话行右内边距由 4px 改为 8px，与工作区行一致；像素测量两侧“⋯”右边缘均为 x≈232 |
| 会话记录的右侧菜单也要从右侧弹出 | 会话菜单锚点由 `Anchor::BottomRight + 6px` 改为 `Anchor::LeftCenter + 8px`，与工作区菜单同列向右弹出 |
| “最新消息”改成 Pin 图标，合适的时候自动显示，放到对话区右下角、输入区右上角上方 | 移除输入区右上角“最新消息”文本按钮；消息列表未停留底部（距底部 ≥60px）时在对话区右下角显示浮动 Pin 按钮，点击回到底部并自动隐藏 |
| 功能图标鼠标悬浮改为 pointer 样式 | 会话/工作区菜单、侧栏新建与搜索、引擎与模型下拉（含模型选项行）、设置与审批按钮、菜单项等补 `cursor_pointer`；禁用按钮不加 |
| 移除模型选择器右侧的引擎标识 | 输入区顶部行删除引擎圆点与“Claude Code / Codex”文字，仅保留目录与模型选择器 |
| 工具调用 title 左右 padding 不一致 | 默认图标包不包含 `terminal.svg`，工具卡片标题左侧留下约 24px 的图标占位；移除该占位后标题左内边距与右侧一致（实测标题字形距卡片左边 12px，无图标占位像素） |
| 设置中 CLI 相关信息单独放到 CLI 菜单 | 左栏新增“CLI”页（路径查找顺序与可执行文件/版本说明）；通用页只保留本机数据与界面设置 |

补充：Pin 图标位于完整图标目录中，应用资产源由默认组件包切换为 `gpui_kit::assets::AllAssets`；默认组件图标保持不变。

本轮验证：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`（43 passed）、`cargo build --locked` 均通过；种子数据下 1216×808 与 850×500 窗口布局正常，窄窗下聊天页无裁切，会话菜单向右弹出仍在窗口内，设置弹窗仍可完整滚动。

仍待人工确认：真实鼠标悬浮时光标是否为 pointer（静态截图无法证明）；Pin 的点击手感与自动显隐时机；会话菜单向右弹出时在窄窗或长列表底部是否被视口裁切。

## 第八轮：2026-10-23 只读 Git 变更面板

新增右侧只读 Git 变更面板，并用隔离的合成 Git 仓库 + 非仓库目录做自动预验：

| 目标 | 实现 |
| --- | --- |
| 只读查看当前工作区 Git 状态与 diff | “执行详情”面板新增“Git 变更”页：分支、暂存/未暂存/未跟踪数量、变更文件列表，以及彩色暂存/未暂存 diff |
| 只读保证 | 仅执行 `rev-parse` / `status` / `diff`，status 与 diff 加 `--no-optional-locks`；新增单测在临时仓库上比对 `.git` 文件指纹，确认读取面板前后无变化；测试见 `src/git.rs` 的 `real_snapshot_is_read_only_and_sees_worktree_changes` |
| 失败可恢复 | 非 Git 工作区显示“该工作区不是 Git 仓库”与“重新读取”，不显示原始 fatal 文本；未安装 Git 时提示未找到 git.exe |
| 工具详情不回归 | 面板保留“工具详情 / Git 变更”两页，工具卡片仍打开工具详情 |
| 窄窗可用 | 850×500 下 Git 页以覆盖层显示，内容可滚动 |

实现细节：每段 diff 最多保留 200 KiB、最多列出 2000 个文件，超出截断并提示；Git 命令 15 秒超时并随 Job 结束进程树；命令在后台线程运行，不阻塞 UI。面板打开、点“刷新”或一轮 CLI 任务结束（面板可见）时重读；切换工作区会丢弃旧快照重新读取。未跟踪文件只列出路径，不读取内容。

验证：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`（48 passed，其中 5 个新增 git 测试）、`cargo build --locked` 与双视图启动冒烟通过。

仍待人工确认：真实大仓库 / 宽 diff 的渲染与滚动流畅度、真实 CLI 任务结束后的自动刷新、Windows 未安装 Git 环境的提示，以及只读行为在真实仓库上的手工核对。

## 第九轮：2026-10-23 输入区与侧边栏重构

| 反馈 | 实现 |
| --- | --- |
| 不在会话中时 chat 无法输入；未选工作区时左上角应能快速选择；已选工作区时新会话默认上次使用的引擎；不再在右下角显示“新建会话”；chat 宽高一致 | 输入区收敛为同一个 composer：会话 / 无会话 / 无工作区三种状态宽度、高度、内边距一致；左上角新增工作区选择器（切换已有工作区或添加目录）；无会话时发送会按 `UiSettings.last_provider` 自动新建会话；移除右下角“新建会话”按钮 |
| 会话内的“变更 / 详情”按钮改成一个侧边栏图标，两个功能收进侧边栏，类似 tab 并支持 split；右侧侧边栏最大宽度为内容区一半 | 会话标题右侧改为单个侧边栏图标；面板内保留“工具详情 / Git 变更”页签，split 时两页并排显示；宽度上限按剩余内容区的一半计算 |
| 右侧侧边栏拖动不跟手（向左拖先变宽再变窄） | 右侧侧边栏改为应用自管宽度：拖动左边缘按指针位置 1:1 更新，最小 260px、最大为剩余内容区一半；不再使用会重排的 resizable 面板组；实测拖动 65px 后宽度 408px，继续拖到上限 481px |
| 右侧侧边栏关闭时聊天内容居中 | 保持以左侧工作区栏右侧的整个内容区为基准居中（用户确认该基准正确） |
| 窄窗 | 右侧侧边栏改为覆盖层，页签可用、split 按钮隐藏 |

实现细节：`UiSettings.last_provider` 随状态保存上次选择的引擎；无会话发送在解析 CLI 路径、检查工作目录之后才创建会话，失败不会留下空会话。右侧侧边栏的宽度和 split 状态只在内存中维护；拖动使用 `on_drag` / `on_drag_move`，宽度按“起始宽度 + 指针位移”计算并夹在 `[260, 内容区一半]`。

验证：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`（48 passed）、`cargo build --locked` 与双视图启动冒烟通过。

仍待人工确认：真实输入法下无会话直接发送、上次引擎在重启后的默认值、侧边栏拖动跟手度与 split 下两侧内容滚动、工作区选择器长列表。

## 第十轮：2026-10-24 外观页聊天排版与代码块

| 目标 | 实现 |
| --- | --- |
| 聊天正文可配置 | 外观页新增“聊天文字”卡片：正文字体（下拉，含“默认（跟随主题）”）、字号 12–20、行高 1.2–2.0；作用于用户气泡、Markdown 回复与输入框 |
| 代码块可配置 | 新增“代码块”卡片：代码字体（含“默认（系统等宽）”）、字号 10–18、复制按钮开关、语法高亮开关；代码字体与字号写入全局主题，代码块、行内代码与 Git diff 同步 |
| 即时反馈 | 新增“聊天预览”卡片，用同一段 Markdown（含 rust 代码块与行内代码）实时反映上述设置；“恢复默认”一键回到 15 / 1.5 / 13 与默认字体、开关全开 |
| 不 panic | 字体下拉只列 `text_system().all_font_names()` 已安装字体；启动时清除 state.json 里已卸载的字体族，回退默认 |
| 高亮可关 | `gpui-kit` 启用 `syntect` 特性；关闭高亮时向 `TextViewStyle` 传入进程级静态空 `HighlightTheme`，代码块保持等宽单色 |
| 复制按钮不遮挡 | 开启复制按钮时代码块上内边距改为 34px，语言标签与复制图标浮在右上角；复制 id 按“会话 + 消息 + 代码块序号”唯一，多块互不串状态 |

实现细节：新增字段全部 `#[serde(default)]`，v2 状态文件无需迁移；读取时对字号、行高再次 clamp，手改 `state.json` 也不会把布局推坏。代码字号走主题，是因为代码块节点由主题的 `mono_font_family / mono_font_size` 生成，正文 `text_size` 不会覆盖它。切换明暗主题会重载主题配置，因此每次 `apply_appearance` 之后重新写入代码字体。

预验发现并修复：`Select` 根元素会吃掉行内剩余空间，把同行提示文字压成 0 宽并把行高撑到溢出，导致标签看不见；改为用固定宽度（188px、`flex_shrink_0`）容器包裹下拉。外观页的卡片头与“标签 + 提示 + 控件”行此前在六张卡片里各写一遍，统一抽为 `settings_card` / `settings_card_note` / `settings_control_row`。

验证：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`（79 passed）、`cargo build --locked` 通过。种子数据 1216×808 下：外观页顶部（主题 / 聊天文字）与滚动后（代码块 / 聊天预览 / 内容区域 / 侧栏 / 标签页）截图正常；写入 Arial 18 / 1.8 与 Consolas 16 后聊天页正文与代码块按设置渲染；关闭高亮时代码块单色且语言标签与复制按钮不重叠，开启高亮关闭复制按钮时回到内置样式。

仍待人工确认：字体下拉的真实展开与搜索（弹窗为独立窗口，截图与 UIA 取不到）、中文字体族在正文与代码块的实际观感、只读数据目录下控件禁用态、以及长对话在放大字号后的滚动性能。

## 第十一轮：2026-10-24 用户消息中的围栏代码块

评估过把用户消息整体改成 Markdown 渲染，结论是不做：用户输入不是受控的 markdown 源，`__init__.py`、行首 `#` 与 `-`、日志里的 `---` 会被重新解释，等于静默改写用户原话。改为只识别整段 ``` 围栏。

| 目标 | 实现 |
| --- | --- |
| 只切代码块 | `split_fenced_code` 按行扫描，成对的 ``` 围栏才切出代码段；正文段继续用 `SelectableText` 原样显示，行内反引号、`#`、`-` 不受影响 |
| 不吞正文 | 未闭合的围栏按纯文本保留；闭合围栏要求同种标记、长度不小于起始且无语言信息；起始缩进 ≤3 空格才算围栏，代码体只按围栏缩进裁剪 |
| 复用排版 | 代码段用 `chat_markdown_view` 走 `TextView::markdown`，因此跟随代码字体/字号、语法高亮与复制按钮设置；正文段继续跟随正文设置 |
| 去重 | 回复、外观页预览、用户代码块三处相同的 `TextView` 装配与 `code_block_actions` 闭包统一抽为 `chat_markdown_view` / `code_block_actions` |

实现细节：只支持 ``` 不支持 `~~~`（`markdown` 解析器虽两种都认，但用户文本里 `~~~` 更少见，少一条待验证路径）；解析前统一剥掉行尾 `\r`，种子数据里的 CRLF 不会破坏分段；正文段仅裁掉贴围栏边界的空行，段内换行原样保留。

验证：`cargo fmt --check`、`cargo clippy --locked --all-targets --features ui-tests -- -D warnings`、`cargo test --locked --features ui-tests -- --test-threads=1`（85 passed / 11 ignored）、`cargo build --locked` 通过；新增 4 个分段单元测试。顺带修好 `ui-tests` 特性下 `mod ui_tests` 缺少 `ChatTypography` 导入导致的编译失败。种子数据首条用户消息改为“正文 + rust 围栏 + 正文”，1280×860 截图 `chat-user-code-top.png` 显示代码块等宽着色、语言标签与复制按钮在右上角、上下正文保持纯文本。

仍待人工确认：用户消息里连续多段代码块与超长单行代码在 680px 气泡内的换行观感；关闭语法高亮后用户代码块与回复代码块是否一致；选中跨越正文段与代码段时的拖拽选择体验。

## 第十二轮：2026-10-24 聊天文本的拖拽选中与 Ctrl+C

反馈：聊天文字“选中有问题，不跟手，不是 | 光标那种选中文字复制”；追加要求：不要复制菜单提示，AI 回复要像普通文本一样流畅选中。

根因是**同窗口挂了两层 `TextSelectionLayer`**。gpui-kit 的 `base::Root`（`crates/base/src/root.rs:254`）已经在窗口根挂了唯一一层，而 `src/main.rs` 用的是 `gpui_kit::open_window`，它会把 `ConduitApp` 包进 `base::Root`（`crates/kit/src/lib.rs:185`）。上一轮误判为"从未挂载"（只 grep 了 `src/`，漏看库内 Root），在 `ConduitApp::render` 里又加了一层：两层各自持有选区状态与窗口级鼠标处理，一份写状态、另一份画高亮，于是出现"剪贴板里是整行、屏幕上只亮半行"的错位，也就是用户说的不跟手。

| 目标 | 实现 |
| --- | --- |
| 恢复跟手选中 | 删除自建的 `TextSelectionLayer`，全窗口只保留 `Root` 那一份；拖拽跟随、I 形光标、双击选词、三击选行随之恢复 |
| 去掉复制提示 | 删除 `selection_copy_menu` 与两处 `context_menu` 包裹，以及验收脚本临时新增的 `-RightClickAfter` 时序 |
| Ctrl+C 可用 | 会话容器 `.track_focus(&chat_focus)`，并在鼠标抬起时若已有选区就把焦点移到会话区（`src/app.rs:9559`）：焦点原本在输入框，`Textarea` 无条件消费 `Copy`，所以键盘路径必须先离开输入框；随后 `Root` 的 `key_context("Root")` + `ctrl-c → Copy` 生效 |

新增回归测试 `drag_across_assistant_reply_tracks_the_pointer`（`ui-tests` 特性）：在真实窗口里用 `VisualTestContext` 模拟按下—移动—抬起，断言选中文字随指针增长、`painted_quads()` 里选区高亮的右边界同步跟随，最后派发 `Copy` 动作并断言剪贴板内容等于选区文本。测试用短拖拽探测定位文本行，避免依赖虚拟列表尚未布局时的 `bounds_for_item`。

复现命令（1280×860，种子数据第 2 个会话）：

```powershell
cargo build --locked
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action seed
# AI 回复正文拖拽选中（截图证据：高亮与拖拽区间一致）
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name chat-select-reply -Width 1280 -Height 860 -ClickAt '127,205' -ScrollAt '700,400,-25' -DragAt '375,455,700,455'
# 用户气泡拖拽选中后 Ctrl+C，读回剪贴板
Set-Clipboard -Value 'SENTINEL-NOT-COPIED'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name chat-ctrl-c -Width 1280 -Height 860 -ClickAt '127,205' -ScrollAt '700,400,-25' -DragAt '735,122,1155,122' -SendKeys '^c'
Get-Clipboard
```

验证：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、同名加 `--features ui-tests`、`cargo test --locked --features ui-tests -- --test-threads=1`（86 passed / 11 ignored）、`cargo build --locked` 通过。截图 `chat-select-reply.png` 显示回复正文高亮区间等于拖拽区间（368→697px 对应 375→700px）；`chat-ctrl-c.png` 显示用户气泡整行高亮，`Get-Clipboard` 返回同一行的 37 字原文（拖拽终点落在"关系"与"？"之间，故不含问号）。

仍待人工确认：跨正文段与代码段的连续选区观感、触控板拖动选中、明暗主题下选区高亮对比度，以及"选中后焦点离开输入框"是否会影响连续打字习惯。

## 仍未覆盖，必须人工完成

清单 1、2、5、6、8、9 全项；3 的复制一致性；4 的动态滚动跟随；7 的 100%–200% DPI 与多屏；10 的取消检测与文件选择；5 的真实审批允许/拒绝与多审批队列。人工完成后把结果写回 [验证记录](m0-validation.md#手工-ui-验收清单)，不得用本预验代替。





