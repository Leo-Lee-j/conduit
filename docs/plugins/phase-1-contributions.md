# P1：界面贡献与内置适配

设计版本：0.3；日期：2026-11-05；状态：P1-0 已记录原型结果，P1-1 数据注册层已实现，三处正式 UI 待迁移。前置阅读：[共同约定](README.md)与[Shell 复用评估](gpui-shell-reuse-assessment.md)；下一阶段优先验证 [P2-A Shell](phase-2-shell-runtime.md)，[P2-B 独立进程](phase-2-local-runtime.md)为备选。

当前代码与验证边界见 [P1-1 实现记录](p1-registry-validation.md)。本文仍描述完整 P1 目标；已实现的数据契约与后续 UI / 生命周期工作分别记录，不能将注册层测试等同于整个阶段验收。

## 目标与交付边界

本阶段把左侧功能入口、右侧视图和底部状态项接入统一注册表，交付一个同时使用三处区域的内置“工作区概览”示例。内置工具详情迁入通用右侧容器，现有工作区、文件、Git、会话和设置行为应保持可用。

P1 的扩展仍随应用编译发布。外部清单解析、正式脚本 / 进程运行时、安装 UI 和持久化授权在后续阶段实现；本阶段先将接口和状态结构设计成可承接这些能力的形式。

## Shell 复用对本阶段的约束

注册表与位置 / 作用域模型保持与执行后端无关。Shell 的 ComponentRegistry 是原生组件绑定表，Base 的 PanelRegistry 是停靠恢复表；本阶段仍需管理 Conduit 三处贡献的 owner、排序、注销和命令身份。

renderer key 应能由 app 层解析为内置视图，或未来所选后端的视图适配器，不提前把它定义为 P2-B 的三个模板枚举。Shell 视图也必须使用同一 ViewInstanceKey；左侧和右侧可同时挂载，不能用同一个实体在两处重复渲染来代替实例设计。底栏继续接收有界状态数据，由宿主保持高度和溢出规则。

增加 P1-0 复用验证，对 G1–G6 形成记录，包括锁定版本 Windows 构建、每插件 Policy / HostModule、多视图和底栏命令、根节点 / 浮层、资源上限及退出清理。原贡献改造可以先推进；P2-A 选型与公开 SDK 冻结必须等待验证。上游接口需要补充时先列明确切缺口，不依赖 crate-private 方法。

## 数据模型与依赖方向

`src/plugins/` 中的类型只依赖普通 Rust 数据和序列化类型。建议新增 `ids.rs`、`contributions.rs`、`registry.rs`、`commands.rs`、`context.rs`；由 `src/plugins.rs` 声明模块，并已从现有 `src/lib.rs` 导出为 `conduit::plugins`，支持独立库级测试；ConduitApp 适配将在 P1-2 开始接入。

| 类型 | 必要字段 | 约束 |
| --- | --- | --- |
| `ContributionSet` | owner、commands、views、activity_items、status_items | 一个 owner 的贡献整体校验并原子注册 |
| `ViewDescriptor` | id、title、location、scope、renderer、when | location 为 navigation / right；renderer 引用宿主提供者，后续适配所选 P2 的脚本视图或原生模板 |
| `ActivityItem` | id、title、icon、action、order、when | action 为 OpenView 或 ExecuteCommand，两者互斥 |
| `StatusItem` | id、scope、alignment、priority、text、icon、tooltip、command、when | alignment 为 left / right；动态值与声明分开 |
| `CommandDescriptor` | id、title、argument_policy、required_capabilities | 命令属于 owner；内置业务约束由实际处理器再次验证 |
| `CommandInvocation` | command_id、args、origin、context_version | 点击时捕获上下文，不在异步完成时重新猜测目标 |
| `ViewInstanceKey` | owner、view_id、scope_key | 区分不同工作区或会话的视图状态 |
| `ScopeKey` | Global / Workspace / Session | Session 必须同时绑定工作区及会话身份 |

以下为完整阶段的结构示意；当前纯数据版本见 `src/plugins/state.rs`，GUI 尺寸、拖动、工具内容与实例缓存仍由后续 app 适配补齐。

```rust
struct NavigationState {
    selected: ViewId,
    collapsed: bool,
}

struct DockState {
    active: ViewId,
    open: bool,
    width: f32,
    drag: Option<ResizeOrigin>,
}

struct ToolSelection {
    session_id: u64,
    message_key: String,
}

struct ViewInstanceKey {
    owner: PluginId,
    view: ViewId,
    scope: ScopeKey,
}
```

`ConduitApp` 持有 `PluginUi`，负责注册表访问、导航 / 右侧状态及视图实例缓存。原生视图实体由 `app` 层的 renderer map 持有；数据注册表只保存 renderer key。这样可以保留内置视图对应用内部状态的访问，而不让 `plugins` 模块反向依赖整个应用。

`ToolDetailState` 单独保存按会话划分的工具选择和滚动状态。通用容器不理解工具输入、输出或原生会话 ID。文件浏览器继续保存目录、文件文档、Diff 和 Git 缓存，移出的只是导航模式与折叠状态。

