# 工作台界面插件扩展性评估

评估日期：2026-11-04。依据：当前工作区源码，基准提交 `96f681c`，包含评估时已有的未提交修改。本文描述拟议方案；没有实现或运行插件原型，工期和性能目标均为规划估算。

本评估已细化为[三阶段设计文档](plugins/README.md)。实施时以阶段文档的接口与事务约定为准；其中明确由独立插件注册表持久化启用与授权，UiSettings 只保存布局偏好。下文保留初始评估与示意清单。

**2026-11-04 补充：初版遗漏了当前依赖仓库中的 GPUI Shell。** [复用评估](plugins/gpui-shell-reuse-assessment.md)已核对官网与锁定提交，当前建议改为先验证 Shell，保留独立进程作为备选。下文“首个动态版本采用进程”、固定模板和 21–34 人日均属于原进程路线，不是 Shell 的选型结论或预算；实施入口以更新后的阶段总览为准。

**建议支持这三个区域的扩展，先建立统一的注册与命令机制，再提供受信任的本地插件运行时。** 三处布局已有清晰的渲染入口，可以逐步接入。主要工作集中在动态视图身份、上下文隔离、生命周期和宿主 API。若目标仅是让团队增加内置功能，完成第一阶段即可；若要安装独立发布的第三方插件，需要继续完成运行时及管理能力。

这里将“左侧图标功能区域”理解为工作区 / 文件 / Git 所在的竖向图标栏及其关联页面；“右侧侧边栏”指当前工具详情；“底部工具栏”指中间主内容区下方的状态与操作栏。底部新增可展开面板、终端及任意位置停靠，需要另行扩展布局范围。

当前实现与接入成本如下。

| 区域 | 当前源码证据 | 建议扩展点 | 相对成本 |
| --- | --- | --- | --- |
| 左侧图标区 | [files.rs](../src/app/files.rs) 的 `Navigation` 固定为 Workspaces / Files / Git；`render_navigation` 手写按钮并通过 `match` 选择页面 | `activityItems` 声明图标、提示、徽标、排序及动作；关联 `navigation` 视图或命令 | 中：需要提取导航身份和状态，并支持动态视图 |
| 右侧侧边栏 | [dock.rs](../src/app/dock.rs) 的 `render_detail_dock` 专用于工具详情；[state.rs](../src/app/state.rs) 的 `DetailView` 只有 Tool | `right` 视图容器统一处理标题、切换、关闭、宽度、加载和错误状态 | 中高：容器、工具详情内容和会话状态需要分离 |
| 底部工具栏 | [toolbar.rs](../src/app/toolbar.rs) 使用 `StatusBar.left/right`，展示工作区、分支、运行状态、推送和性能入口 | `statusItems` 声明左右位置、优先级、文字、图标、提示和命令 | 低到中：布局易接入，动态刷新与拥挤处理仍需设计 |

[shell.rs](../src/app/shell.rs) 将工具栏加入中间内容区，再与右侧面板拼接；它目前不是贯穿整个窗口的全局状态栏。右侧在窄窗口下切换为浮层，宽窗口下独立调整宽度。建议首版沿用这些行为；插件的“全局 / 工作区 / 会话”数据作用域应与它在窗口中的物理位置分别定义。

[app.rs](../src/app.rs) 集中持有 `selected_workspace`、`selected_session`、`details`、`detail_view`、`detail_text`、`detail_width` 等状态。左侧模式和折叠状态归属 `FileBrowser`。当前增加视图仍会涉及这些固定字段和分支，适合先引入按标识查找的注册表。

现有 [extensions.rs](../src/extensions.rs) 的 `Kind` 为 Skill / Mcp，[设置扩展页](../src/app/settings/extensions.rs) 提供目录扫描、配置编辑入口、Skill 导入和 MCP 连接测试。这些可以作为插件管理交互的参考，但尚不具备 Conduit 界面插件的注册、加载或运行协议。建议将新模块命名为 `plugins`，并在产品中分别使用“应用插件”和“Skills / MCP”。

**统一扩展协议可以让一个插件同时贡献三个区域的功能。** 例如“工作区概览”插件在左侧提供文件分类入口，在右侧显示选中文件的统计，底部提供概览按钮。它们共享插件身份、上下文和命令，不需要三套加载系统。

