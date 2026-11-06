# P2-A：G4 输入、焦点与主题回归

2026-11-06 后续：桌面 check / Clippy 的旧阻塞已在新基线上复核消除，真实 Windows IME 仍因原生连接不可用而未验收；独立推进的 G5 测量工具及结果见[桌面性能测量入口](p2-performance-probe-2026-11-06.md)。下文保留 2026-11-05 的历史结果。

日期：2026-11-05。承接 [G6 浮层清理](p2-overlay-cleanup-2026-11-05.md)。本轮补充真实桌面组合结构中的自动化输入与交互回归；真实 Windows IME 候选框、系统组合输入事件和原生视觉验收仍未完成，G4 尚不能标记为生产准入通过。

## 本轮范围

- [开发夹具](../../tools/shell-prototype/fixtures/development-overview/summary.js)的摘要、dialog 和 sheet 均提供保留状态的原生输入框，采用 `gpui-component` 的 `InputGroup` / `InputGroupInput`，有明确可访问名称。摘要显示当前输入和主题，便于原生验收。
- [桌面交互测试](../../src/app/plugin_ui/plugin_shell/tests/interaction.rs)使用组件 Root → ShellRoot → ConduitApp 的实际组合，覆盖中文 / emoji 编辑、粘贴、明暗主题切换、模态 Tab / Esc、焦点恢复、设置页往返和窄窗输入状态。
- 组合输入测试通过 `EntityInputHandler` 注入标记、更新、提交与取消，检查混合 UTF-8 / UTF-16 范围、定向插件命令不抢焦点和单次撤销。这是输入处理接口回归，不能代替真实 Windows 输入法。
- 共享仓库的设置页已改为独立工作区。宿主模态回归使用实际组件 Root 的原生 dialog，另测设置页返回后的插件输入焦点与草稿保留；不再将“打开设置”当作“打开宿主弹窗”。
- 跟随主工程版本 0.1.8 同步实验桌面 manifest 和实验 lockfile 的两个本地 package 版本；不升级外部依赖。桌面检查 runner 纳入新增测试文件的格式检查。

## 验证记录

日志位于忽略的 `tools/shell-prototype/artifacts/`。本轮改动未改变上游补丁，候选上游源码摘要仍为 `e8d19bbb0faaa950cdb8f3c7dde5938bb3c06e9b2f3c43576d6b731b05e72171`；版本同步后的实验 lockfile SHA-256 为 `11d0245a30ef8c8168979a31e30fd185120b0d769fc018c743e927f6bcb57fde`。

| 检查 | 结果 |
| --- | --- |
| 新增桌面交互回归 | 4 项通过；`g4-interaction-tests.log` 记录实际编译与运行结果 |
| 原型常规回归 | 33 项库测试 + 34 项 UI / model 测试通过，共 67 项；6 项故意故障测试保持 ignored，本轮没有重新执行 |
| 已构建桌面二进制回归 | 42 项 `app::plugin_ui`（含新增 4 项）+ 3 项更新测试通过，共 45 项；与原型合计 112 项，不重复计算新增 4 项 |
| 原型格式、check、Clippy | 通过；all-targets + ui-tests Clippy 使用 `-D warnings` |
| 桌面相关文件格式 | runner 的 rustfmt 检查通过，包含新增测试文件 |
| 桌面 all-targets + ui-tests check | 最后一次运行通过，耗时 24.145 秒；见 `desktop-check-results.json` / `desktop-check.log` |
| 桌面 all-targets + ui-tests Clippy | 未通过，`-D warnings` 检出并行改动中的 7 项问题；见 `desktop-clippy.log`，详情如下 |
| 真实 Windows IME / 原生视觉 | 未完成，自动化连接不可用；本轮无新原生截图 |

桌面二进制回归使用本轮已成功构建的 `target/shell-prototype/debug/deps/conduit_shell_desktop-3c475ad861da8dfb.exe`，SHA-256 为 `57d8d99240a82677cf5d2c562ff0d460f6b33907c80a8b86e16d5f745e00e957`，对应 `g4-desktop-built-ui.log` / `g4-desktop-built-update.log`。共享源码随后继续变化，因此该结果不能推定任意后续工作树快照已经通过。

最后一次桌面 check 已通过；Clippy 仍报告并行改动中的 7 项问题：`src/app/tools.rs` 两处参数过多，`src/transcript/chunks.rs` 一处返回类型复杂，`src/transcript.rs` 三处多余借用，以及 `src/app/chat/rows.rs` 在测试模块之后声明实现。根工程 `cargo fmt --all --check` 也曾报告并行修改中的格式差异；本轮没有批量改写其他任务的文件，也不复用 G6 的全工程检查结论。

`desktop-check-results.json` 记录 `source_changed_during_checks: true`：检查前后的桌面源码摘要分别为 `50153736ad756734c41955a7148780c33c8f3a458af761efe96f03b5eb747c45` 和 `4cad0db6304826a12c81ac9b4265545e62ce3c7ede042830261449b7464c500e`。因此上述分项结果不能组合成“当前稳定工作树已通过全部检查”的结论；并行改动收敛后仍需复核。

```powershell
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify-desktop.py --phase test
python tools/shell-prototype/verify-desktop.py --phase check
```

850px 下右侧工具栏沿用覆盖中央内容的窄窗设计；测试检查输入框处于面板内、面板不遮挡底栏、关闭后恢复中央可用空间及重新打开保留草稿。G6 截图中的中央内容遮挡不能仅凭截图认定为内容宽度被错误裁切；原生视觉与点击验收仍需继续。

## 原生验收与后续

本轮 Windows Computer Use 返回 `Computer Use native pipe is unavailable`，两个现有入口均无法连接。因此没有把直接注入中文或 GPUI 测试上下文当成真实 IME 通过，也没有复用旧截图作为本轮证据。

恢复连接后，使用独立数据目录启动实验桌面，验证：

1. 在宿主输入框和插件摘要输入框用 Windows 中文输入法输入拼音，观察候选框位置；分别以空格 / Enter 提交、Esc 取消，检查没有重复文字、误发送或误关浮层。
2. 在插件 dialog / sheet 输入框执行同样操作，先取消候选再逐层关闭；检查 Tab / Shift-Tab、宿主原生表单叠放及焦点返回。
3. 保留输入草稿切换明暗主题、进入设置再返回；检查文本、焦点、提示和背景均同步。
4. 在 850 / 1100 / 1600px 窗口检查右侧面板、底栏、输入法候选位置及关闭后的中央内容恢复，并保存实际查看过的截图。

完成 G4 原生验收后，再做 G5 UI 交互 p95、宿主总内存与原生视图开销测量。Fast / release 成品运行、未来插件存储写入屏障及 P3 安装管理仍未完成。
