# P2 前置：G2 / G5 宿主控制、G1 兼容与下一步契约

日期：2026-11-05；状态：G2/G5 候选补丁已通过原型验证，G1 Fast 全目标类型兼容已修复。本文接续 [P1-5](p1-5-progress-2026-11-05.md)，
不把代码完成或原型测试等同于 Shell 生产准入。

## 本轮修改

补丁与重现方式见 [patches/README.md](../../tools/shell-prototype/patches/README.md)。
基线仍为 `11345ec002d9f182a266b5d5dd3a217090700a87`，没有升级 GPUI 或修改 Cargo 缓存源码。
仅原型工程通过 Cargo patch 使用生成的上游副本；正式应用保持现有 P1 贡献层。

G2 新增 `PluginManager::load_with_policy` 与 `ShellRuntime::load_view_with_policy`。
宿主在求值前提供裁剪后的能力和 HostModule，manager 固定清单 ID 与数据目录身份。
原型不再设置默认全局 Policy。模块求值、初始化、挂载、异步续体和回调继续使用上游已有的
Policy / application generation 传播路径，停止还会显式撤销 VM 的任务及后续入口。

G5 新增 `RuntimeLimits` 和 `new_isolated_with_components_and_limits`。
受限实例使用解释器；宿主必须按 owner 创建独立 VM，避免堆与累计预算被不同插件共享。
资源违规进入终止状态，不能通过重置计数器继续使用旧 VM；用户重新激活创建新实例。
普通可恢复 JS 业务异常不等同于资源违规。

| 默认配置 | 数值 | 约束对象 |
| --- | --- | --- |
| JS 堆 / 栈 | 64 MiB / 512 KiB | QuickJS 管理的内存与解释器栈 |
| render / event / module | 25 / 50 / 100 ms | 单次脚本入口；Promise 批次也计入 |
| 累计执行 / 调用数 | 200 ms、4096 次 / 滑动 1 秒 | 同一 VM 的脚本、转换与同步宿主工作；原生物化另以 render 预算检查 |
| 原生描述操作 / 节点 / 深度 | 20000 / 2048 / 64 | 描述构建、重复子树展开和递归物化前检查 |
| 数据转换 | 20000 个值、1 MiB 字符串 / 入口 | HostValue、宿主 JSON 转换与日志转换的有界路径 |
| 日志 | 100 条、64 KiB / 滑动 1 秒 | 脚本 console 输出，在写入 tracing 前检查 |

这些是原型起点，不是产品性能承诺。JS 堆不包含 Rust、GPUI、图像、字体、GPU 内存。
描述检查不能抢占一个正在执行的 Rust 函数、系统调用或组件内部布局；超时只能在下一检查点
或返回后确认。因此生产宿主必须限制同步函数和可用组件，耗时工作走后台、有界数据回传。
完整应用的内存与交互 p95 仍单独验收，不能用 25 ms watchdog 代替。

## 验证记录

以下为应用本轮补丁后的结果；未打补丁的初版结果保留在 [原型基线](shell-prototype-validation.md)。
本轮环境为 Windows x64 / MSVC，`rustc 1.95.0 (59807616e 2026-04-14)`、
`cargo 1.95.0 (f2d3ce0bd 2026-03-21)`；与初版记录中的工具链不同，结果分别保留。

| 检查 | 结果 |
| --- | --- |
| 正常模型 / GPUI 回归 | **13 passed、0 failed、4 ignored**；含 5 项模型、8 项 UI / 权限测试 |
| 独立 render watchdog | **1 passed**，默认 render 预算 25 ms，实测 27 ms；停止后新实例恢复 |
| 独立资源故障测试 | **3 passed**；故障矩阵、累计执行、原生返回边界分进程执行 |
| rustfmt、默认 check、Clippy all-targets + ui-tests `-D warnings` | 通过 |
| 默认 dev 可执行文件 | 编译、链接通过 |
| 默认 release 类型检查 | 通过；未链接 release 成品，未测 GPU 性能 |
| Windows 原生窗口冒烟 | 通过；已查看激活前后截图，底栏激活后左右脚本视图显示，关闭退出码 0 |
| Fast all-targets + ui-tests 类型检查 | 通过；有上游弃用警告，未运行 Fast 成品 |
| 补丁重建 | 全新目录生成后与工作副本源码 SHA-256 一致 |

资源故障矩阵使用 32 MiB JS 堆、100 ms module、150 ms render / event、500 ms 累计预算。
时间包含加载、挂载及对应测试动作，属于本机单次探针，不能作为生产 p95。

| 故障 | 实测时间 | 触发限制 |
| --- | --- | --- |
| 模块死循环 | 100 ms | 单次执行 |
| 事件死循环 | 161 ms | 单次执行 |
| owner-less 定时任务死循环 | 157 ms | 单次执行 |
| Promise 死循环 | 156 ms | 单次执行 |
| 有界分配增长 | 7 ms | JS 堆；故障后观测 32,800,396 bytes，约 31.3 MiB，低于 32 MiB 配置 |
| 描述节点过多 | 9 ms | 原生描述节点 |
| 日志过多 | 6 ms | 日志速率 |
| 递归数据过多 | 12 ms | 转换值总数 |
| 描述树过深 | 5 ms | 展开深度 |