## 注册、身份与显示条件

- 内置功能使用 `conduit.core.workspaces`、`conduit.core.files`、`conduit.core.git`、`conduit.core.tool-details` 等视图 ID；外部插件不得占用 `conduit.*`。内置注册调用带宿主专属 owner 类型，不能仅凭清单里的字符串获得身份。
- 所有贡献 ID 在同一注册表内唯一。注册前验证 ID、renderer、图标、位置、作用域，以及所引用的视图和命令；失败时不插入任何部分。
- 活动入口只能直接绑定同 owner 的 navigation 视图。打开右侧视图通过命令完成；状态项也只绑定同 owner 命令。保留宿主内部适配器执行核心命令的能力，外部跨插件命令不在 v1 范围。
- 注册时返回按 owner 管理的注销句柄。注销先阻止分发新命令，再移除贡献和实例；当前选中视图失效时执行回退，不留悬空 ID。
- `when` 首版只支持固定布尔键，可省略、单键、`!key` 或以 `&&` 连接的条件。键限定为 `workspace.open`、`session.selected`、`session.running`、`document.open`、`host.writable`；未知键或其他语法在注册时拒绝。
- 条件由宿主计算。`when` 只影响可见性；权限、只读状态和命令执行资格另行检查。首版插件不能注入自定义上下文键。

默认活动项按 `(order 升序, id 升序)` 排列；状态项按 `(priority 降序, id 升序)` 选择保留次序。用户排序偏好在 P3 接入，始终高于插件默认顺序。显式相同优先级时必须保持确定顺序，避免刷新后跳动。

## 三个区域的布局与交互

| 区域 | 本阶段规则 | 需要保留的现有行为 |
| --- | --- | --- |
| 左侧入口 | 顶部为核心导航与功能贡献，底部保留设置、关于和折叠；空间不足时将后序可见功能放入更多菜单，选中功能仍可在菜单中定位 | 图标栏折叠后仍可使用；工作区任务徽标及文件 / Git 刷新逻辑继续生效 |
| 左侧页面 | 通过 ViewId 找 renderer；点击某页时展开侧栏；不因再次点击当前页而隐式重建其状态 | 共用默认宽度和拖动后宽度；键盘切换 Git 通过同一个导航动作 |
| 右侧容器 | 标题区显示当前视图标题、视图切换菜单和关闭按钮；一次显示一个视图；宿主统一提供加载、空白及失败占位 | 窄屏浮层、宽屏限宽、拖动跟手、标题固定、内容独立滚动 |
| 工具详情 | 点击工具详情图标时显式激活核心工具视图并记录对应会话和消息；背景工具事件仅更新内容 | 工具输入 / 输出、复制、状态和滚动位置继续可用 |
| 底部 | 左右贡献区域与核心状态分开；插件文本限制宽度，溢出项进入更多菜单；工具栏高度保持现有 34px | 工作区、分支、执行状态、推送与性能入口仍可操作；与侧栏底部对齐 |

底栏布局先分配宿主关键运行 / 错误提示和必要操作，再为贡献项分配剩余宽度。低优先级项先进入更多菜单；没有完整项能容纳时，只留下更多入口。文字截断保留完整 tooltip，菜单中的命令可用性与原入口一致。

图标应来自宿主的版本化名称表。P1 沿用现有图标资产；P2 的外部清单只允许该名称表，不接收远程图片、任意 SVG 或任意文件路径。

右侧关闭仅改变容器可见性，保留活动 ViewId 和实例状态。当前插件被移除或当前作用域消失时，选择核心工具详情作为有效回退目标并关闭容器，避免自动打开其他内容。左侧发生同类失效时回退到核心工作区页，维持原折叠状态。普通数据更新不得抢焦点。

## 上下文与异步状态

`PluginContext` 是宿主生成的不可变快照，至少包括工作区身份、可选会话、当前文档标识、宿主可写状态和路由版本。内部数据可以较丰富；P2 对每个外部插件另做权限过滤。

| 触发 | 处理 |
| --- | --- |
| 切换工作区 / 会话 / 文档 | 先更新宿主选择，再生成新 contextVersion，切换对应 ViewInstanceKey，取消旧实例的在途请求 |
| 重排工作区 | 使用稳定 WorkspaceKey 找到原上下文；不要把旧数组下标保存在插件实例中 |
| 仅主题或摘要变化 | 更新相应数据修订号，只使依赖该信息的视图失效 |
| 导航页或右侧被隐藏 | 保留有限状态缓存，停止持续刷新隐藏视图；重新打开时按需刷新 |
| 工作区或会话移除 | 使相关 scope_key 失效，清理实例与工具选择，不展示其他作用域的旧结果 |
| 只读 / 关闭状态变化 | 重算动作可用性；禁止副作用命令，后续由 P2 关闭外部运行时 |