| 拟议组件 | 职责与边界 |
| --- | --- |
| `PluginManifest` | 插件 ID、版本、宿主 API 兼容范围、入口程序、声明权限、激活条件及扩展项 |
| `ContributionRegistry` | 管理活动图标、视图、状态项；校验命名空间和引用；统一排序、隐藏和注销 |
| `CommandRegistry` | 使用字符串命令 ID；处理参数、上下文、权限与执行状态；适配现有 GPUI action |
| `PluginHost` | 按需激活、协议握手、后台通信、超时、停止、错误恢复和日志 |
| `PluginContext` | 提供经授权的工作区 / 会话摘要、当前文档、主题与只读状态；带上下文版本 |
| 宿主视图渲染器 | 接收有界数据，在 GPUI 中绘制预设列表、键值信息和 Markdown；保留焦点与滚动状态 |
| `PluginStorage` | 按插件及工作区划分设置和数据，提供版本与容量限制 |

内置功能可以通过内部适配器注册原生视图，外部插件通过可序列化的数据协议贡献内容。GPUI 实体、窗口上下文和 `&mut ConduitApp` 留在宿主内部。原有业务逻辑可以逐个接入适配器，无需一次迁移所有内置功能。

三个区域的交互规则建议如下。

1. 左侧图区分功能入口和底部系统入口。插件声明默认顺序，用户的隐藏 / 固定 / 排序偏好覆盖默认值；图标超量进入更多菜单。图标可打开关联导航页，也可执行命令。不要让“图标 ID”“视图 ID”和“命令 ID”成为同一个隐含概念。
2. 右侧支持多个已注册视图，首版同时显示一个，通过标题区菜单或标签切换。工具详情作为内置视图注册；用户显式点击工具详情时切换，后台工具事件只更新数据。保留现有窄屏浮层、拖动限宽、独立滚动和关闭行为。
3. 底部提供左右扩展区域，运行与错误信息拥有宿主保留空间。状态项按优先级折叠，限制文字宽度及更新频率；点击可以执行命令或打开对应视图。首版不增加完整的底部面板容器。
4. 视图与图标的隐藏条件只控制显示。执行命令时重新校验上下文、宿主可写状态、业务限制和授权，避免通过直接命令调用绕过限制。首版只支持有限的上下文布尔条件，避免引入任意表达式执行。

下面是接口形状示例，字段均为设计草案，当前应用不能加载此文件。它展示一个插件如何关联左侧列表、右侧信息页和底部入口。

```json
{
  "id": "example.workspace-overview",
  "version": "0.1.0",
  "hostApi": "^1.0.0",
  "runtime": {
    "kind": "process",
    "program": "bin/workspace-overview.exe"
  },
  "permissions": ["workspace.metadata.read"],
  "activationEvents": [
    "onView:example.workspace-overview.index",
    "onView:example.workspace-overview.summary",
    "onCommand:example.workspace-overview.showSummary"
  ],
  "contributes": {
    "commands": [
      {
        "id": "example.workspace-overview.showSummary",
        "title": "打开工作区概览"
      }
    ],
    "activityItems": [
      {
        "id": "example.workspace-overview.activity",
        "title": "工作区概览",
        "icon": "folder",
        "order": 100,
        "view": "example.workspace-overview.index",
        "when": "workspace.open"
      }
    ],
    "views": [
      {
        "id": "example.workspace-overview.index",
        "location": "navigation",
        "title": "工作区概览",
        "renderer": "list"
      },
      {
        "id": "example.workspace-overview.summary",
        "location": "right",
        "title": "概览详情",
        "renderer": "keyValue"
      }
    ],
    "statusItems": [
      {
        "id": "example.workspace-overview.status",
        "alignment": "right",
        "priority": 20,
        "text": "工作区概览",
        "command": "example.workspace-overview.showSummary",
        "when": "workspace.open"
      }
    ]
  }
}
```

激活流程建议为：扫描并校验清单 → 注册静态入口 → 用户首次打开视图或执行命令 → 启动该插件的进程 → 握手校验 API 版本 → 请求当前上下文的数据 → 宿主绘制。底部静态按钮可以在插件进程尚未启动时展示；后台持续更新应另行声明激活条件，不能因注册一个状态项就默认常驻。

进程通信可以采用独立的 stdio JSON-RPC 协议；协议需要补齐消息边界、请求 ID、取消、版本协商、最大消息长度及错误响应。插件 stdout 专用于协议，stderr 进入有界日志。首版使用完整视图快照和少量状态更新消息；频繁变化的数据按视图合并更新。不要直接把 CLI 原始事件作为公开插件 API。

**运行时选择建议与目标分层。** 以下是工程判断，实际资源开销仍需原型验证。

