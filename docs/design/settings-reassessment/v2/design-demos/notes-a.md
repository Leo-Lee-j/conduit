# A 第二轮：IDE 属性面板

交付文件：`dir-1.html`。仅新增本方向 HTML 与本说明；保留 v1、src 与共享研究文件。

## 结构与设计理由

- 184px 固定文字目录、密集属性编辑区、280px 即时预览。四域为工作台、AI与工作流、扩展能力、应用维护，十一分类均可访问。
- 默认同时显示主题 3 项、界面 2 项、聊天 3 项、代码 4 项、背景 1 项，共 13 项真实参数。没有多层字体标签，跨区域比较无需切换页面。
- 属性按「名称 / 当前值 / 作用范围」对齐，宽屏名称列固定约 30%，剩余空间优先给作用范围；分组标题附就地重置。桌面控件主要 30px、属性行 36px、分组间距 12px；正文 14px，辅助说明 12px。
- 数值输入和字体对照摘要使用 Consolas / Cascadia Code，数值保持 14px；中文属性名继续使用界面字体，控件高度不变。
- 深中性灰用于工具界面，细线与 2–4px 圆角建立分层。青绿只用于选中、焦点、修改状态和作用范围，不使用装饰插图、虚构配置键或连接指标。
- 右侧含真实排印场景：界面工具栏、聊天正文与输入示例、独立代码块。下方显示当前作用区域与三类字体当前值，不增加装饰性卡片。
- 编辑区保持深灰，主题模式和浅深主题即时作用于右侧应用预览；「跟随系统」读取浏览器配色偏好。选择主题时作用区域说明明确此预览范围。
- 默认值严格采用 v2 spec：跟随系统；界面 Segoe UI 14px；聊天 Segoe UI 15px、行高 1.6；代码 Cascadia Code 13px，行号与高亮开启；背景默认。
- 不设保存按钮。修改即时进入内存并反馈「预览已更新」，刷新恢复默认，不写入应用。

## Selectors 与交互

| 功能 | 选择器 / 操作 |
| --- | --- |
| 十一分类 | `.nav-item[data-page]`；完整结构示例为 `models`、`sessions`、`engines` |
| 13 项属性 | `.property-row[data-field]`；分组 `.property-group[data-group]` |
| 主题模式 | `#theme-mode button[data-value="system"]` / `light` / `dark` |
| 浅深主题 | `#light-theme`、`#dark-theme` |
| 界面字体字号 | `#ui-font`、`#ui-size` |
| 聊天字体字号行高 | `#chat-font`、`#chat-size`、`#chat-line` |
| 代码字体字号 | `#code-font`、`#code-size` |
| 行号 / 高亮 | `#code-numbers`、`#code-syntax`；`role="switch"` 与 `aria-checked` |
| 数值步进 | 各 `#row-*` 内 `[data-step="minus"]` / `plus`；支持数值输入与边界约束 |
| 背景 | `#chat-background button[data-value="default"]` / `gray` / `green` |
| 各组重置 | `[data-reset="theme"]` / `ui` / `chat` / `code` / `background`；仅重置该组 |
| 修改标记 | `#字段ID-changed`；仅与初始值不同时显示「已改」 |
| 搜索 | `#settings-search`；Ctrl/Cmd+K 聚焦并选中原查询；归一化大小写、全角与空白，支持连续中文及空格分词；方向键与 Enter，Escape 关闭 |
| 搜索空结果 | `#search-results .search-empty` 内「清空搜索」按钮恢复 |
| 搜索定位 | 跳转分类与具体字段，滚动、聚焦，并短暂标记 `.located` |
| 预览开关 | `#toggle-preview`、`#preview-rail`、`#settings-body[data-preview]` |
| 字体与主题预览 | `#application-preview`、`#conversation`、`#code-preview` |
| 当前作用范围 | `#active-group`、`#scope-description`、`#locate-active` |
| 自动更新状态 | `#preview-status`、`#change-count` |
| 新会话默认服务 | `#default-service`、`#reset-default-model`；重置不影响服务列表 |
| 服务示例增改 | `#add-service`、`[data-service-id]`、`#service-dialog`、`#service-name` |
| 手机分类目录 | `#menu-button`、`#sidebar.is-open`；Escape 关闭，打开时 Tab 焦点循环 |
| 返回工作台示例 | `#close-settings`、`#workbench`、`#reopen-settings` |

## 第一轮截图后修正与复测落点

- `代码字号` / `代码 字号` / `字号 代码` 均匹配 `code-size`；`聊天字号`、`界面字体`、`聊天行距` 增加短名称别名。
- Ctrl/Cmd+K → 输入「代码字号」→ Enter，目标为实际数值框 `#code-size`；输入法组词期间的 Enter 不触发跳转。主题、背景结果落到对应组内第一个可操作按钮。
- 属性编辑不重建控件 DOM。数值输入时保留当前编辑文本，失焦才规范化；步进按钮到达上限或下限而禁用时，把焦点移到同一数值框，避免丢失键盘落点。
- 字体选择、开关、组重置更新数值和预览，不主动转移焦点。宽屏列宽与 14px 等宽摘要由父代理统一复测，无共享截图或 polish-log 写入。

## 响应式与边界

- 1440×900：184px 侧栏、主属性区、280px 预览，全部 13 项默认参数同屏；字体调大后允许内部滚动。
- 1100×800：保留三栏，省略作用范围说明列，作用范围仍通过预览说明与控件可访问描述获取。
- 800×800：保留 184px 分类目录，默认收起预览，点击预览在参数列表下展开，避免挤压属性控件。
- 375×812：目录改抽屉，预览默认收起，表单保持名称 / 值两列；控件扩大至约 36px，导航约 40px；底部状态栏 34px 单行。
- 代码字号按 spec 使用 13px，不把代码字号误用为全局正文。代码缩放范围 11–20px，界面 14–18px，聊天 14–22px；界面名称不改成等宽字体。

## 诚实范围与交接

- 所有示例值均为设计值，不读取用户配置。系统字体未安装时回退，不下载字体；真实 Conduit logo 从仓库 PNG 内嵌。
- 属性编辑器是本方向的深灰固定外壳，主题切换验证右侧应用场景，不将切换效果误报为写入原生应用。
- 服务仅演示本地 CLI 名称与引用关系，不请求网络、不检测在线状态、不编造模型型号。浏览 AI 引擎不修改默认值。
- 新会话默认不改变已有会话；服务编辑明确提示共享 ID 对已有引用的后续影响。
- 其他分类明确显示「结构示例」，工作台返回同样为结构预览，不虚构完整业务实现。
- 不创建共享验证文件。截图与多尺寸统一 QA 由父代理负责；可直接使用上述稳定 selectors 验证。
