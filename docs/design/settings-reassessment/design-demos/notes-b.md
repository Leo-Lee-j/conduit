# B · 预览工作台

交付：`dir-2.html`，独立 HTML / CSS / JavaScript，无框架、无外部资源。默认打开「外观与字体 → 文字 → 聊天」。只读了共同规范、品牌规范与 B 方向卡，没有参考另外两版。

## 设计理由

- 宽屏保留 176px 项目上下文、184px 设置分类。余下工作区域按 43:57 分配设置与预览，阅读结果得到最大有效面积。
- 设置分类完整列出 11 项；外观内部先按主题、文字、背景分区，再把文字分成界面、聊天、代码三个互相独立的标签。
- 深石墨表面以细边界区分层级。青绿色只用于选中、焦点和操作反馈。没有渐变、装饰性插画或大面积介绍文案。
- 主视觉来自用户问题、助手正文、真实排版代码和输入框。代码内容同时显示当前字号与行距值，控件变化有两种可直接观察的反馈。
- 初始聊天页面只保留字体、字号、行距与当前文字组重置，主题和背景从页内标签进入，避免在首屏重复堆放相同入口。修改后即时反馈自动保存，仅作用于本次预览，不额外要求保存。
- 所有界面元素都使用本机字体。正文最低 14px，辅助标签最低 12px；主要按钮、范围增减和开关命中区为 44px。

## 主要交互选择器

| 场景 | 选择器与操作 |
|---|---|
| 全部分类 | `#settings-nav [data-category]`，11 个分类；当前项带 `aria-current="page"` |
| 手机分类 | `#category-select`，375px 下替换纵向导航 |
| 外观分区 | `[data-section="theme"]` / `[data-section="text"]` / `[data-section="background"]` |
| 独立字体组 | `#font-ui` / `#font-chat` / `#font-code` |
| 聊天字体、字号、行距 | `#setting-chat-font` / `#setting-chat-size` / `#setting-chat-line` |
| 界面字体、字号 | `#setting-ui-font` / `#setting-ui-size` |
| 代码字体、字号 | `#setting-code-font` / `#setting-code-size` |
| 代码行号、语法高亮 | `#setting-line-numbers` / `#setting-highlight`，`role="switch"` |
| 增减字号、行距 | `[data-step]`，对应范围滑块同步更新 |
| 主题模式 | `[data-choice="theme"][data-value="system|light|dark"]` |
| 浅色、深色主题 | `[data-choice="lightPalette"]` / `[data-choice="darkPalette"]` |
| 背景及自定颜色 | `[data-choice="background"]` / `#setting-background-color` |
| 当前组重置 | `#reset-group`；主题或背景页为 `#reset-section` |
| 全部重置 | 数据与更新 → `#reset-all` → `#confirm-reset`；同时清除示例服务和追加文字 |
| 搜索并定位 | `#settings-search` → `[data-search-result]`；示例关键词「行距」「代码字号」「默认模型」 |
| 扩展阅读现场 | `#expand-preview`，再次点击恢复设置与预览 |
| 选择示例代码 | `#copy-code`，选中文字并提示使用 Ctrl+C，不自动写剪贴板 |
| 输入预览文字 | `#preview-input` → `#composer-form` 提交；只向示例会话添加本人输入，不生成回复 |
| 会话结构 | `[data-category="sessions"]`；默认引擎、默认模型及 Enter / Ctrl+Enter 示例 |
| 模型服务结构 | `[data-category="models"]` → `#add-service` → `#service-dialog` |
| 服务编辑 | `#service-name` / `#service-url` / `#service-model`；提交 `#service-form`；已有条目通过 `[data-edit-service]` 编辑 |
| 自动保存表达 | `#change-status`，修改即时生效并自动保存在当前页面内存；无额外保存或保留按钮，刷新恢复默认 |
| 其他结构页 | `.structure-row` 原生 details，可用键盘展开 |

搜索支持 Ctrl+K、Enter 定位首项、上下方向键选择结果、Escape 收起。定位后聚焦实际控件并短暂描边。两层页内标签支持左右方向键及 Home/End。服务与重置使用原生 dialog，支持焦点约束、Escape 和关闭后焦点恢复。

## 响应式策略

- 1440×900：176px 项目上下文 + 184px 分类 + 43:57 编辑/预览；左右区域独立滚动。
- 1100×800：隐藏项目上下文，保留分类与并排设置/预览。
- 800×800：保留 184px 分类，编辑和预览上下堆叠，整页滚动。
- 375×812：分类改为下拉；设置在前、预览在后；代码区域独立横向滚动，页面不依赖横向滚动。
- 尊重系统减少动态效果偏好。跟随系统主题会响应系统明暗变化。

## 局限与事实边界

- 所有设置、服务条目和聊天输入只存在于当前页面内存。没有应用配置读写、持久化、网络请求、凭据访问或 CLI 执行。
- 服务默认模型与本地 CLI 是示例来源，不展示连接成功、安装检测、模型型号或版本状态。服务地址不进行请求或可用性验证。
- 会话页与服务页为可操作结构示例，其他分类为可展开的分组示例，不宣称已实现生产功能。
- 默认模型文案明确只影响新会话；编辑共享服务提示引用同一 ID 的既有会话后续使用可能受到影响。
- 「中性浅灰 / 柔和纸白 / 深石墨 / 墨黑」是此设计稿的配色样本，不是对现有生产主题清单的断言。
- 未安装的字体由系统回退，页面不做字体安装探测；选择相近或缺失字体时差异可能不明显。
- 自定背景通过亮度选择黑/白正文；字体和字号仍保持独立。代码示例允许在手机端横向滚动。
- 高字号或追加文字会增加预览内部滚动；窄屏的完整预览在表单之后。
- 原图从 `assets/app-icons/source.png` 逐字节转为 base64；助手标识复用同一图片，不重画 logo。

## 本次检查

- 已检查脚本语法、静态 ID 唯一性、原图与内嵌图片一致性、外部资源引用，以及网络/持久化 API。
- 截图、四尺寸视觉打磨与浏览器交互验收由主代理继续执行；本 notes 不宣称已经完成视觉验证。
