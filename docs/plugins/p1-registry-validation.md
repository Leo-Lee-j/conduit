# P1-1：贡献身份与注册层实现记录

日期：2026-11-05；范围：[P1 设计](phase-1-contributions.md)的 P1-1，以及供下一步 UI 适配使用的选择状态模型。P1-0 的 Shell 结论仍见[原型验证记录](shell-prototype-validation.md)。

**纯数据注册层已实现并导出到 `conduit::plugins`。** 本轮完成原子注册、身份与引用校验、条件求值、命令分发前检查、稳定作用域、注销句柄与选择回退。正式 ConduitApp 还没有调用这个注册表，界面仍沿用现有实现；不能把本轮测试当作 P1-2～P1-4 的三处 UI 回归通过。

## 实现与边界

| 文件 | 当前职责 |
| --- | --- |
| [plugins.rs](../../src/plugins.rs)、[lib.rs](../../src/lib.rs) | 从现有库入口导出插件数据模型，无 GPUI 依赖或初始化副作用 |
| [ids.rs](../../src/plugins/ids.rs) | Plugin / View / Command / Item ID，外部身份拒绝 `conduit.*`，宿主身份只能由 crate 内 Rust 代码构造 |
| [contributions.rs](../../src/plugins/contributions.rs) | 视图、活动入口、状态项、整组贡献；显示数据与声明分开建模 |
| [context.rs](../../src/plugins/context.rs) | 宿主路径身份、工作区 / 会话 scope、不可变上下文、版本生成与固定条件语法 |
| [registry.rs](../../src/plugins/registry.rs) | 宿主 renderer / icon allowlist、整组校验与发布、按 owner 撤销、声明查询与排序、点击请求生成 |
| [commands.rs](../../src/plugins/commands.rs) | 命令参数与 capability，携带注册身份 / 上下文版本 / scope 的命令请求，执行前重新校验 |
| [state.rs](../../src/plugins/state.rs) | ViewInstanceKey、左右选中视图、关闭保留选择、注销 / scope 消失后的回退；尚无 GPUI 实体缓存 |
| [builtins.rs](../../src/plugins/builtins.rs) | 核心工作区 / 文件 / Git / 工具详情及简单状态项声明；显式 opt-in 的工作区概览三处声明 |
| [tests.rs](../../src/plugins/tests.rs) | 注册失败回滚、两个 owner、撤销与重注册、过期请求、作用域切换、回退及参数边界 |

库入口只导出数据层，没有为正式应用增加 feature、运行时依赖或启动时加载行为。`builtins::register(false)` 只注册核心声明；`true` 额外登记示例。P1-4 接入时须从开发 / 测试开关决定是否传 `true`，默认发布界面不能因此出现示例按钮。renderer key 是宿主映射表的键，注册不会执行 renderer 或业务处理器。

### 注册协议

- 每个 owner 一次提交 ContributionSet，先检查全部 ID、标题、renderer、icon、引用和参数策略，再发布。失败不占用任何 ID，也不推进注册表 revision。
- ID 在整个注册表跨类型唯一，并且必须位于 owner 的 `owner.*` 命名空间。同名 owner 不能重复登记；升级先显式撤销旧注册。
- renderer allowlist 同时指定 owner、location 和 scope。活动项只能引用同 owner 的 navigation view，或同 owner 命令；状态项命令还须使用相同 scope。
- 目前活动 / 底栏命令不携带参数，因此不能绑定必须接收对象参数的命令。视图或宿主命令可提交受限 JSON 对象；具体业务字段仍由处理器验证。
- Registration 是显式撤销句柄，含注册表身份与单调代次。其他窗口注册表的句柄、已撤销句柄、同 owner 重注册前的旧句柄均不能用于分发或注销。
- `unregister` 先让命令与视图请求失效，返回被移除的声明 ID 清单；app 适配器随后释放对应实体 / 任务并 reconcile 选择。Drop 不自动注销，避免销毁时无法执行 GPUI 清理的隐式副作用。