每个矩阵故障都验证旧 VM 拒绝重新加载、弱引用最终释放、另一正常 VM 仍可渲染，最后显式
激活新实例恢复。累计测试用多个约 8 ms 的短 render 耗尽 80 ms 累计预算。
原生边界测试将 Rust 函数阻塞 30 ms、render 预算设为 10 ms：实际 32 ms 后判定超时，
脚本捕获异常后再次调用无参数宿主函数仍被拒绝。这直接验证“返回后计费”，没有实现原生抢占。

G2 的双 owner 测试特意共享一个 VM，验证同名 HostModule、不同 storage 授权、动态导入、
module/init/click/await 身份、卸载取消和同 ID 新代次的隔离；生产资源隔离仍要求每 owner
独立 VM。静态发现和拒绝授权均不求值脚本，保留旧视图也不能复活已卸载回调。
保留旧 bool `load` 无法导入未授权模块的负例，不将它作为新注入 API 的通过证据。

结果文件位于原型 `artifacts/`，包含真实命令和退出码；本轮标识为：

```text
patch_sha256  41bb09d22d60c3f540de5519a8ce1b25eb248d74719e7c16eb7ebc3b6ea613e0
source_sha256 dabf3e707e29f59cf627d8e9383a69f2a19ede006726a0092fd384b666b17dca
lock_sha256   83471be4b3a0187054925f8e015b456457eadcd67521f16de5e43d157b4610d4
```

`verify.py` 各阶段会更新同名结果及日志；原型基线文档保留历史数字，不应把当前生成的
`check-fast.log` 或 `watchdog.log` 当作未打补丁时的日志。

## G1 Fast 兼容

沿用锁定的 `gpui-fast 0.1.11`，补齐 Base、Component、Kit 测试工具和 Shell 的 API 差异：
修饰键判断传入 App；动态快捷键解析使用宿主 OS；动态键绑定使用 `KeyBinding::load`。
键绑定仍在全部校验成功后统一安装，避免无效脚本参数导致 panic 或部分安装。
默认后端保留原 API 路径。

`--locked --features gpui-fast,ui-tests --all-targets` 类型检查已通过；上游滚轮类型及方法
的弃用警告仍存在。这个结果修复原型的 Fast 编译阻塞，不代表 Fast 成品已链接、运行，
也不代表正式应用已经合并这些依赖。补丁重建所得源码与工作副本逐字节一致。

## 下一步运行时契约

后续实现：下列 G3 定向命令与多视图停止原型已完成，结果见
[G3 进度](p2-targeted-commands-2026-11-05.md)。清单绑定与开发加载适配器也已完成，见[开发加载器记录](p2-development-loader-2026-11-05.md)。本节保留当时交接契约，当前下一步为正式窗口接入与生产验收。

以下为当时给适配器和 G3 定向命令验证的约束；已实现开发契约见上述最新记录，生产 SDK 尚未交付。

| 边界 | 契约 |
| --- | --- |
| 激活身份 | owner 来自已验证注册；每次激活分配新的 generation。HostModule 闭包捕获 owner / generation，脚本不能自行指定调用身份 |
| 实例身份 | 继承 P1 `ViewInstanceKey` 和 `contextVersion`；同 owner 的视图独立保存状态，作用域切换使旧请求失效 |
| 授权 | 主机创建空 Policy，再逐项加入批准内容。清单请求不直接变成授权；M0 不开放 Shell storage、文件、网络、进程、exit 或额外环境变量 |
| 资源 | 一 owner 一 VM；限制在求值前安装，所有该 VM 的视图与任务共用预算。隐藏不等于停止，不可见任务仍受限 |
| 故障 | 限额故障 → Failed → 停止接收调用 → 取消任务并撤销实例；显式激活才创建新 generation |
| 正常停止 | 宿主先关闭接受入口、撤销上下文，再 stop VM、释放视图、等待已接受宿主写入、释放模块与 VM；写入屏障仍属于后续实现 |
| 命令 | 必须指向 owner / instance / command，并与焦点路由无关；不能用 ShellAction 或传递 JS 函数的 HostValue 假装已有定向总线 |
| 数据 | 只返回过滤、缓存、有界的工作区及会话摘要；禁止同步扫描全文聊天或工作区；所有异步返回再次经过 P1 ticket / generation 校验 |

本轮完成 G2/G5 宿主控制候选补丁与 G1 Fast 类型兼容；当时的实现顺序：G3 定向命令与多视图统一停止原型 →
冻结两份清单的实现绑定字段 → 开发加载适配器 → 正式窗口、退出与更新回归。
在剩余准入条件和生产测量完成前，P2-A 仍为候选后端；不并行交付第二套运行时。

G3 的历史交接范围（开发加载器阶段已完成前四项中的实现部分）：

1. 先扩展可审阅的公开定向入口，宿主持有目标视图句柄，参数与返回值仅为有界数据；
   handler 始终在目标 Policy、应用代次和资源预算下执行。此时不冻结 Conduit 清单字段。
2. 在原型中让同一 owner 的左、右两个实例共用 VM 停止边界，分别保存视图状态；
   保持焦点在宿主内容区，从底栏将命令送到指定实例，并检查其他实例未改变。
3. 用两个 owner、工作区 A/B 切换、停止后迟到 Promise、同 ID 重载验证命令不会串到
   错误 owner / scope / generation；停止后所有视图、owner-less 任务和旧句柄都不可复活。
4. 以上通过后，才把入口绑定字段写入两份清单并接开发加载器。G4 正式浮层/焦点、
   G5 整体内存与交互性能、G6 退出/更新写入屏障仍分别保留验收。
