# P2 前置：G3 定向命令与多视图统一停止

日期：2026-11-05。接续 [G2/G5 与 G1 记录](p2-host-controls-2026-11-05.md)。
本记录保留独立 Shell 原型 G3 候选入口、宿主路由及回归验证。随后完成的清单实现绑定和开发加载器见
[开发加载器进度](p2-development-loader-2026-11-05.md)；正式桌面接入和生产准入仍在后续阶段。主应用没有接入本轮 Shell 依赖。

## 已实现范围

- `ShellRuntime::command_target(&view, cx)` 绑定一个具体视图代次，要求已经安装限额的解释器。
  `CommandTarget` 仅保留视图及 VM 弱引用；目标与 VM 不匹配、释放、卸载、代次失效或停止时拒绝调用。
- `target.dispatch(name, payload, window, cx)` 在目标 Policy、应用代次和 Event 预算下调用
  `on_command(name, payload, cx)`，`this` 为目标 JS 实例；不经过 GPUI 焦点 action 路由。
  禁止脚本调用过程中重入该入口。
- handler 返回纯数据或 Promise。`CommandCall::try_take(cx)` 可消费就绪结果；异步路径为
  `call.wait().await` 后 `reply.accept(cx)`。接收时再次校验视图和运行代次，包括已缓存在通道中的结果。
- 原型 `src/commands.rs` 绑定 owner、view ID、generation、contextVersion，并在派发和接收时验证。
  视图须属于宿主指定 owner；命令仅接受当前原型允许的 `increment` / `read-after-delay`。
  这些名称是夹具约定；后续开发清单通过 command binding 声明实现名，不将夹具名称固定为 SDK。
- 同 owner 的左右视图保存独立状态、共享一个 VM 的预算与停止边界。停止先撤销宿主上下文，
  再停止 VM、清除视图与命令入口；owner-less 任务与待接收命令一并取消。
- 底栏新增 `Increment summary`，只更新右侧计数器；宿主内容区新增焦点探针及结果显示。
  原底栏打开动作、脚本点击、工作区切换和只读行为保留。

handler 的 `cx` 与普通事件相同，是调用期上下文；不能保存到 await 之后使用。
需要异步通知视图时，使用 `cx.spawn(async cx => …)` 提供的任务上下文。
`cx.sleep(...)` 返回的 Promise 可以直接用于异步纯数据结果。未结算 Promise 占用现有调度器
任务名额，每 VM 最多 1024 项；丢弃结果句柄只放弃接收，不撤销已经发生的脚本副作用。
统一取消边界是视图/应用释放和 VM 停止，不承诺强制撤回任意宿主后台工作。

## 数据与故障边界

| 项目 | 原型契约 |
| --- | --- |
| 命令名 | 1–128 ASCII 字符，字母、数字及 `._:-` |
| 参数 / 返回 | null、bool、有限 number、string、array、record；undefined 映射 null |
| 深度 / 总值数 | 根为深度 0，最多深度 16、4096 个值 |
| 字符串及对象键 | 每条数据合计最多 256 KiB UTF-8 字节 |
| 诊断 | 返回给命令调用者的脚本错误最多 4096 UTF-8 字节，保持字符边界 |
| 结果通道 | 每条命令 1 个结果；转换失败和 Promise 拒绝均结算为错误 |
| 运行预算 | 沿用 G5 的单次和累计执行、堆、原生调用与桥接限额；命令无独立免限额通道 |
| 停止与恢复 | 停止后旧命令及缓存结果失效；恢复创建新 VM / 代次，旧句柄不按 ID 自动重定向 |

HostValue 不传递 JS 函数或 GPUI 句柄。循环对象、超大稀疏数组、嵌套函数及非有限数会拒绝；
转换过程逐步检查预算。普通业务异常保持 VM 可用；资源故障使 VM 终止并取消所有任务。
跨工作区结果还必须通过宿主 ticket 校验，单靠 Shell 的应用代次不能代表 Conduit 上下文。

