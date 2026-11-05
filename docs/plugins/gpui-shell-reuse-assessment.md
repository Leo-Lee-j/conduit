# GPUI Shell 复用评估

源码评估基线：2026-11-04；后续更新：2026-11-05，已实现独立验证原型，实测见[原型验证记录](shell-prototype-validation.md)。本文保留未打补丁的源码评估依据；后续 G2/G5 宿主控制与 G1 Fast 兼容补丁见 [本轮进度](p2-host-controls-2026-11-05.md)，构建与运行结论以后续记录为准。本文补充并修正[初始评估](../ui-plugin-extensibility-evaluation.md)，影响[阶段总览](README.md)中的 P2 选型。

G3 后续候选补丁已验证具体视图的定向命令、双 owner / 多视图停止、工作区和代次失效，
见 [G3 进度](p2-targeted-commands-2026-11-05.md)。清单实现绑定和开发加载适配器已完成，见[开发加载器记录](p2-development-loader-2026-11-05.md)；后续已接入真实 ConduitApp 验证入口，见[桌面接入记录](p2-desktop-integration-2026-11-05.md)。[G6 浮层清理](p2-overlay-cleanup-2026-11-05.md)已补按运行时撤销浮层、焦点恢复与更新前同步清理。下一步为真实 IME / 浮层焦点和完整应用性能验收，未来插件存储写入屏障仍待实现。
此结果不改变下文未打补丁源码的事实，也不代表正式应用 G1–G6 全部通过。

## 结论与决策

**优先验证 GPUI Shell 作为界面插件运行时。** 它与本需求直接相关：在 Rust 宿主中加载 JavaScript，生成原生 GPUI 视图，提供能力控制与宿主模块。前版只比较了原生进程、WASM 和 WebView，遗漏了当前依赖仓库已经具备的脚本运行时。

Conduit 仍需实现三处贡献的注册、排序、作用域、命令路由、懒激活和安装事务。Shell 可以替代一部分脚本执行、原生视图构建、异步任务与异常呈现工作；不能直接替代这些产品规则。当前版本存在公开 API、资源控制与根视图接入缺口，尚不足以宣布可直接采用。

决策顺序为：P1 保留与后端无关的贡献模型并完成复用原型 → 通过准入检查后执行 [P2-A Shell 方案](phase-2-shell-runtime.md) → 未通过时评审所需上游补丁，或选择 [P2-B 独立进程备选](phase-2-local-runtime.md)。首版只实现一个外部运行后端，不把两套运行时同时交付作为目标。P3 的安装、授权与数据恢复继续由宿主负责。

## 证据基线与版本差异

本次读取了用户提供的官网及其官方 Markdown 页面，并核对 Cargo 已下载的上游源码。网络检索工具未返回可用正文，官网正文通过 HTTPS 的 Markdown 路由取得。下面的实现判断以精确提交源码为准，不把网页示例当作编译验证。

| 项目 | 已核实结果 | 依据 |
| --- | --- | --- |
| Conduit 依赖 | `gpui-kit` Git rev `11345ec002d9f182a266b5d5dd3a217090700a87`；锁文件中的版本为 `0.8.0` | [Cargo.toml](../../Cargo.toml)、[Cargo.lock](../../Cargo.lock) |
| 提交日期 | 上述提交日期为 2026-10-24；核对时 Cargo checkout 无已跟踪文件修改 | 上游 Git 提交对象及工作区状态 |
| Shell 是否已有 | 同一提交存在独立 `gpui-shell` crate，版本 `0.8.0`，该提交设置 `publish = false` | U2 |
| 是否已被 Conduit 使用 | `gpui-kit` 默认 feature 为 `component` / `assets`，无 Shell re-export；Conduit 未声明 `gpui-shell`，锁文件也无该包 | U1、[Cargo.toml](../../Cargo.toml) |
| 官网状态 | 读取时官网标注 M0 / 实验接口，并写明 Rust crate 从 `0.8.1` 起发布；这不代表项目已升级或最新版本就是 `0.8.1` | W1 |
| 接入条件 | 可先评估同一 Git rev 的独立 Shell 依赖，无需先整体升级；新增依赖仍须验证 Windows 构建及 feature 一致性 | U1、U2；工程判断 |