这里的宿主身份和句柄不是供脚本自选的参数。未来 P2 清单解析必须走外部 owner 构造，并由宿主绑定 Registration 与 grants；不能把 renderer catalog、宿主身份构造或 `Registry::prepare_view` 等宿主入口直接开放给任意脚本。`prepare_view` 是跨内置视图的宿主导航入口；外部 API 需要先核对 caller owner。

### 条件、权限与上下文

`When` 只接受省略条件、固定键、`!key` 和 `&&`，其他语法或未知键拒绝。活动顺序按 `(order 升序, id 升序)`，状态项按 `(priority 降序, id 升序)`；UI 溢出布局尚未接入。

显示条件仅筛选声明。执行前单独检查注册代次、owner、contextVersion、scope、关闭状态、授权与只读状态。外部命令在只读状态全部拒绝；内置只读查询可由明确的非写命令执行。声明 `HostMutation` capability 的命令必须要求 writable。

WorkspaceKey 使用 Store 已确定的绝对 PathBuf，构造时不访问文件系统、不转小写、不使用数组下标。传入的 session 不属于当前 workspace 时，快照把 session 及其 running 状态清空。工作区 / 会话 / 文档 / writable / closing 变化递增版本；只变 running 摘要不改变路由版本，执行时仍使用最新资格判断。P1-2 接入时宿主负责在这些真实选择变化后调用 ContextTracker，不能继续用旧快照分发。

单个注册表及 ContextTracker 由同一窗口持有。并行窗口各自维护注册表和上下文；不把这里的单窗口版本数当作全局事件版本。

### 选择与清理

左侧注销后回退核心工作区页并保留折叠状态；右侧回退工具详情并关闭。关闭右栏仅改变可见性，保留选择和 ViewInstanceKey。工作区切换产生新的 scope key；scope 不存在时清除旧 key。调用者须保证核心回退 owner 在窗口生命周期内保持注册，否则 reconcile 返回错误。

当前实现只维护身份与选择，不执行 renderer、数据写入或异步任务取消。旧 ViewRequest 可通过注册身份 / 上下文版本失效，但未来实体缓存还需额外的实例代次与实际任务清理，不能将此检查称为完整 P1-5 生命周期完成。

### 边界与限额

单个 owner 最多 512 个声明、单窗口最多 128 个 owner；ID 最多 160 字节，标题最多 160 字节，状态文本最多 256 字节、tooltip 最多 1024 字节，不允许控制字符。对象命令参数的声明预算最多 64 KiB、深度最多 16；遍历先约束结构，再检查准确 JSON 编码字节数。

这些限制针对已构造的数据，不是外部输入解析器或进程资源限额。P2 仍须在接收字节和解码前设置边界，也仍受 Shell 原型中 G1 / G2 / G5 缺口约束。

## 验证记录

在仓库根目录执行：

```powershell
cargo test --locked --lib plugins::
cargo test --locked --lib
cargo clippy --locked --lib -- -D warnings
rustfmt --edition 2024 --check src/plugins.rs
rustfmt --edition 2024 --check --config skip_children=true src/lib.rs
```

实测结果：

| 检查 | 结果 |
| --- | --- |
| 插件注册层测试 | 18 passed，0 failed |
| 完整库级测试 | 27 passed，0 failed，其中包含 9 项原有更新协议测试 |
| `cargo clippy --locked --lib -- -D warnings` | 通过 |
| 新模块与库入口 rustfmt | 通过 |
| 本轮修改的 whitespace 与文档链接 | 通过 |

库级测试原始输出保存在本地 `target/plugin-registry-validation/library-tests.log`，由现有 target 规则忽略。

测试不会读写真实工作区或实例化 GUI；使用宿主路径身份的合成输入验证模型。库级测试还覆盖原有更新协议，未改动其实现。

## 下一步：P1-2 左侧适配

把 Registry 与 ContextTracker 放入 ConduitApp 的 PluginUi，先让工作区 / 文件 / Git 的活动入口从注册表产生，并统一导航动作。保留现有 SidebarView 缓存、文件 / Git 刷新副作用、任务徽标与拖动宽度；随后补更多菜单和真实工作区重排测试。P1-3 再迁移通用右栏与工具详情，P1-4 接入底栏及内置示例，最后执行 P1-5 实例缓存与清理验收。
