# 原生设置 UI 接入

## 本阶段交付

在原有 Rust / GPUI 设置工作区上增量接入，不替换为弹窗，也不迁移配置或改写会话历史。产品、视觉/交互、技术集成和 QA 并行工作，按文件划分写集；已有 `services.rs` 与 `workspace_tests.rs` 用户修改保持不变。

- 公共壳：统一标题、分区标题、文字层级与间距，跟随现有主题，不增加装饰性渐变或逐字段卡片。
- 导航：四个域、11 个可见入口。依据设置主区实际宽度与 UI 字号，选择文字侧栏或带文字的分类下拉；保留主工作台周边面板。
- 外观与字体：外观 / 文字 / 背景三组；界面、聊天、代码字体独立；界面恢复与聊天/代码恢复分开；静态排版预览不会读取会话或执行示例代码。
- 会话设置：默认服务/模型、默认权限、聊天显示、自动命名四组。仅默认服务/模型使用当前浏览引擎；权限与显示偏好有独立范围说明。
- 模型与服务：沿用现有列表/页内详情、复杂编辑表单、凭据保护、未保存提示和异步探测；共享配置编辑展示引用范围。默认切换仍必须明确操作，浏览不改变默认值。
- 搜索：旧分类词及自动命名入口保持可发现；字体、背景、权限、显示与命名结果进入真实对应分组；引擎默认结果携带明确的引擎身份。

## 路由兼容

`SettingsUi.page` 的既有数值身份不变，使用 `SettingsRoute` 明确命名。内部仍有 12 个身份，导航只展示 11 个入口；不能通过删除 route 5 把内部路由数量裁成 11。旧自动命名 route 5 仍可直接进入，导航高亮归到 route 10「会话设置」，但直接进入时仍渲染兼容命名页，而不是把全部页面内容重定向到新会话页。

| ID | 内部身份 | 新导航 |
|---|---|---|
| 0 | General | 工作台 / 通用 |
| 1 | Appearance | 工作台 / 外观与字体 |
| 2 | Shortcuts | 工作台 / 快捷键 |
| 3 | Engines | AI 与工作流 / AI 引擎 |
| 4 | Skills | 扩展能力 / Skills |
| 5 | AutomaticNaming | 兼容入口，归属会话设置 |
| 6 | Git | AI 与工作流 / Git 工作流 |
| 7 | Maintenance | 应用维护 / 数据与更新 |
| 8 | Services | AI 与工作流 / 模型与服务 |
| 9 | Mcp | 扩展能力 / MCP 服务 |
| 10 | Sessions | AI 与工作流 / 会话设置 |
| 11 | Plugins | 扩展能力 / 插件 |

会话设置 route 10 的 group 0/1/2/3 固定为默认服务/模型、默认权限、聊天显示、自动命名。外观的 group 0/1/2 分别是外观、文字、背景。

- 旧 route 5 保留两组语义：group 0 为自动命名开关与触发方式，group 1 为命名引擎、服务和模型配置（含配置摘要与展开）。新会话页将同一套 field / 回调组合到 group 3，不复制存储逻辑。
- 旧命名搜索目标统一映射到 route 10 / group 3，并展开命名配置；兼容路由与搜索落点不是同一件事。
- `settings_defaults_page` 仍是兼容接口；`settings_conversation_page` 从其取得默认 group，重设页面标题/图标后追加其余三组。默认组只构造一次，不再逐引擎复制整套控件。顶部管理栏与模型/服务页共用 `connection_settings.engine`；切换它只改变浏览上下文，不修改默认引擎、认证或已保存的默认选择。

## 生效范围与边界

### 默认值与共享服务编辑必须分开说明

| 操作 | 生效范围 | 不应作出的承诺 |
| --- | --- | --- |
| 设置默认服务 / 模型 | 当前浏览引擎的未来或待发新会话采用这些初始值；显式草稿模型选择沿用既有优先级 | 会改写已有会话的服务引用、模型或消息历史 |
| 设置默认权限 | 应用级默认，供未来或待发新会话采用；已有会话保留原权限 | 会追改已有会话权限，或模型切换等同授权变更 |
| 编辑同 ID 的共享服务 | 引用该 ID 的新旧会话在后续使用 / 启动时可能读取更新后的端点、模型配置或凭据配置 | “共享服务编辑只影响新会话”，或正在运行的 CLI 一定会热更新 |
| 查看服务 / 切换浏览引擎 | 只改变设置页的浏览对象 | 已设为默认、已完成认证，或已切换原生 CLI 当前配置 |

