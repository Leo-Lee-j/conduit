# P2-A：G6 浮层所有权与停止清理

日期：2026-11-05。承接[真实桌面开发接入](p2-desktop-integration-2026-11-05.md)，补齐 ShellRoot 保留脚本浮层时的清理路径。正式发布依赖仍未启用，G4 / G5 生产准入及未来插件存储写入屏障仍未完成。

## 实现

候选补丁为 [shell-owned-overlays.patch](../../tools/shell-prototype/patches/shell-owned-overlays.patch)，继续由 `prepare_upstream.py` 从固定提交和补丁系列生成；不修改 Cargo 缓存。根应用仅在 `conduit_shell_desktop` 下使用新增接口。

- 每次创建 VM 生成独立浮层身份，身份只持有停止信号，不反向持有 VM。一个插件重开后获得新身份；旧回调与清理不能操作新实例。
- dialog / sheet 记录创建它们的运行时。停止、资源违规和不可恢复的任务队列错误关闭信号，ShellRoot 的前台任务据此清理；绘制前和下一次脚本浮层调用也复查，避免等待定时轮询或旧侧滑面板阻挡新实例。
- 按身份移除通知、HUD 和 tooltip；通知 ID 在运行时内命名，另一插件使用同名 ID 不会覆盖它。tooltip 只持有原生文字视图，但其延迟显示和隐藏任务同样需要取消。
- 脚本只能关闭自己的弹窗、侧滑面板、通知和 HUD；已有其他 owner 的 sheet / HUD 时保留原内容。宿主 Rust 调用仍能按窗口规则主动关闭浮层。
- 删除中间浮层时修复仍存活浮层的焦点恢复目标，跳过被移除的根或其子控件；当前焦点属于宿主或其他插件时不移动它。旧帧的原生关闭监听器核对浮层身份，不能误关后来打开的层。
- 修复锁定上游 `window.open_sheet()` 的默认参数：JS 包装器传入 `undefined`，原 `Opt<String>` 不能将它解释成默认位置。现在默认和显式位置均走合法解析。

新增宿主接口：`ShellRoot::clear_stopped_overlays(window, cx)`。Conduit 的停用、真实窗口关闭和更新准备在停止运行时后同步调用，更新线程创建前已撤销 ShellRoot 引用。上下文变化、只读、注册撤销和 adapter 丢弃通过同一停止信号处理。GPUI 当前帧保留的视图引用在正常帧退休时释放；停止标记先行阻止脚本重入，因此不能将同步撤销 root 引用描述成所有图形资源已即时销毁。

Base 补充 `ToastManager::retain` 用于立即撤销指定生命周期条目，以及 `TooltipOverlay::is_visible` 用于验证显示状态。清理不依赖脚本 deactivate，也不通过关闭整个窗口或所有 owner 的浮层实现。

## 验证

测试和最终构建结果在本轮完成后记录于此。日志及可重现摘要位于忽略的 `tools/shell-prototype/artifacts/`。

新增集成用例位于 [overlay_tests.rs](../../tools/shell-prototype/src/overlay_tests.rs)，覆盖宿主与双插件交错弹窗、焦点恢复、sheet / HUD 防替换、同名通知、迟到异步打开、旧命令句柄、重开代次及鼠标不移动时 tooltip 清理。已有资源故障探针先打开实际 dialog / sheet / toast 再触发异常。

桌面用例扩展了停用、工作区切换、只读、注册撤销、关闭、更新准备，并验证宿主设置弹窗仍保持打开和焦点。更新用不存在的安装包检查准备接缝，不启动安装器。

```powershell
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase watchdog
python tools/shell-prototype/verify.py --phase resources
python tools/shell-prototype/verify.py --phase commands
python tools/shell-prototype/verify.py --phase development-fault
python tools/shell-prototype/verify-desktop.py --phase test
python tools/shell-prototype/verify-desktop.py --phase check
python tools/shell-prototype/verify-desktop.py --phase fast-check
python tools/shell-prototype/verify-desktop.py --phase build
pwsh -NoProfile -File tools/shell-prototype/smoke-windows.ps1 -Desktop -Overlays
```

`-Overlays` 使用独立测试数据目录，仅操作本次 PID 的窗口，捕获脚本弹窗、Esc 后的侧滑面板、重新打开后的关闭前状态，最后通过 WM_CLOSE 验证带浮层退出。截图须实际查看，文件存在不等于通过。

## 下一步与边界

下一步为 G4：真实 Windows IME 组合输入，以及宿主 / Shell 浮层同时存在时的完整焦点、Esc、主题和输入行为；随后补 G5 主线程交互 p95、宿主总内存与原生视图开销测量。

当前 HostModule 仍只有只读摘要，尚无插件存储写入；本轮不声称完成未来写入屏障或 P3 升级事务。Fast / release 类型检查不能代替对应成品的链接与运行。只有完成剩余准入后才冻结 P2-A 生产契约并进入 P3 安装管理。
