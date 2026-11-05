# P2-A：开发插件加载适配器进度

日期：2026-11-05。状态：**开发加载链路完成并通过原型验收；后续真实桌面开发接入见[单独记录](p2-desktop-integration-2026-11-05.md)，正式发布、持久化信任和性能准入仍未完成。**

本轮把 G3 的定向命令原型接到实际的 P1 `Registry`、`ViewInstanceKey`、`RequestTicket` 和上下文追踪器。加载器只服务明确指定的本地开发目录，创建会话内快照后再运行脚本；它不是安装器，也不改变 Conduit 主窗口的生产实现。

## 已冻结的清单契约

`gpui-shell.json` 提供 Shell 身份、版本、包内入口和系统能力声明。当前开发契约要求：

- `id`、`name`、合法 semver `version`、精确 `shell-version: "0.8.0"`；
- `entry` 必须是包内 `.js` 文件，且必须对应一项声明的 view entry；
- `capabilities` 目前只接受 `{"storage": false}`；dependencies 必须为空。

`conduit-plugin.json` 提供 `schemaVersion: 1`、匹配 0.1.0 的 `hostApi` semver 范围（如 `"^0.1.0"`）、业务权限和三类宿主入口。view、command、activity item、status item 的 ID 继续复用 P1 规则，view 与 command 的关系写入静态 `binding`。以下为贡献片段，完整可加载示例见 [development-overview](../../tools/shell-prototype/fixtures/development-overview/conduit-plugin.json)：

```json
{
  "contributes": {
    "views": [{
      "id": "example.workspace-overview.summary",
      "title": "Workspace summary",
      "location": "right",
      "scope": "workspace",
      "entry": "summary.js"
    }],
    "commands": [{
      "id": "example.workspace-overview.increment",
      "title": "Increment summary",
      "scope": "workspace",
      "binding": {
        "view": "example.workspace-overview.summary",
        "command": "increment"
      }
    }]
  }
}
```

加载前会检查未知字段、owner 和 scope、跨 owner 引用、图标、条件、命令参数预算、权限子集及入口文件。`activationEvents` 如果出现，必须精确覆盖所有声明的 view 和 command；未填写时由静态贡献派生。脚本不能注册或修改静态贡献。

view 的 location 只接受 `navigation` / `right`，scope 只接受 `global` / `workspace` / `session`。command 的 arguments 为 `{kind: "none"}`（默认）或 `{kind: "object", maxBytes, maxDepth}`。activity action 为 `{kind: "openView", view}` 或 `{kind: "command", command}`；status command 引用同 owner 的已声明命令。当前图标表为 folder、file、git-fork、layout-dashboard；when 沿用 P1 的受限条件表达式。

## 快照和 CLI

```powershell
cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype -- --inspect-plugin tools/shell-prototype/fixtures/development-overview

cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype -- `
  --dev-plugin tools/shell-prototype/fixtures/development-overview `
  --allow-permission workspace.metadata.read `
  --allow-permission session.summary.read