默认值改变的是“新会话采用哪些初始值”；共享服务编辑改变的是“同一个引用 ID 后续解析到哪些配置”。修改默认服务/模型不会追改旧会话，但修改被旧会话引用的共享服务可能影响其后续使用，因此保存说明必须保留引用范围提示。本阶段没有新增服务版本化或会话专属配置快照，也不承诺正在运行的进程会立即采纳编辑。

### 服务不可用与模型待确认不是同一种状态

- **服务不可用**：当前引擎的 registry 中没有可匹配的已保存来源。保留 saved `ModelChannel` 与原 ID，显示不可用并引导显式重选，不静默迁移、清值或替换默认。
- **Legacy CCS 兼容**：`ModelChannel::Ccs(sourceID)` 显示时按 `engine + ModelProvider::ccs_id(engine, sourceID)` 解析到同引擎、已存在的 registry 条目。存在时可正常显示该服务；不存在或只有其他引擎的同 source ID 时仍不可用。此解析只作用于显示，保留原 `Ccs(sourceID)` 与模型 ID，不写配置迁移。
- **模型待确认**：已保存模型未出现在当前可见目录中，不足以证明模型不可用。目录可能尚未加载、不完整或刷新失败；现有 `ModelCatalog` 没有统一且权威的 Loaded 状态，部分失败还可能记录在 `note` 而非 `error`。此时保留原模型 ID 作为标签并提示待确认，不仅凭目录未命中断言不可用，也不自动清空或换成服务默认模型。
- 用户明确重选服务 / 模型或选择“使用服务默认模型”属于主动配置操作，与上述状态展示分开。本轮没有改变真实 CLI 启动的 fallback 策略；保留配置不等于承诺启动一定成功，也不等于承诺永不 fallback。

### 聊天显示、命名与写入边界

- 推理过程与问题历史折叠属于应用级显示偏好，即时影响现有聊天视图；不改变模型输出、已发送内容或会话权限。
- 自动命名的 first 策略在首个完成轮次触发；latest 策略可在已有会话的后续完成轮次更新标题。两者都不批量重命名历史会话。
- 关闭自动命名只取消 automatic 任务，保留 manual 任务、手动 AI 重命名能力及命名配置。自动触发控件停用不等于独立命名模型配置不能供手动命名使用；命名模型配置也不会更改会话的聊天模型。
- 持久化 setter 保留只读 / closing guard 与 persist。只读实例仍可导航、搜索和切换浏览引擎，不能借浏览操作写入默认值或服务配置。

## 验证方式

本阶段以原生 GPUI 行为测试、发布配置构建和隔离 Windows 窗口检查为准；HTML 原型的 152 项检查不能算作原生验收。下表区分自动化结果与有限的本机窗口观察；截图不是完整人工签收。

- 翻译、格式：`python scripts/check-i18n.py`、`cargo fmt --all --check`。
- 设置回归：`cargo test --locked --features ui-tests settings:: -- --test-threads=1`。
- 原有连接 UI：`cargo test --locked --features ui-tests --bin conduit app::ui_tests::settings_connections_switch_engines_and_guard_service_editors -- --test-threads=1`。
- Clippy：`cargo clippy --locked --all-targets --features ui-tests -- -D warnings`。
- 发布构建：`cargo build --release --locked --bin conduit`。
- 原生测试覆盖：分类的键盘导航与只读浏览、旧路由和搜索兼容、跨宽度草稿/输入 Entity/引擎默认保留、字体独立恢复及写入边界，以及 Legacy CCS 默认来源的只读解析、同 source ID 跨引擎隔离与配置不迁移。
- Windows 窗口检查使用 `target/settings-native-acceptance/capture-isolated.ps1` 与临时 workspace；合成服务、未配置的演示凭据，独立 `USERPROFILE` / CLI 配置目录，不连接真实模型服务。

## 当前验收状态

自动化结果于 2026-11-23 在 Windows 本地工作区执行。设置专项包含新加的跨断点弹层焦点回归；截图是本机隔离配置窗口的目视检查。此表记录工程验证进度，不是完整人工签收。