上游默认 Shell features 为 `quickjs`、`components`、`webview`。其中 `webview` 会启用 `gpui-fast`；Conduit 当前把 GPUI Fast 作为可选后端。因此原型应显式关闭 Shell 默认 features，再选择 `quickjs` / `components`，并把 Conduit 的可选 `gpui-fast` 同步传给 Shell。Shell 还会启用 `gpui-base/inspector`，应计入 release 构建成本。[U1、U2、U3]

另一个构建前提是上游 workspace 的 `[patch.crates-io] rquickjs` 指向 `crates/shell/rquickjs-compat`。Conduit 不能假定上游 workspace 的补丁和 profile 会随 Git 依赖自动应用；原型须核对兼容门面、QuickJS/JIT/stdlib 的解析结果及根工程补丁要求。源码评估阶段没有修改 Cargo 文件；后续[独立原型](../../tools/shell-prototype/Cargo.toml)已验证该 patch，并在实验锁文件中固定兼容的 RSA / PKCS1 版本。[U10]

## 三处扩展如何分工

| 需求 | Shell / Base 已有能力 | Conduit 需要补齐 | 本轮建议 |
| --- | --- | --- | --- |
| 左侧图标与导航页 | 原生 `ScriptView`、导航组件和 View 状态 | activity ID、图标表、排序、隐藏、更多菜单、按工作区切换 | 宿主绘制图标，脚本提供关联页内容 |
| 右侧侧边栏 | `ScriptView`；Base Dock / `ScriptPanel` 可构建停靠面板 | 工具详情适配、视图切换、窄屏浮层、限宽与关闭规则 | 将脚本内容挂入现有容器，先不迁移整个 Dock |
| 底部工具栏 | 现有 Rust `StatusBar.left/right`；Shell 也有状态栏绑定 | contribution owner、优先级、文本限宽、溢出和动作权限 | 继续由宿主绘制有界状态项；脚本提交数据和命令意图 |
| 宿主数据访问 | `HostModule` / `HostValue`，同步与异步函数，类型声明 | 工作区 / 会话权限、上下文版本、用户动作与只读校验 | 复用模块机制，提供范围明确的 `conduit` API |
| 生命周期 | 清单发现、加载、卸载、每插件 Policy、任务取消、异常视图 | 显式激活、包摘要授权、实例隔离、停用及更新屏障 | 使用或扩展公开加载接口，不重新写 JS 引擎 |
| 安装与状态 | Shell 按应用身份提供存储；Dock 支持 dump / load | 安装 UI、可信快照、启停、版本与数据代次成对回退 | 延续 P3 的宿主管理方案 |

前两项的界面证据见 [files.rs](../../src/app/files.rs)、[dock.rs](../../src/app/dock.rs)；底栏证据见 [toolbar.rs](../../src/app/toolbar.rs)、[shell.rs](../../src/app/shell.rs)。Shell 证据见 U3–U8。底部 Dock 表示可展开停靠面板，与本需求的固定底部工具栏是不同布局概念，不能直接对应。

Base 的 `PanelRegistry` 负责按名称恢复停靠面板；Shell 的 `ComponentRegistry` 负责绑定脚本可调用的原生组件；它们都不能直接承担 Conduit 三处贡献的注册表。上游 Shell README 也明确指出其余贡献注册机制尚未补齐。[U3、U8]

## 必须先验证的接入缺口

