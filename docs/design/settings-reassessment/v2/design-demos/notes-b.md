# B 第二轮 · 预览检查器

文件：`dir-2.html`。仅写入本方向 v2 HTML 与本 notes；没有修改第一轮或共享文件。

## 结构与密度

- 1440px 桌面：56px 领域轨 + 150px 全部分类 + 878px 阅读预览 + 356px 检查器。原先的左编辑、右预览已反转为中间预览、右侧检查器。
- 右侧五组同时展开：主题 3 项、界面 2 项、聊天 3 项、代码 4 项、背景 2 项，共 14 项；无需切换文字标签才能查看其他字体组。
- 主题模式使用三段控件；浅色与深色主题使用色样选项，标题旁以「配色样本」明确标注为本次设计演示。字体使用本机选择框；字号及行高采用滑块与数字输入并列，既能试调也能精确填写。
- 桌面属性行 36px、普通输入 30px、分段按钮 24px（外框 30px）、开关命中区 30px。正文 14px，辅助标签 12px；代码样本按规范默认 13px。
- 使用低饱和墨蓝、细分隔线和品牌青绿。技术感来自属性对齐、独立参数与直接反馈，没有假终端或虚构的应用配置键。
- 聊天和代码展示真实可读的 TypeScript 排序样本，不以配置说明代替工作内容。字体调整直接作用于对应内容。
- 仍完整展示全部 11 个分类，归属四域：工作台（通用、外观与字体、快捷键）；AI与工作流（AI 引擎、模型与服务、会话设置、Git 工作流）；扩展能力（Skills、MCP 服务、插件）；应用维护（数据与更新）。领域轨 title / aria-label 使用完整域名，显示简短标签。选择领域不隐藏其他分类；进入 AI 引擎页只浏览结构，不改变默认引擎。

## 默认值及截图口径

默认值与 v2/spec.md 一致：跟随系统；界面 Segoe UI / 14px；聊天 Segoe UI / 15px / 1.6；代码 Cascadia Code / 13px；行号、高亮开启；默认背景。

**跟随系统是真实媒体查询，不伪装固定深色。** 为观察 B 的墨蓝设计方向，统一截图时可将浏览器 `prefers-color-scheme` 设为 `dark`；也可点 `[data-choice="theme"][data-value="dark"]`。在浅色系统下默认正确展示浅色，仍可切换深色。

## 选择器与交互

| 场景 | 选择器 / 行为 |
|---|---|
| 四个领域 | `[data-domain]`：workspace / ai-workflow / extensions / maintenance |
| 11 分类 | `#category-nav [data-category]`；当前项有 `aria-current="page"` |
| 窄屏分类 | `#category-select`，≤920px 显示完整分类下拉 |
| 14 项检查器 | `#inspector-content .property-row`；默认一次渲染全部 14 行 |
| 主题模式 | `[data-choice="theme"][data-value="system|light|dark"]` |
| 配色样本 | `[data-choice="lightPalette"]` / `[data-choice="darkPalette"]` |
| 界面字体 / 字号 | `#setting-ui-font` / `#setting-ui-size` |
| 聊天字体 / 字号 / 行高 | `#setting-chat-font` / `#setting-chat-size` / `#setting-chat-line` |
| 代码字体 / 字号 | `#setting-code-font` / `#setting-code-size` |
| 对应滑块 | 数字输入 ID 追加 `-range`，例如 `#setting-chat-size-range`；与数字输入双向同步 |
| 行号开关 | `#setting-line-numbers`；`role="switch"` 与 `aria-checked` 同步 |
| 高亮开关 | `#setting-highlight`；关闭后保留代码文本、取消配色 |
| 背景方式 / 颜色 | `#setting-background` / `#setting-background-color`；默认 / 柔和模式禁用颜色输入并保留行；先选择自定纯色才可调色；重置后重新禁用 |
| 分组重置 | `[data-reset-group="theme|ui|chat|code|background"]`；仅恢复当前组 |
| 搜索 | `#settings-search` → `[data-search-index]`；定位真实控件并短暂描边 |
| 检查对象反馈 | `#affected-group`；聚焦控件或修改时反映对应文字组 |
| 状态反馈 | `#update-status` / `#update-detail`；始终采用「预览已更新」，无保存按钮 |
| 内容切换 | `[data-view="conversation"]` / `[data-view="code"]`；仅代码隐藏聊天段落与输入框 |
| 代码选择 | `#select-code`；只选中文字，用户可自行 Ctrl+C，不自动写剪贴板 |
| 追加预览文字 | `#preview-input` → `#composer-form`；默认 Ctrl+Enter，不发往服务、不生成回复 |
| 字号结果 | `#metric-ui` / `#metric-chat` / `#metric-code`；实际样式在 `.preview-heading` / `.chat-text` / `.code-content` |
| 会话页 | `[data-category="sessions"]`；默认引擎、默认模型、发送快捷键结构示例 |
| 模型页 | `[data-category="models"]`；默认模型、共享服务结构示例 |
| 服务增改 | `#add-service` / `[data-edit-service]` → `#service-dialog`，填写 `#service-name` / `#service-url` 后提交 `#service-form` |
| 其他分类 | `.structure-row`，原生 details 结构示例 |
| 快捷键说明 | `#keyboard-help` → `#help-dialog` |