异步结果必须与提交时的 scope_key、contextVersion 和实例身份匹配。内部原生适配器可以沿用现有后台任务，但需要在结果进入插件视图前执行同样的核对。插件界面不订阅每个聊天 token；需要摘要时由宿主按变化合并通知。内置命令自行校验参数类型；P2 的外部命令使用有界纯数据对象（进程路线为 JSON，Shell 路线为 HostValue），插件负责业务字段校验，首版不引入完整 JSON Schema 执行引擎。

视图实例使用独立的 GPUI Entity 或保留状态容器；数据相同不触发全应用 `notify`。P1 对缓存设置有限容量并通过 LRU 淘汰隐藏实例，初始上限为每窗口 32 个，其中可见实例不淘汰；P3 只持久化允许恢复的轻量状态。

## 内置适配与改造清单

| 位置 | 改造内容 |
| --- | --- |
| [files.rs](../../src/app/files.rs) | `Navigation` 分支迁为 ViewId 查询；`show_git_navigation` / `toggle_git_navigation` 调用统一导航动作；保留文件业务状态 |
| [sidebar.rs](../../src/app/sidebar.rs) | 保留 `SidebarView` 缓存边界，接入注册表变更和必要上下文更新 |
| [dock.rs](../../src/app/dock.rs) | 提取工具内容 renderer 和通用容器，不在容器中查找工具消息 |
| [tools.rs](../../src/app/tools.rs)、[sessions.rs](../../src/app/sessions.rs) | 工具详情入口与标题栏切换按钮调用统一的 OpenView / CloseDock 动作 |
| [toolbar.rs](../../src/app/toolbar.rs)、[shell.rs](../../src/app/shell.rs) | 插入贡献项、更多入口和容器；沿用宽度与浮层逻辑 |
| [app.rs](../../src/app.rs)、[state.rs](../../src/app/state.rs) | 引入 PluginUi / DockState / ToolDetailState，移除被替代的导航字段 |
| 拟新增 `src/app/plugin_ui.rs`、`plugin_views.rs` | 原生 renderer map、动作分发和示例视图 |
| 拟新增 `src/plugins/` 基础模块 | 贡献声明、验证、注册 / 注销、上下文和命令数据 |

内置业务不必全部重写成插件：P1 至少将三个核心导航页、工具详情、一个简单状态项和示例接入注册路径。Git 分支选择器、复杂弹出控件仍可作为宿主保留内容，避免为复杂内置控件提前扩大外部 API。

示例在测试及开发开关下注册，默认发布界面不额外出现示例按钮。增加示例的三个贡献只修改其定义和 renderer 注册，不再给 `files.rs`、`dock.rs`、`toolbar.rs` 各添加一个业务专用分支。

## 实施顺序与验收

| 工作项 | 产物 | 验证 |
| --- | --- | --- |
| P1-0 Shell 复用验证 | G1–G6 结果、公开 API 路径及后端选择记录 | 多实例、按插件权限、主线程预算与 Root 兼容；不以 standalone 示例代替集成 |
| P1-1 身份与注册 | 已实现数据模型、原子注册、注销、条件与路由校验；见[实现记录](p1-registry-validation.md) | 重复 ID、非法引用、未知条件、跨 owner / 代次、回退规则的单元测试 |
| P1-2 左侧接入 | 核心导航适配、更多菜单、统一导航动作 | 文件 / Git 导航、工作区重排、折叠及宽度回归 |
| P1-3 右侧接入 | 通用容器、工具详情适配、实例状态 | 宽 / 窄屏、独立滚动、显式打开、会话隔离 |
| P1-4 底栏与示例 | 状态项布局、命令分发、完整示例 | 三处联动、窄窗口溢出、重复点击和只读行为 |
| P1-5 进入 P2 的接口检查 | 宿主数据与 GPUI 对象边界、异步版本检查 | 示例注销后无残留，旧上下文结果不更新新视图 |

需要保留的既有测试包括 [ui_tests.rs](../../src/app/ui_tests.rs) 中的 `tool_dock_keeps_header_visible_and_scrolls_independently_in_both_layouts`、`navigation_modes_share_default_and_retained_width_without_duplicate_git`、`sidebar_bottom_bar_matches_the_toolbar_and_opens_about`、`workspace_drag_reorders_and_preserves_navigation_identity`，以及 [files.rs](../../src/app/files.rs) 的文件与 Git 导航回归。

新测试重点验证可观察行为：注入两个不同 owner、快速切换两个工作区、移除当前 owner、使旧异步结果迟到，以及在小窗口中触发溢出。用示例验证注册路径，避免只断言字段能被写入。

P1 完成条件：三个区域均通过注册路径呈现示例；核心导航和工具详情回归通过；实例状态不跨工作区 / 会话串用；注销清理与布局回退通过；提交一份实际测试记录。原贡献改造预估 6–10 人日；另为 P1-0 规划 2–3 人日验证时间盒，超出时记录缺口并重新评估，不自动扩大为上游重构。贡献部分通过即可服务内部功能；进入 P2 前还需完成后端决策记录。

主要实施风险是 `ConduitApp` 的共享状态与既有导航副作用耦合。通过保留内置适配器、逐区迁移和现有回归控制范围；不以迁移全应用到新架构作为阶段完成前提。