| 编号 | 源码事实 | 对设计的影响与准入要求 |
| --- | --- | --- |
| G1 构建与公开 API | Kit 没有 `gpui_kit::shell` 导出。网页使用的 `load_app`、`instantiate`、`ScriptView::new` 在该提交为 crate-private；公开入口包括 `load_application` / `mount_application` 与 `load` / `try_load` | 原型只能调用外部 crate 可访问的 API；记录所需依赖、补丁、后端和实际可编译入口 |
| G2 授权与加载 | `Policy::with_host_module` 公开，但 `PluginManager::load` 仅接受布尔 authorize；内部 `load_plugin` 从清单新建 Policy，没有注入宿主模块、裁剪 grant 或自定数据代次的参数 | 需要明确 Policy 的公开装配 / 加载 / 卸载接口。不能把内部测试能调用的私有方法写成宿主可用方案，也不能临时切换全局默认 Policy 冒充异步隔离 |
| G3 多视图与命令 | manager 每个 plugin 暴露一个入口 `ScriptView`；`HostModule` 只接收纯数据，不能接收 JS 函数；`ShellAction` 使用 GPUI 焦点路径 | 验证一个插件同时挂载左、右两个独立实例，并从底栏路由到正确 owner / scope。不得承诺 `registerView(callback)` 或把焦点 action 等同插件命令总线 |
| G4 根节点与浮层 | `ShellRoot::find` 查找窗口根，或 `gpui_base::Root` 的直接子 View；Conduit 使用组件 Root 包裹 `ConduitApp` | 候选结构为组件 / Base Root → ShellRoot → ConduitApp；验证设置弹窗、脚本浮层、tooltip、主题、焦点与 Esc，不将 ShellRoot 只塞进右侧内容 |
| G5 资源与响应 | VM 与 App 共用 UI 线程；默认无 JS 堆上限；默认 render / event / detached 看门狗分别为 2 / 5 / 10 秒；未发现公开宿主配置这些限额的入口 | 秒级中断不足以满足聊天交互目标。需要可配置资源边界及可测故障恢复；未解决前不开放普通第三方脚本安装。每插件 VM 也不隔离 UI 线程或进程崩溃 |
| G6 静态发现与关闭 | Shell 清单不提供 Conduit 的静态贡献与激活声明；unload 会取消 Policy 任务，但 Conduit 还有注册表、数据写入与更新流程 | 发现时只读数据，首次点击才执行 JS；停用须撤销贡献和任务、完成写入屏障，不能将 drop 视为全部清理完成 |

G1–G6 的源码依据分别见 U1–U9，阅读源码本身不构成通过。2026-11-05 的未打补丁实测见[初版验证记录](shell-prototype-validation.md)。后续本地候选补丁提供 G2 的公开 Policy 注入和 G5 的资源限制入口，G1 Fast 原型全目标类型检查已通过；故障实测、补丁范围及生产验收边界见 [G2/G5/G1 进度](p2-host-controls-2026-11-05.md)。

特别需要纠正文档与源码的差异：官网 Capabilities / Engine 页面写有 256 MiB 堆上限，但锁定提交的 `sandbox.rs` 明确移除了堆上限，runtime 构造也未调用堆限额设置。长篇架构文档仍写 Dock 无脚本绑定，实际 `DockArea` / `dock_area` / `register_panel` 已在引擎中绑定。因此不能只读概览或架构文档判断实现状态，也不能引用官网基准作为 Conduit 的测量结果。[W2、W3、U7、U8、U9]

## 权限与管理的调整

`Policy::new()` 默认无系统能力；**从 manifest 构造请求时，未写 storage 字段默认请求存储**，与空 Policy 不是同一语义。宿主必须显式计算授权，首版 Shell 清单使用 `capabilities.storage = false`，持久化通过 P3 的 Conduit 存储 API；不得直接接受默认存储、网络或进程执行请求。[U4、U5]

Shell 内置 `localStorage` 的文件布局、容量及写入模型与 P3 的 workspace namespace / CAS / data-generation 不相同，不能两套并行保存同一数据。该版本 manager 的 `with_data_home` 只改变共同前缀，实际路径仍为 `gpui-shell/plugins/<id>`，不是可直接传入每插件数据代次的接口。[U4、U5、U11]

Shell 使用进程内能力检查、语言限制和中断机制，不提供操作系统进程隔离。授予子进程执行或扩大的 HostModule 会改变边界。本轮 Shell 候选只接受明确信任的本地脚本，默认不授予文件系统、网络、环境变量额外名单、子进程、退出和 Shell 持久化能力；不把这种限制表述为可运行任意不可信代码。[U4–U7]

包保持不可变。公开 `set_editor_files(false)` 可用于关闭加载时的类型文件写入；Shell 清单 dependencies 非空时加载器会物化 Git 依赖，因此本轮直接拒绝动态依赖，要求作者把所需 JS 放入已核验包内。开发类型生成放在包快照之外。[U9]