生产隔离仍要求每 owner 独立 VM。原型按工作区切换重建 VM；正式 `ViewInstanceKey` 缓存、
包内入口核验及安装来源由下一步适配器实现，不能将当前夹具的静态映射作为任意包加载协议。
G5 的原生操作不可抢占、JS 堆不等于进程总内存等限制继续适用。

## 验证记录

环境：Windows x64 / MSVC，rustc 1.95.0，Kit / Shell 固定
`11345ec002d9f182a266b5d5dd3a217090700a87`，Cargo.lock 未升级任何版本。

| 检查 | 本轮结果 |
| --- | --- |
| 常规模型 / GPUI 回归 | **19 passed、0 failed、5 ignored**；新增 6 项 G3 集成测试 |
| G3 独立命令故障测试 | **1 passed**；同步与 sleep 后 Promise 死循环均在本机约 **50 ms** 终止 |
| G5 故障回归 | render watchdog 1 项、资源故障 3 项，分别执行全部通过 |
| rustfmt / 默认 check / Clippy | 通过；Clippy 全目标 + ui-tests、`-D warnings` |
| Fast 全目标 + ui-tests 类型检查 | 通过；保留已知上游滚轮 API 弃用警告 |
| 默认 release 类型检查 / dev 编译链接 | 通过；release 仅类型检查，未链接成品 |
| Windows 原生窗口 | 通过；已查看激活/命令截图，右侧计数 1、左侧 0、返回值 1，关闭退出码 0 |
| 全新目录补丁重建 | 通过，与工作副本源码哈希一致 |

G3 六项集成测试覆盖：

1. 宿主内容保持焦点时，底栏更新右侧；左侧不变，反向指定左侧也保持独立。
2. 两 owner 各用独立 VM，同 owner 双视图共享停止；同步/异步身份正确，另一 owner 持续可用。
3. Promise 拒绝、同步抛错、转换失败、参数与返回的数量/深度/字节限制；后续健康调用仍成功。
4. 工作区 A/B 切换、停止及同 ID 新 VM；包括 VM 仍存活而单独 contextVersion 变化的已结算结果。
5. 释放视图后取消永不结算的命令，保留命令句柄也不阻止 VM 弱引用释放。
6. 同 VM 内 manager 卸载/同 ID 重载；保留旧 Entity 和旧命令，也不能绕过应用代次失效。

故障探针在单独进程执行，runner 在编译完成后设置 120 秒外部超时。50 ms 是单机样本，
不代表真实交互 p95。截图与当前命令/退出码保存在忽略的 `artifacts/` 中。

## 可审阅补丁与重现

在 G2/G5、G1 补丁之后追加
[`shell-targeted-commands.patch`](../../tools/shell-prototype/patches/shell-targeted-commands.patch)。
原有补丁不改写；源码仍从精确 Git 提交生成到 `.upstream/`，不修改 Cargo 缓存。

```powershell
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase commands
python tools/shell-prototype/verify.py --phase resources
python tools/shell-prototype/verify.py --phase watchdog
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase fast-check
python tools/shell-prototype/verify.py --phase release-check
python tools/shell-prototype/verify.py --phase build
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1
```

```text
patch_sha256  d52758cff6094b716cb2b06f4b2555532fbd78118309a9b036eaa619af0d595f
source_sha256 2fc1d08ad65257e6f879ec3310b4d68052d3ba9fed7f2d015c7119aaf67cfaf8
lock_sha256   83471be4b3a0187054925f8e015b456457eadcd67521f16de5e43d157b4610d4
```

## 下一步

清单入口、command binding、开发快照和 P1 适配器已完成，具体实现与验证见
[P2-A 开发加载适配器](p2-development-loader-2026-11-05.md)。当前结果支持继续接正式桌面 Shell 入口，仍不构成 Shell 生产后端准入或第二套运行时的交付承诺。

后续分别验收 G4 正式浮层 / 输入法 / 焦点、G5 整体内存与交互性能、G6 关闭 / 更新及宿主写入屏障。Fast 仍只完成类型检查，未链接和运行 Fast 成品；release 也不因此视为成品验收。
