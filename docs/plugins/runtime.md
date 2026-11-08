# Shell 开发运行时

当前为受控开发候选，不是可安装到正式 Conduit 的插件规范。运行与构建命令以[原型 README](../../tools/shell-prototype/README.md)为唯一入口，精确基线和候选补丁见[补丁说明](../../tools/shell-prototype/patches/README.md)。

## 清单与静态发现

| 文件 | 责任 |
| --- | --- |
| `gpui-shell.json` | 插件身份、版本、JS 默认入口及 Shell 系统能力声明 |
| `conduit-plugin.json` | 宿主 API、贡献、业务权限与显式激活声明；不重复声明插件身份和系统权限 |

当前开发加载器要求 Shell `shell-version` 精确为 `0.8.0`，宿主 `schemaVersion` 为 1，`hostApi` semver 范围匹配 0.1.0；这些都不等于 Conduit 应用版本。完整可运行清单见[development-overview 夹具](../../tools/shell-prototype/fixtures/development-overview/)。

静态贡献复用[原生贡献契约](contributions.md)。`views[].entry` 指向包内 JS；`commands[].binding` 指向同 owner、同 scope 的 view 和 `on_command` 名称。视图位置为 navigation/right，作用域为 global/workspace/session；激活事件只有 `onView` / `onCommand`，显式提供时必须完整覆盖声明入口，省略时由贡献派生。

未知宿主字段、非法路径、跨 owner 引用、保留身份、不兼容版本或额外依赖均拒绝。静态预览不执行 JS，脚本不能再动态注册同一份入口。空工作区不实例化 workspace/session 视图。

## 快照与授权

`--inspect-plugin` 只读清单和摘要。`--dev-plugin` 将包复制到宿主管理的临时目录，重新读取副本、核验摘要，再要求显式批准全部业务权限；授权仅在当前进程有效。源目录修改不影响当前快照，无自动监听、热重载或持久化信任。

| 限制 | 当前开发值 |
| --- | ---: |
| 每份清单 | 256 KiB |
| 普通单文件 / 包内总字节 | 8 MiB / 16 MiB |
| 目录与文件条目 / 深度 | 256 / 16 |
| 相对路径 | 240 bytes |
| 命令参数 | 64 KiB / JSON 深度 16 |
| 视图实例缓存 | 32 |

路径仅接受约定的 ASCII 相对路径；拒绝绝对路径、`..`、反斜线、设备名、尾随点、大小写冲突、符号链接、junction/reparse point 和非普通文件。摘要包含排序后的路径与字节及格式域标识；首次和再次激活均核验快照。清理只针对宿主创建的路径。

每个 owner 独占 `ShellRuntime` 与显式 `Policy`，不临时切换全局默认 Policy。当前只批准 `workspace.metadata.read` 和 `session.summary.read`。`conduit` HostModule 提供 `getContext`、`getWorkspaceMetadata`、`getSessionSummary`，返回有界摘要，不暴露 GPUI 实体、函数、任意路径或文件句柄。

没有文件、网络、进程或持久化 localStorage 授权；进程内 sessionStorage 不承担信任或恢复。上下文、运行代次和权限在模块求值、视图初始化、命令及异步续体处检查。同用户的恶意并发文件修改不在完整 OS 隔离保证内。

## 激活、命令与停止

注册所有静态贡献后 VM 数为零。只有显式打开视图或执行命令才激活 owner；同 owner 左右视图共享 VM，各实例状态独立。隐藏视图可保留缓存和受限任务，再显示时刷新摘要。

命令先经过原生校验并创建票据，再由 `CommandTarget::dispatch` 定向调用目标 view 的 `on_command`，不依赖当前键盘焦点。返回值有界，接收时核对 owner、scope、contextVersion、实例代次及票据；当前错误可见，过期结果丢弃。

工作区、会话、文档、只读或关闭状态变化使上下文失效并停止旧 owner。`stop_owner`、`stop_all`、注销和窗口销毁撤销代次、取消任务与票据、释放缓存；旧 PreparedCommand 不能重新激活已停止的 owner。

根结构为组件/Base Root → ShellRoot → ConduitApp。浮层带运行时所有权；停止时只撤销该 owner 的 dialog/sheet/toast/HUD/tooltip，保留宿主和其他插件，并恢复有效焦点。关闭及更新准备同步接入清理；未来存储写入还需持久化屏障，见[后续方案](roadmap.md)。

## 资源与恢复边界

补丁对 JS 堆、栈、单次/累计执行、调用数、原生描述操作、展开树和转换数据、日志分别设限。违规 VM 不再复用，取消其任务，恢复必须显式新建 VM；普通业务异常按对应恢复路径处理。

解释器中断不能抢占正在运行的 Rust 函数、系统调用、GPUI 布局或绘制。原生操作只能返回后确认超时，JS 堆也不包含 native/GPU 内存。因此 HostModule 必须提供有界摘要，耗时 I/O 放后台；拥有一个 VM 不等于拥有隔离线程或防崩溃沙箱。

缓存挂载使用 `ScriptView::set_fill_container` 与宿主尺寸约束，验证原生输入和百分比布局。更改缓存边界必须覆盖主题、焦点、尺寸、动态状态与注册表身份失效。

以上行为已有自动化验证入口，真实 IME、GPU、Fast 成品和性能结果仍按[准入清单](validation.md)判断。