## 阶段与预算影响

- P1 的注册、布局及内置适配继续推进；增加 G1–G6 的集成验证，输出实测记录和所需上游接口清单。
- P2-A 优先复用 Shell；保留 `ViewId`、`ViewInstanceKey`、上下文版本、静态激活和命令校验，替换自行设计的传输及模板执行后端。
- P2-B 的 stdio / JSON-RPC / 三种模板保留为备选，不作为 Shell 必须兼容的协议。
- P3 的包摘要信任、安装注册表、升级回退和只读规则继续适用；日志、异常、SDK 与停止验收按实际后端调整。

原 21–34 人日仅为独立进程路线估算。Shell 可以减少模板及协议工作，但会增加兼容接口与主线程资源控制工作；现有证据不能支持“引入即可完成”或降低总预算。建议先给复用验证 2–3 人日时间盒，此数仅为规划；未能形成可编译、可清理、可测的最小原型就结束时间盒并记录缺口，不以等待上游无限延长。P1 原 6–10 人日不含该验证；Shell 版 P2 / P3 在 G1–G6 结果明确后重估。

2026-11-04 的工作范围是文档与源码核对；2026-11-05 已追加独立原型及实际验证。原型未接入正式 ConduitApp，也没有测得正式应用的性能指标。

## 官方资料与源码索引

官网为 GPUI Kit 官方资料。本评估对其内容作摘要和源码对照；网站示例与性能数字不构成本项目保证。固定版本源码均来自以下仓库及提交：

```text
https://github.com/longbridge/gpui-kit/tree/11345ec002d9f182a266b5d5dd3a217090700a87
```

| 索引 | 页面或该提交中的路径 | 核对重点 |
| --- | --- | --- |
| W1 | `https://gpui-kit.com/zh-CN/shell/`；正文 `https://gpui-kit.com/zh-CN/shell.md` | 定位、实验状态与发布说明 |
| W2 | `https://gpui-kit.com/zh-CN/shell/capabilities.md` | 系统能力、资源限制描述 |
| W3 | `https://gpui-kit.com/zh-CN/shell/engine.md` | 主线程模型与上游基准 |
| W4 | `https://gpui-kit.com/zh-CN/shell/hosting.md` | 嵌入、浮层、刷新、生命周期 |
| W5 | `https://gpui-kit.com/zh-CN/shell/host-module.md` | HostModule 与纯数据边界 |
| W6 | `https://gpui-kit.com/zh-CN/shell/dock.md` | 停靠、恢复、面板与外观 |
| U1 | `crates/kit/Cargo.toml`、`crates/kit/src/lib.rs` | 默认 features、公开导出 |
| U2 | `crates/shell/Cargo.toml` | 版本、默认 features、引擎依赖 |
| U3 | `crates/shell/README.md`、`crates/shell/src/lib.rs` | 实验状态、贡献缺口、公开边界 |
| U4 | `crates/shell/src/plugin.rs`：`PluginManifest`、`load_plugin`、`PluginManager` | 发现、授权、加载、卸载、默认存储请求 |
| U5 | `crates/shell/src/policy.rs`：`Policy::new`、`with_host_module` | 逐插件授权及模块 |
| U6 | `crates/shell/src/root.rs`：`ShellRoot::find`；`host_modules.rs`、`action.rs` | 根节点、纯数据、动作路径 |
| U7 | `crates/shell/src/engine/quickjs/sandbox.rs`：Resource limits、`Budgets` | 无默认堆上限、秒级看门狗 |
| U8 | `crates/shell/src/dock.rs`、`engine/quickjs/mod.rs`：Dock 绑定；`crates/base/src/dock` | 面板实现及脚本绑定 |
| U9 | `crates/shell/src/engine/quickjs/mod.rs`：`load_application`、`mount_application`、`load_app`、`set_editor_files` | 公开加载、默认 Policy、类型写入、依赖物化 |
| U10 | 根 `Cargo.toml`：`patch.crates-io`、dev profile；`crates/shell/rquickjs-compat/Cargo.toml` | QuickJS 兼容门面与构建条件 |
| U11 | `crates/shell/src/storage.rs`、`view.rs` | 存储确认、`ScriptView::refresh` |
