# Chat M0 UI 自动预验记录

日期：2026-10-22。本记录是**自动预验**：用隔离数据、Win32 窗口截图与 UI Automation 驱动界面后的人工（Agent）目检，**不代替** [验证记录](m0-validation.md#手工-ui-验收清单) 中要求的人工验收签署。人眼、输入法、真实按键手感、DPI 与多屏等仍须人工完成。

## 方法与环境

- 构建：`cargo build --locked`，Debug 二进制 `target/debug/conduit.exe`；基线提交 `712a4f8`，本记录同时包含设置弹窗窄窗修复。
- 隔离数据：`scripts/ui-acceptance.ps1 -Action seed` 生成 `target/ui-acceptance/data*/state.json`，包含一个临时工作区、Claude 与 Codex 各一个合成会话；不使用 `%LOCALAPPDATA%\Conduit`，不写入正式数据。
- 驱动方式：`Start-Process` 启动 → `PrintWindow` 抓取整窗 PNG → Agent 读图目检；UI Automation 读取按钮/输入框并可 `Invoke` 按名点击；导航与滚动使用窗口坐标点击与滚轮事件。执行期间窗口会短暂出现在桌面上。
- 自动化脚本：[scripts/ui-acceptance.ps1](../scripts/ui-acceptance.ps1)。证据截图：[docs/screenshots/acceptance/](screenshots/acceptance/)。

## 结果

| # | 清单项 | 自动预验 | 证据 | 备注 |
| --- | --- | --- | --- | --- |
| 1 | Ctrl+Enter | 未覆盖 | — | 需要真实按键与输入法组合，交人工 |
| 2 | 中文输入法 | 未覆盖 | — | Windows TSF 输入无法由本脚本可靠模拟 |
| 3 | Emoji 与复制 | 部分 | [chat-markdown.png](screenshots/acceptance/chat-markdown.png) | Emoji、行内代码、围栏代码块、列表渲染正常；鼠标选择与剪贴板一致性未覆盖 |
| 4 | 滚动跟随 | 部分 | [chat-codex-long.png](screenshots/acceptance/chat-codex-long.png) | 长回复静态渲染与底部定位正常；上滚不被拉回、回到底部继续跟随的动态行为未覆盖 |
| 5 | 审批弹窗 | 未覆盖 | — | 需要真实 CLI 触发审批；本轮未执行，交人工（生产参数下可能不产生审批，按清单记录为不可复现） |
| 6 | 焦点恢复 | 未覆盖 | — | 需要真实按键输入验证 |
| 7 | 最小窗口与 DPI | 部分 | [chat-narrow.png](screenshots/acceptance/chat-narrow.png)、[settings-narrow-after.png](screenshots/acceptance/settings-narrow-after.png) | 850×500 下 Chat 与设置页无重叠、按钮可见；**发现并修复设置弹窗底部裁切**（见下）；100%–200% 缩放与多屏未覆盖 |
| 8 | 多工作区 | 未覆盖 | — | 需要两个真实运行任务并行与滚动交互 |
| 9 | 快速关闭 | 未覆盖 | — | 冒烟脚本使用强制结束，不是正常关窗验收；运行中任务关闭未覆盖 |
| 10 | 设置页 | 部分 | [settings-general.png](screenshots/acceptance/settings-general.png)、[settings-claude.png](screenshots/acceptance/settings-claude.png)、[settings-codex.png](screenshots/acceptance/settings-codex.png) | 三页可进入；路径来源、实际启动路径正确；点击“检测版本”返回“版本 2.1.301 · 与本项目已验证版本一致”；取消检测与文件选择对话框未覆盖 |
| 11 | 启动冒烟 | 通过 | `scripts/smoke.ps1 -View chat`、`-View settings` 均 PASS | 隔离数据、窗口响应；不覆盖输入与点击 |
| 12 | 模型选择与同步 | 通过（界面项） | [settings-models.png](screenshots/acceptance/settings-models.png)、[settings-models-codex.png](screenshots/acceptance/settings-models-codex.png)、[chat-model-picker.png](screenshots/acceptance/chat-model-picker.png) | 来源列表、CLI 当前模型、CCS 9 Provider / 23 模型、当前 Provider 标记（Claude：anyrouter；Codex：公司）与配置一致；选择器只并入当前 Provider 模型；页面未出现 Token、端点密钥等敏感字段 |

补充：设置页与模型同步的“重新读取”“检测版本”可点击且结果落位；工具卡片可打开右侧执行详情（[chat-tool-detail.png](screenshots/acceptance/chat-tool-detail.png)）。

## 本轮发现并修复

设置弹窗内容固定 `560px` 高，而对话框层会按“视口高度 − 边缘留白”裁切超出部分：850×500 最小窗口下底部的“完成”按钮与说明被裁掉（[settings-narrow-before.png](screenshots/acceptance/settings-narrow-before.png)）。已改为高度随视口自适应（`视口高度 - 96px`，限制在 360–560px），正文区域自行滚动；修复后窄窗“完成”可见（[settings-narrow-after.png](screenshots/acceptance/settings-narrow-after.png)），常窗保持 560px 不变。

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

坐标参数与 1216×808 窗口（100% 缩放）对应；不同缩放或窗口尺寸下需按截图重新定位。审批流示例中的发送按钮坐标、审批按钮坐标、探针路径均为本机 100% 缩放下的实测值，换机器需重测。
## 第二轮：2026-10-22 UI 改进与生产参数审批自动预验

本轮按反馈完成五项界面改进，并用同一套隔离脚本复验：

| 改进 | 实现 | 证据 |
| --- | --- | --- |
| 自定义标题栏 | 移除 Windows 原生标题栏，改用 gpui-kit `TitleBar`：品牌区 + 窗口拖动/双击最大化 + 最小化/最大化/关闭命中区；侧栏品牌行并入标题栏，工作区“+”移到分区标题 | [ui-v2-main-window.png](screenshots/acceptance/ui-v2-main-window.png) |
| 审批卡片 | 宽度改为与对话文字列一致（最大 840px 居中），重做为警示图标 + 标题/队列序号 + 说明 + 可滚动详情 + 右对齐“拒绝 / 允许本次” | [ui-v2-approval-pending.png](screenshots/acceptance/ui-v2-approval-pending.png) |
| 左右面板 | 侧栏与执行详情改为可拖拽调宽（侧栏 200–420px，详情 240–480px）；窄窗时详情仍以覆盖层显示 | [ui-v2-sidebar-resized.png](screenshots/acceptance/ui-v2-sidebar-resized.png)、[ui-v2-detail-resized.png](screenshots/acceptance/ui-v2-detail-resized.png) |
| 最新消息与底部工具栏 | “最新消息”移到输入区右上角；底部新增横向预留工具栏（当前显示状态提示），供后续功能扩展 | [ui-v2-main-window.png](screenshots/acceptance/ui-v2-main-window.png)、[ui-v2-narrow.png](screenshots/acceptance/ui-v2-narrow.png) |
| 设置弹窗 | 垂直居中；模型目录与 cc-switch 读取改到后台线程，打开不再阻塞首帧；入场动画改为轻量 fade | [ui-v2-settings-centered.png](screenshots/acceptance/ui-v2-settings-centered.png) |

### 生产参数审批链路自动预验（新增）

此前只有测试专用 `untrusted + read-only` 夹具；本轮用**生产参数**（`on-request` + `workspace-write`）在隔离工作区跑真实 Codex 0.163.0 任务，脚本通过 UI Automation、剪贴板输入与合成点击完成：

| 场景 | 结果 |
| --- | --- |
| 请求越界写文件 | 收到真实 `item/commandExecution/requestApproval`，界面显示“请求执行命令”审批卡片（[ui-v2-approval-pending.png](screenshots/acceptance/ui-v2-approval-pending.png)） |
| 点击“拒绝” | 状态由“等待你的审批”变为“已提交拒绝本次操作”；模型回复确认未写入；探针文件不存在；本轮完成（[ui-v2-approval-denied.png](screenshots/acceptance/ui-v2-approval-denied.png)） |
| 点击“允许本次” | 状态变为“已提交允许本次操作”；探针文件内容为 `ok`，模型回复 `APPROVAL_TEST_DONE`；本轮完成（[ui-v2-approval-allowed.png](screenshots/acceptance/ui-v2-approval-allowed.png)） |

以上为脚本点击，不代替人工点击手感与视觉确认；生产默认权限模式下的 Claude 审批仍依赖本机 CLI 配置。

### 设置弹窗打开时序

在 Debug 构建、隔离数据下重复 3 次测量：窗口句柄出现 593–698 ms，设置弹窗“完成”按钮可被 UI Automation 检出 789–861 ms（均从进程启动起算）。模型目录读取已移出 UI 线程，首帧不再等待配置与 cc-switch 查询。
## 第三轮：2026-10-22 反馈修复

针对使用反馈做了四项修正，均已完成编译、单测与启动冒烟，具体手感需人工复验：

| 反馈 | 处理 | 验证 |
| --- | --- | --- |
| 标题栏拖动“一帧一帧”、跟手延迟 | Debug 构建下窗口移动会触发整窗重绘，而依赖未优化导致重绘过慢。为 dev profile 的依赖启用 `opt-level = 2`、应用本体 `opt-level = 1`（保留 debug 信息），不改变 release 行为 | 全量重建通过、`cargo test` 36 passed、`clippy -D warnings` 通过、双视图冒烟通过；帧率/手感需真实鼠标复验 |
| 审批卡片默认收起命令内容，可展开 | 新增审批详情折叠状态：默认只显示标题、说明与操作按钮；点“展开详情/收起”切换，滚动区域只在展开时构建 | 折叠态截图 [ui-v3-approval-collapsed.png](screenshots/acceptance/ui-v3-approval-collapsed.png)；展开路径编译通过，自动预验脚本在此轮前台/剪贴板限制下未能稳定复现，需人工点一次确认 |
| 设置弹窗直接呈现、无动画 | 之前的淡入是为规避同步读取卡顿；模型目录读取已在后台线程，恢复组件默认的滑入入场 | 最终帧截图 [ui-v3-settings.png](screenshots/acceptance/ui-v3-settings.png)；动画流畅度需人工观察 |
| AI 内容重复呈现 | 根因：流式事件含隐藏 thinking 块，文本块的 stream index 与 assistant 快照的可视块序号错位，同一文本被存成 `...-1` 与 `...-0` 两条。协议改为按文本块序号对齐流式增量与快照；加载旧数据时清理相邻重复助手文本与已由助手文本承载的失败通知；另加失败信息去重与相邻重复块合并 | `cargo test` 38 passed（含 `claude_hidden_thinking_block_does_not_duplicate_visible_text`、`restored_duplicate_assistant_blocks_and_failure_notices_are_cleaned`、`failure_echoed_as_assistant_text_is_detected`） |

界面当前状态见 [ui-v3-main-window.png](screenshots/acceptance/ui-v3-main-window.png)。
## 第四轮：2026-10-22 历史整理（重命名 / 归档 / 删除 / 搜索）

实现 P1 历史整理并通过界面预验：

| 功能 | 实现 | 证据 |
| --- | --- | --- |
| 搜索 | 侧栏顶部搜索框（Ctrl+V 输入可用），按标题与消息正文实时过滤；无匹配的工作区整块隐藏 | [ui-v4-search-filter.png](screenshots/acceptance/ui-v4-search-filter.png)、[ui-v4-history-sidebar.png](screenshots/acceptance/ui-v4-history-sidebar.png) |
| 操作菜单 | 每个会话行右侧“⋯”打开重命名/归档/删除菜单 | [ui-v4-session-menu.png](screenshots/acceptance/ui-v4-session-menu.png) |
| 重命名 | 对话框预填当前标题，取消/保存；标题为空时拒绝并提示 | [ui-v4-rename-dialog.png](screenshots/acceptance/ui-v4-rename-dialog.png) |
| 归档 | 菜单切换归档；默认从列表隐藏，工作区“已归档”按钮可显示并带“已归档”标记；归档状态写入 `state.json`（本轮以持久化文件核对） | [ui-v4-archived-view.png](screenshots/acceptance/ui-v4-archived-view.png) |
| 删除 | 二次确认对话框，提示不可恢复 | [ui-v4-delete-dialog.png](screenshots/acceptance/ui-v4-delete-dialog.png) |

数据层：`SessionRecord.archived` 为 v2 可选字段（缺省 false），保存/加载与搜索匹配均有单测；`cargo test` 40 passed。删除确认按钮的自动化点击链未稳定复现，删除逻辑与对话框视觉已验证，最终删除动作建议人工点一次确认。

## 第五轮：2026-10-22 侧栏信息架构与搜索弹窗

按反馈调整侧栏入口与搜索形态：

| 反馈 | 实现 | 证据 |
| --- | --- | --- |
| 去掉侧栏 Claude Code / Codex 新建入口，保留新建会话为图标，顶部改为一行图标功能 | 侧栏顶部改为 [＋ 新建会话] [搜索] 图标行；新建会话使用当前“新建会话引擎” | [ui-v5-main-window.png](screenshots/acceptance/ui-v5-main-window.png) |
| 引擎切换移到 chat 左下角 | 输入区底部工具栏左侧“新建会话引擎”Claude Code / Codex 切换，不改变已打开会话；无会话时输入区用该引擎新建 | [ui-v5-engine-switch.png](screenshots/acceptance/ui-v5-engine-switch.png)；切到 Claude 后新建，`state.json` 中新增会话 provider=Claude |
| 工作区右侧菜单 + 新建会话图标 | 工作区行新增“+”与“⋯”；菜单含“移除工作区”（仅移除应用路径，二次确认） | [ui-v5-workspace-menu.png](screenshots/acceptance/ui-v5-workspace-menu.png)、[ui-v5-remove-workspace-dialog.png](screenshots/acceptance/ui-v5-remove-workspace-dialog.png)；实测确认后 state workspaces 由 1→0，磁盘文件未动 |
| 搜索移到顶部图标旁，点击弹出搜索窗口 | 弹窗输入框 + 命中会话列表；标题与摘要高亮命中词，点击结果切换会话 | [ui-v5-search-highlight.png](screenshots/acceptance/ui-v5-search-highlight.png) |

自动预验发现并修复：搜索高亮把字符数当作字节数映射，CJK 文本下范围越界导致进程 panic（窗口空白）。改为逐字节映射并在摘要中复用同一范围计算；新增 Unicode 与多字节摘要测试（42 passed）。

仍待人工确认：图标 tooltip/点击手感、引擎切换后新建会话是否符合使用习惯、搜索弹窗长列表滚动。

## 第六轮：2026-10-22 输入与引擎交互细节

| 反馈 | 实现 | 证据 |
| --- | --- | --- |
| 工作区右侧菜单向右弹出 | 工作区菜单锚点改为 `Anchor::LeftCenter`，菜单出现在行右侧 | [ui-v6-workspace-menu-right.png](screenshots/acceptance/ui-v6-workspace-menu-right.png) |
| 每个工作区默认展示最近 N 条会话，设置可调 | `UiSettings.sessions_per_workspace`（默认 5，范围 1–30）；侧栏超出显示“显示更多（还有 X 条）”并可展开/收起；设置 → 通用 → 界面提供步进器，点击“+”后 `state.json` 持久化为 6 | [ui-v6-session-limit.png](screenshots/acceptance/ui-v6-session-limit.png)、[ui-v6-session-expanded.png](screenshots/acceptance/ui-v6-session-expanded.png)、[ui-v6-settings-session-limit.png](screenshots/acceptance/ui-v6-settings-session-limit.png) |
| Enter 发送、Ctrl/Shift+Enter 换行 | 在 `ConduitComposer > Input` 与 `ConduitComposer` 绑定 Enter→SendPrompt、Ctrl/Shift+Enter→Newline；合成按键验证 Ctrl+Enter 插入换行（两行草稿），Enter 在新会话中发送成功并生成用户消息 | [ui-v6-ctrl-enter-newline.png](screenshots/acceptance/ui-v6-ctrl-enter-newline.png)、[ui-v6-enter-send.png](screenshots/acceptance/ui-v6-enter-send.png) |
| 去掉快捷键提示，引擎下拉移到该位置、向上弹出并同步右上角模型选择器 | 输入区左下角引擎下拉（`Anchor::BottomLeft` 向上弹出）；切换后切到该工作区该引擎最近的未归档会话，右上角模型选择器随会话同步（截图中切到 Codex 后模型选择器显示 GPT-5.6-Sol + Codex） | [ui-v6-composer-engine.png](screenshots/acceptance/ui-v6-composer-engine.png)、[ui-v6-engine-popup.png](screenshots/acceptance/ui-v6-engine-popup.png)、[ui-v6-engine-sync.png](screenshots/acceptance/ui-v6-engine-sync.png) |

预验中发现并修复：工作区行“+”按钮点击冒泡到行选择，导致新建会话后选中被覆盖；已停止该按钮的点击传播。`cargo test` 43 passed。

仍待人工确认：Enter/Ctrl+Enter 在真实中文输入法组合态下的行为（合成按键不等同 IME）、模型选择器与引擎下拉的手感、会话数上限在长列表下的滚动表现。

## 仍未覆盖，必须人工完成

清单 1、2、5、6、8、9 全项；3 的复制一致性；4 的动态滚动跟随；7 的 100%–200% DPI 与多屏；10 的取消检测与文件选择；5 的真实审批允许/拒绝与多审批队列。人工完成后把结果写回 [验证记录](m0-validation.md#手工-ui-验收清单)，不得用本预验代替。