| 方案 | 适用目标 | 本项目中的判断 |
| --- | --- | --- |
| 随应用编译的 Rust 模块 | 团队内部增加功能、验证扩展 API | 第一阶段采用；仍需随应用构建发布 |
| 受信任的独立进程 + 数据协议 | 本地安装、独立更新、可用多种语言开发的插件 | 首个动态版本采用；可借鉴现有进程和取消机制，增加内存与进程启动成本 |
| WASM / WASI + 显式宿主接口 | 需要严格控制可访问能力的第三方代码 | 作为后续单独验证项；需要运行时、SDK、资源配额及完整能力边界 |
| 动态 Rust DLL 并直接传递 GPUI 对象 | 在宿主中运行原生代码 | 首版暂缓；Rust ABI 不提供稳定性保证，且会耦合宿主和 UI 类型版本 |
| WebView + HTML / JS 界面 | 图表、复杂网页组件等自由布局 | 有明确场景后单独验证；当前界面与 Markdown 均走原生路径，需额外设计嵌入、焦点、主题和消息桥接 |

VS Code 的 Tree View 文档提供了通过清单注册视图容器的参考，但本方案不承诺兼容 VS Code 扩展。Rust Reference 明确说明 Rust ABI 没有稳定性保证；使用 C ABI 可以另行设计边界，但不会自动解决 GPUI 对象的版本与生命周期问题。Wasmtime 文档说明其 WASI 文件系统接口采用能力模型；实际隔离仍取决于宿主开放的导入与资源配置。

**独立进程提供故障边界；首版的信任范围必须明确。** 当前 [child.rs](../src/child.rs) 已实现 stdio、隐藏窗口启动及 Windows Job 进程树清理；`spawn_with_env` 从 `std::env::vars()` 构造子进程环境，随后应用覆盖值。插件启动应增加明确的环境构造策略，只传递必要变量，避免直接继承服务凭据。不能改变现有 CLI 的环境语义来实现这一点。

清单权限约束的是经宿主 API 发起的调用。普通本地可执行程序仍可能直接使用操作系统能力，因此 `workspace.metadata.read` 不能被描述为操作系统级只读沙箱。首版需要用户显式安装并信任本地插件，不自动执行仓库中发现的插件。面向不受信任代码的发布需要另行验证 WASM / WASI 或操作系统沙箱；现有 Job 清理能力不足以证明权限隔离。

工程中还需要把以下边界落实到协议和数据模型中。

| 边界 | 必须实现的行为 | 当前可参考的位置 |
| --- | --- | --- |
| 稳定身份 | 使用带命名空间的插件 / 视图 / 命令 ID；工作区采用稳定标识，最初可由规范化根路径映射，不能使用可重排的数组下标 | [sidebar.rs](../src/app/sidebar.rs) 的工作区重排及索引映射 |
| 上下文一致性 | 区分所选工作区、会话所属工作区及当前文档；请求绑定上下文版本，切换后丢弃过期结果；命令调用重新验证目标 | [app.rs](../src/app.rs)、[files.rs](../src/app/files.rs)、[sessions.rs](../src/app/sessions.rs) |
| 独立视图状态 | 按插件、视图、工作区及可选会话保存状态；工具详情选择与通用容器选中态分离；禁用当前视图时回退到可用内置视图 | [dock.rs](../src/app/dock.rs)、[state.rs](../src/app/state.rs) |
| UI 响应 | 渲染路径只读取已取得的数据；扫描、运行和网络调用在后台；每个插件独立队列与限额，轮询共享总时间预算 | [stream_events.rs](../src/app/stream_events.rs)、[event_queue.rs](../src/event_queue.rs) |
| 事件语义 | 状态快照可合并；命令结果、停止及失败消息必须有处理空间；卸载后拒绝迟到消息 | 现有事件队列有数量 / 字节限制及控制事件保留空间，但类型绑定 CLI `Event`，应复用思路或提取通用层 |
| 渲染限额 | 限制消息字节、列表条目、Markdown 长度、更新速率和图标来源；限制链接动作；原生列表采用虚拟化 | 现有列表、文件和工具详情的有界呈现方式 |
| 生命周期 | 懒加载、激活超时、取消、崩溃错误占位、有限重试；禁用时注销入口、订阅和任务，随后终止插件进程树 | [child.rs](../src/child.rs) 与后台任务模式 |
| 数据保存 | 布局偏好使用默认字段兼容旧配置；插件数据独立于会话历史，按插件划分、限制容量并带 schema 版本 | [store.rs](../src/store.rs)、[persistence.rs](../src/persistence.rs) |
| 业务约束 | 有副作用的宿主命令继续走现有会话 / Git 调度与检查；首版宿主 API 以读取摘要、打开视图和导航为主 | [session_controller.rs](../src/app/session_controller.rs)、[git_ui.rs](../src/app/git_ui.rs) |

