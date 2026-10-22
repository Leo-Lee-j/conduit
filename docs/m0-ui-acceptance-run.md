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
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-models -AppArgs '--settings' -ClickAt '251,378'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-models-codex -AppArgs '--settings' -ClickAt '251,378' -ScrollAt '700,500,20'
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-detect -AppArgs '--settings' -ClickAt '243,273;933,394' -SettleMs 5000
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name settings-narrow -AppArgs '--settings' -Width 850 -Height 500
./scripts/smoke.ps1 -View chat
./scripts/smoke.ps1 -View settings
```

`ui-acceptance.ps1` 以 UTF-8 BOM 保存；用 Windows PowerShell 5.1 执行时改为 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\ui-acceptance.ps1 ...`，否则会因中文编码或执行策略解析失败（2026-10-22 已修复并双版本验证）。

坐标参数与 1216×839 窗口（100% 缩放）对应；不同缩放下需按截图重新定位。脚本只做截图与点击证据，不改变生产数据。

## 仍未覆盖，必须人工完成

清单 1、2、5、6、8、9 全项；3 的复制一致性；4 的动态滚动跟随；7 的 100%–200% DPI 与多屏；10 的取消检测与文件选择；5 的真实审批允许/拒绝与多审批队列。人工完成后把结果写回 [验证记录](m0-validation.md#手工-ui-验收清单)，不得用本预验代替。