| 项目 | 当前状态 | 证据 / 回填责任 |
| --- | --- | --- |
| 设置专项 | 33 passed / 0 failed / 1 ignored | `cargo test --locked --features ui-tests settings:: -- --test-threads=1`；ignored 是要求真实 Windows 桌面与 GPU 的宠物设置快照 |
| 弹层跨断点焦点回归 | 4 passed / 0 failed | 包括 rail→compact、compact→rail 返回焦点、紧凑分类菜单关闭和服务编辑器输入/草稿保留 |
| 旧连接 UI 用例 | 1 passed / 0 failed | `settings_connections_switch_engines_and_guard_service_editors` |
| Clippy | 通过 | `cargo clippy --locked --all-targets --features ui-tests -- -D warnings` |
| 格式与差异 | 通过 | `cargo fmt --all --check`、`git diff --check`；Git 提示若干已有 LF 文件未来可能转换为 CRLF |
| i18n | 通过 | `python scripts/check-i18n.py`，3351 条消息 × 中英两语言 |
| Debug 构建 | 通过 | `cargo build --locked --bin conduit` |
| Release 构建 | 通过 | `cargo build --release --locked --bin conduit`；产物 `target/release/conduit.exe` |
| 隔离窗口截图 | 有限目视检查完成 | `target/settings-native-acceptance/out/`；未连接真实模型服务或真实 CLI |
| 全仓库 UI 测试套件 | 897 passed / 5 failed / 26 ignored | `cargo test --locked --features ui-tests -- --test-threads=1`；3 项失败引用了外观/背景旧 group 编号（背景组现在为 group 2），2 项为聊天推理显示断言。设置专项和焦点专项均独立通过；上述 5 项未在本轮修复或复跑 |

### 发布构建与截图验收待补

- **构建证据**：Release profile 编译成功；该命令是本地 Windows 构建，不代表签名、安装包、发布流水线或其他目标平台已验证。
- **截图元信息**：`typography-narrow-light-font20.png` 由 Debug 构建采集，窗口 1000×800、浅色、UI 字号 20、可写隔离演示状态。当前采集脚本没有记录 DPI；不要据此推断多档 DPI 适配。
- **本地观察记录**：宽设置主区的服务与会话截图、服务详情/编辑器、紧凑服务页，以及 1000×800 字体窄窗均保存在 `target/settings-native-acceptance/out/`。新字体截图目视可见完整字体选择框、字号步进控件与可滚动内容；这是单次目视记录，不是自动化像素/几何断言。

| 待核查场景 | 真实截图 / 窗口信息 | 观察结果 |
| --- | --- | --- |
| 宽主区：会话默认设置、服务列表/详情/编辑 | `sessions-wide-dark.png`、`services-wide-dark.png`、`services-shared-editor.png`，1600×1000 | 已目视检查基本层级和编辑弹层；不代表完整业务流程签收 |
| 窄主区 / 大字号：分类下拉与字体控件 | `services-compact-dark.png`、`services-compact-detail.png`、`typography-narrow-light-font20.png`；字体场景 1000×800、20px | 分类菜单通过真实坐标打开；字体页目视确认控件未被裁切。窄窗键盘焦点以 4 项 GPUI 回归验证，未做全面键盘人工巡检 |
| 主题与只读实例 | `appearance-wide-light-font20.png`、`appearance-preview-light-font20.png` | 仅浅色宽窗有截图；只读边界由自动化覆盖，未做完整只读窗口目视检查 |
| 失效服务、模型待确认、有效 Legacy CCS 来源 | 无专门窗口截图 | 数据保持和只读解析由测试覆盖，目视场景未验收 |
| 共享服务引用范围、旧 route 5 与命名搜索落点 | `services-shared-editor.png` | 编辑器截图可见引用范围说明；route / 搜索落点由测试覆盖，未对每个状态人工巡检 |

IME、辅助技术、多档 DPI 和真实 CLI 并发仍须按 `docs/testing.md` 人工验收。未完成或未覆盖的项目保留为待补 / 未验收，不从自动化通过结果推断为人工验收通过。

设计方法参考 Unslop by Damir Sokolovsky（@heydamir，CC BY 4.0）。实际界面延续 Conduit 主题与图标体系，没有引入生成图片。