```

`--inspect-plugin` 只读取清单和包摘要，不求值 JavaScript。`--dev-plugin` 会把目录复制到宿主拥有的临时目录，重新读取副本、计算摘要并解析清单，然后要求 CLI 明确批准全部业务权限。批准只存在于当前进程；没有持久化信任、在线安装、源文件监听或自动热重载。

快照限制如下：

| 项目 | 当前限制 |
| --- | ---: |
| 单份清单 | 256 KiB |
| 普通单文件 | 8 MiB |
| 包内总文件字节 | 16 MiB |
| 包内目录和文件条目 | 256 |
| 目录深度 | 16 |
| 相对路径长度 | 240 bytes |
| P1 命令参数 | 64 KiB / 深度 16 |

摘要按排序后的相对路径和文件字节计算，使用 `conduit-development-package-v1` 前缀及显式 little-endian 长度。首次激活和每次重新激活都会核验快照；修改已复制快照会拒绝加载，修改源目录不会影响当前会话。

包路径只接受 ASCII 字母数字、点、下划线、连字符和 `/` 分隔符；拒绝绝对路径、`..`、反斜线、Windows 设备名、尾随点、大小写冲突、符号链接、junction / reparse point 和非普通文件。清理仅针对宿主创建的快照。上述检查不构成针对同用户恶意并发文件系统修改的完整 OS 隔离保证。

## 适配器行为

1. `DevelopmentHost::new` 先把所有包的清单转换为 P1 `ContributionSet`，一次性注册；注册失败不会留下部分 owner。此时 VM 数量为零。
2. 打开 view 或执行 command 时，宿主按 owner 创建独立 `ShellRuntime` 和 `Policy`，并把 `conduit` HostModule 闭包绑定到 owner、generation、contextVersion 和批准权限。
3. view 使用 P1 的 `ViewInstanceKey` 和容量为 32 的 `InstanceCache`。同一 owner 的左、右 view 共用一个 VM，但实例状态独立；隐藏 view 保留缓存，重新显示时刷新摘要，隐藏脚本任务仍受 VM 限额约束。
4. command 先通过 P1 校验，再创建 `RequestTicket`，最后调用声明 binding 对应 view 的 `on_command`。返回值接收时再次验证 owner、scope、context、可见性、实例代次和 ticket；过期结果被丢弃，当前脚本异常显示在开发窗口。
5. 工作区、会话、文档、只读或关闭状态变化会使上下文版本失效并停止所有 owner VM。摘要数据变化不增加版本，只刷新当前可见 view。
6. `stop_owner`、`stop_all`、unregister 和窗口销毁都会撤销 epoch、停止 VM、取消实例 ticket、释放缓存；保存的 PreparedCommand 即使此前没有激活，也不能在停止后重新启动 owner。

当前 `conduit` HostModule 只提供 `getContext`、`getWorkspaceMetadata` 和 `getSessionSummary`。前者返回宿主绑定的 pluginId、字符串形式的 generation / contextVersion 和不含路径的 workspaceId / sessionId；后两者分别需要 workspace.metadata.read / session.summary.read，返回当前缓存的工作区名称和合成会话数量。

没有文件、网络、进程、Shell 持久化 localStorage 或宿主 GPUI 句柄权限；Shell 的进程内 sessionStorage 保持其原有语义，不用于授权或恢复。权限检查覆盖模块求值、view init、command 和 await continuation。

## 验证结果

以下结果来自 Windows x64、rustc 1.95.0、固定 Kit/Shell rev `11345ec002d9f182a266b5d5dd3a217090700a87`：

| 检查 | 结果 |
| --- | --- |
| 默认测试与 UI 集成 | 33 个库测试 + 28 个模型 / GPUI 测试通过；6 个有意故障测试默认 ignored |
| 开发适配器 | 1 个注册测试 + 8 个 GPUI 集成测试通过，覆盖惰性激活、双 owner 权限、上下文、ticket、隐藏后刷新、停止、带未完成命令的窗口关闭、快照和错误显示 |
| 开发故障探针 | 1 个通过；异步资源故障只停止故障 owner，另一个 owner 可继续使用 |
| G3 / G5 故障回归 | 原有 5 项独立进程探针重新执行通过；连同新增探针共 6 项 |
| rustfmt、默认 check、Clippy `-D warnings` | 通过 |
| Fast 全目标类型检查 | 通过 |
| release 类型检查、dev 构建 | 通过 |
| Windows 默认原生冒烟 | 窗口创建、激活、命令、关闭均通过，退出码 0 |
| Windows 开发窗口冒烟 | 已查看截图：左侧显示 3 个会话、右侧命令计数 1；停止清空两侧，关闭退出码 0 |
| CLI | --inspect-plugin 成功；缺少批准权限时 --dev-plugin 在创建窗口前拒绝 |

复用的 24 项 P1 测试直接编译自 `src/plugins.rs` 及其子模块。主工程仅增加明确的模块路径以供独立原型复用；新增 semver、sha2、tempfile 是现有锁定版本的直接依赖，没有升级依赖版本。本轮未跑主桌面全量回归，不能据此推断生产集成结果。

```powershell
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase development-fault
python tools/shell-prototype/verify.py --phase commands
python tools/shell-prototype/verify.py --phase resources
python tools/shell-prototype/verify.py --phase watchdog
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase fast-check
python tools/shell-prototype/verify.py --phase release-check
python tools/shell-prototype/verify.py --phase build
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1 -Development
```

当前锁文件 SHA-256 为 `6588645cc092ad094f291190fe02fe79a6467435728240ad97c3ce858fb8b344`。上游补丁和生成源码摘要保持不变：

```text
patch_sha256  d52758cff6094b716cb2b06f4b2555532fbd78118309a9b036eaa619af0d595f
source_sha256 2fc1d08ad65257e6f879ec3310b4d68052d3ba9fed7f2d015c7119aaf67cfaf8
```

## 边界与下一步

本记录对应的独立开发窗口只演示固定宽度左右区和静态底栏；后续已通过独立桌面二进制接入真实 ConduitApp、工作区摘要与关闭 / 更新停止调用，见[桌面接入记录](p2-desktop-integration-2026-11-05.md)。CLI 一次接受一个包；多个 owner 在适配器集成测试中验证。

快照复制和激活核验是开发会话中的有界同步 I/O，生产 p95、总进程内存、G4 浮层 / 输入法 / 焦点、G6 退出与更新写入屏障仍需单独验收。隐藏和替换 ticket 只取消结果发布，不能撤销已执行的脚本副作用；停止 VM 才取消其全部任务。Fast 和 release 目前仅类型检查，未链接 / 运行相应成品。进程内 VM 也不等于操作系统沙箱。

后续桌面接入已复用同一 adapter 契约。下一步补齐浮层所有权清理、真实输入法 / 焦点、完整关闭 / 更新屏障与性能测量；安装、信任记录、版本替换和回滚继续放在 P3。