声明式视图仍需验证资源上限，才能控制恶意或错误数据造成的渲染负担。独立进程崩溃后的错误显示、队列公平性和视图缓存均需要专门验证，不能仅从采用独立进程推断主界面一定保持流畅。

建议按以下三个阶段交付。按一名熟悉 Rust / GPUI 的开发者估算，含相关回归，不含插件市场、远程分发、签名服务、完整 WebView 或不受信任代码沙箱；这是预算范围，原型后需要校准。

| 阶段 | 可验收结果 | 估算 |
| --- | --- | --- |
| 1. 三处界面扩展点 | 注册表、命令分发、稳定视图 ID；工具详情接入右侧通用容器；一个内置示例同时出现在左侧、右侧和底部 | 6–10 人日 |
| 2. 本地动态插件 | 清单与兼容校验、按需进程启动、stdio 协议、宿主 API、预设原生视图、限额与错误恢复；可从独立目录安装示例插件 | 10–16 人日 |
| 3. 管理与稳定性 | 启用 / 禁用、授权与版本提示、日志、布局和插件状态保存、开发文档以及集成回归 | 5–8 人日 |

完整的受信任本地插件版本约 21–34 人日，即一人约 4–7 个工作周。只做内置功能扩展约 1–2 个工作周。首版视图应保持为有限模板；如果要求任意复杂交互、即时热重载、公开市场或任意第三方代码隔离，上述估算需要重新制定。

建议新增 `src/plugins/` 承载清单、注册表、命令、协议、运行时和存储，新增 `src/app/plugin_views.rs` / `plugin_ui.rs` 承载原生模板和管理界面。在 `files.rs`、`dock.rs`、`toolbar.rs` 与 `shell.rs` 接入贡献列表；在 `app.rs` 和运行轮询处接入宿主状态及结果分发。`store.rs` 只保存宿主侧布局和启用偏好，插件负载使用独立存储。最终文件划分可在第一阶段原型中调整。

验收应覆盖一个完整插件的使用过程，以及会影响现有用户的边界。

- 同一个示例插件注册左侧图标、右侧视图和底部状态项，并通过命令关联三者；外部插件增删无需重新编译主程序。
- 清单存在重复 ID、不兼容 API、缺失入口或无效视图引用时，给出明确错误且不产生半注册状态；宿主保留的 ID 不可覆盖。
- 切换工作区和会话、重排工作区、切换文件标签后，视图数据和动作目标保持一致；过期异步结果无法写入新上下文。
- 保留导航宽度、右侧浮层和拖动限宽、工具详情独立滚动及底栏对齐；状态项拥挤时仍能看到宿主的重要信息。
- 停止、禁用或卸载插件后，入口、订阅和子进程得到清理；崩溃、握手超时、输出过量和协议错误可以恢复，不影响其他插件和聊天任务。
- 宿主 API 的权限检查、敏感环境变量过滤、插件数据边界和只读行为有验证；文档明确本地进程插件的信任模型。
- 使用相同数据集比较无插件与多个插件时的冷启动、空闲内存、消息流、滚动及拖动表现。目标为未激活插件不启动进程、隐藏视图不持续重绘，性能容差在原型基线完成后确定。

现有回归入口包括 [ui_tests.rs](../src/app/ui_tests.rs) 中的 `tool_dock_keeps_header_visible_and_scrolls_independently_in_both_layouts`、`navigation_modes_share_default_and_retained_width_without_duplicate_git`、`sidebar_bottom_bar_matches_the_toolbar_and_opens_about`，以及 [files.rs](../src/app/files.rs) 中的文件导航、Git 导航和标签测试。评估阶段只核对了源码及这些测试入口，没有运行应用、执行测试或进行性能基准。

本次决策可先确定：三处扩展共用注册表；宿主保留原生绘制；首先验证内置适配器；动态版本采用受信任本地进程；首版 API 提供有限视图模板与受控命令。后续以“工作区概览”示例是否能独立发布、安装和停用作为接口设计的实际检验。

外部技术依据（于 2026-11-04 读取官方文档；仅参考接口与隔离原则，不代表已有兼容实现）：

- Rust Reference，External blocks：`https://doc.rust-lang.org/reference/items/external-blocks.html`
- Visual Studio Code，Tree View API：`https://code.visualstudio.com/api/extension-guides/tree-view`
- Wasmtime，Security：`https://docs.wasmtime.dev/security.html`
