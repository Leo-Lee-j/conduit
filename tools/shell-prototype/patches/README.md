# Shell 宿主控制补丁

基线：`longbridge/gpui-kit` 的 `11345ec002d9f182a266b5d5dd3a217090700a87`。
本目录是原型使用的候选补丁系列，尚未宣称上游合入。

| 补丁 | 用途 |
| --- | --- |
| `shell-host-controls.patch` | G2 显式 Policy 加载；G5 受限解释器、终止与资源观测 |
| `base-fast-compat.patch` | Base 三处 `Modifiers::secondary(&App)` 适配 |
| `component-fast-compat.patch` | 快捷键显示与解析、列表及表格修饰键的 Fast API 适配 |
| `kit-fast-compat.patch` | 测试工具动态快捷键改用带宿主 OS 的解析入口 |
| `shell-fast-compat.patch` | Shell Kbd、动态按键绑定使用 Fast 的动态解析与加载 API |
| `shell-targeted-commands.patch` | G3 视图弱句柄、定向 on_command、有界结果及统一停止取消 |
| `shell-owned-overlays.patch` | G6 运行时浮层身份、停止信号、按 owner 清理、焦点恢复与默认 sheet 参数修复；Base 定向撤销通知及 tooltip 状态观测 |

`prepare_upstream.py` 从精确 Git 提交导出源码，在忽略的 `.upstream/` 中展开并应用补丁。
生成的 Shell / Base / Component / Kit manifest 展开 workspace 继承，兄弟依赖仍声明原
Git rev，再由原型的 Cargo patch 统一替换四个 crate，防止同名 GPUI 类型因来源不同而重复。
Cargo 缓存里的源码不修改；正式应用没有引入这些 patch。锁文件仅移除四个被替换 crate 的
Git source 字段，所有包版本保持原锁定结果。

```powershell
python tools/shell-prototype/prepare_upstream.py
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase watchdog
python tools/shell-prototype/verify.py --phase resources
python tools/shell-prototype/verify.py --phase commands
python tools/shell-prototype/verify.py --phase fast-check
```

首次准备优先使用 `CARGO_HOME` 中的精确 checkout，否则在 `artifacts/` 内 fetch 精确提交。
可用 `--source <checkout>` 显式选择包含该提交的仓库，或 `--destination <fresh-directory>`
验证另一份从头生成的源码。已有输出若不匹配 patch SHA / 源码 SHA 会拒绝使用，不覆盖本地编辑。
Python 需要支持 `tomllib` 与 tar 的 `data` 提取过滤器。

所有补丁应用使用 LF，不依赖机器的 `core.autocrlf` 设置。runner 在结果 JSON 中记录精确
提交、补丁系列 SHA-256、生成源码 SHA-256 和 lockfile SHA-256；本轮已在全新目录重建并
比对源码哈希一致。Fast 兼容分支通过 feature 条件编译保留默认 GPUI API；Fast 类型检查
仍显示上游 ScrollWheelEvent / on_scroll_wheel 的弃用警告，不将其描述为零警告构建。

补丁公开的宿主入口：

- `PluginManager::load_with_policy`：发现、检查重复 ID 后，由宿主返回批准的 Policy；错误拒绝加载。
  manager 固定 application / dataDir 身份，宿主显式提供能力、存储和模块。
- `ShellRuntime::load_view_with_policy`：从求值到挂载都携带明确 Policy，沿用原有作用域和任务代次。
- `ShellRuntime::new_isolated_with_components_and_limits`：每个插件独立、有限额的解释器。
- `ShellRuntime::stop` / `resource_error` / `heap_usage_bytes`：停止、故障诊断与 JS 堆观测。
- `ShellRoot::clear_stopped_overlays`：同步撤销已停止 VM 的浮层引用；默认停止通知也会触发前台清理，绘制与脚本调用前复查。身份不持有 VM，清理保留其他 owner 和宿主浮层，详见 [G6 记录](../../../docs/plugins/p2-overlay-cleanup-2026-11-05.md)。
- `ShellRuntime::command_target` / `CommandTarget::dispatch`：定向调用具体视图的 `on_command`。
  参数与返回只允许有界数据；仅用于受限解释器。`CommandCall::try_take` 或
  `wait().await` 后的 `CommandReply::accept` 在接收时复查目标代次。
  宿主仍需单独验证 contextVersion；停止使待接收及已缓存结果失效。详见
  [G3 记录](../../../docs/plugins/p2-targeted-commands-2026-11-05.md)。

限额包括 JS 堆、解释器栈、单次执行、滑动窗口累计执行与调用数、原生描述操作、展开后的树深度/
节点数、递归数据转换数/字节和日志数/字节。资源违规使 VM 进入不可恢复使用的状态，取消其任务；
恢复必须显式创建新 VM。普通业务 JS 异常仍可使用原来的错误恢复方式。

解释器中断不能抢占正在运行的 Rust 函数、系统调用、GPUI 布局/绘制或原生组件内部工作。
描述树检查与物化分段检查限制可控的展开成本，耗时会计入累计预算，但单个原生操作只能返回后
确认超时。JS 堆数值不包含 Rust/native/GPU 内存。宿主函数必须返回有界摘要，耗时 IO 放到后台。
这些限制及真实交互性能仍要在后续生产集成中分别验收。