搜索支持 Ctrl+K、Enter 定位首项、上下方向键选择结果与 Escape 收起。搜索纯色值时若该控件禁用，会聚焦填充方式并提示先选择自定纯色，不自动更改背景模式。无结果时说明可清空恢复；搜索不破坏当前表单内容。焦点描边清晰，滑块与数字输入支持键盘，dialog 采用原生焦点约束和 Escape 行为。

## 响应式与边界

- 1100×800：领域轨 48px、分类 132px、检查器 332px，保留并排阅读与调整；检查器可内滚。
- 800×800：保留 48px 领域轨，分类转为顶部下拉；预览与 328px 检查器仍相邻，代码可独立横向滚动。
- 375×812：隐藏领域轨，顶部分类下拉与搜索；检查器在前、阅读预览在后，控件增至 40px、属性行约 49px，页面纵向滚动。
- 大字号或追加预览文本可能产生内部滚动；不以缩小正文满足密度。
- 所有示例状态仅存在于当前页内存，无网络、存储、应用配置、凭据或 CLI 调用。刷新恢复默认。
- 字体为系统回退；未安装的字体不会下载。浅色 / 深色配色名称是当前演示样本；界面以简短「配色样本」标记说明，不表示生产已安装主题清单。
- 次级分类显式标注「结构示例」。不编造连接状态、模型型号或版本；仅使用本地 CLI / 服务默认模型。
- 默认模型仅影响新会话的说明明确；共享服务编辑提醒可能影响同 ID 的既有引用。服务增改只创建本页示例条目。
- Logo 从仓库原图逐字节 base64 内嵌，助手标识复用同一图片。

## 验证

本代理检查内嵌 JavaScript 语法、静态 ID、原图一致性和网络/持久化 API，未运行浏览器截图。四尺寸截图、交互 QA 与最终视觉打磨由主代理统一执行。

## 最终对比度修正与局部例外

- 浅色辅助文字调整为 `--muted: #455a69`、`--quiet: #526573`；深色 `--quiet` 提亮至 `#a0b2bf`，`--muted` 保持 `#a1b3c0`。
- 以实际 CSS 色值核算两种浅色、两种深色样本，每套覆盖 quiet / muted 与 8 种界面表面组合，共 64 对；最低对比度分别为：light-cool 4.71:1、light-warm 4.71:1、dark-ink 5.22:1、dark-graphite 5.22:1，均达到 4.5:1。这是静态色值核算；最终浏览器 lint 由主代理统一复核。
- 中间来源文案改为「本地 CLI（服务默认模型）」；追加消息显示「你（本次预览）」。其他结构与快捷键文案改为自然标点，仅保留共同规范要求的底部「设计预览 · 不写入应用」。
- **局部字号例外**：共同 `v2/spec.md` 明确要求真实代码样本默认 Cascadia Code / 13px，因此 `.code-content`（含行号）与 `.inline-code` 保留 13px，作为可调代码阅读样本，不适用界面正文 ≥14px 的一般阈值。此例外只覆盖真实代码；界面正文仍为 14px、辅助标签为 12px，不扩大豁免范围。
- 主代理已反馈 14 项同屏、四尺寸与关键交互 QA 通过；本次仅修正文案与色值，保留现有布局、控件与默认跟随系统行为。

- 终检补修：浅色 `.token-type` 单独设为 `#28634f`，不改变品牌强调色。对雾白代码底对比度 6.11:1，对暖灰代码底 6.03:1；深色类型配色及代码默认 13px 保持不变。
