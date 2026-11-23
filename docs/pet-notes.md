# 桌面宠物实施与验收记录

对应 [正式设计](design/desktop-pet.md)，2026-11-23。

## 已核对的依赖与适配

- `Cargo.lock` 选择 gpui-pre / gpui-pre-windows 0.3.9；GPUI Kit 固定为 `11345ec002d9f182a266b5d5dd3a217090700a87`。沿用仓库 vendor 补丁，不修改 Cargo 缓存。
- 该版本 `WindowKind::PopUp` 默认扩展样式含 TOOLWINDOW/TOPMOST，但窗口样式为 0。继续复用已有无边框和 DirectComposition 适配。
- `gpui-pre-windows/src/events.rs` 的 WM_MOUSEACTIVATE 默认返回 MA_ACTIVATE，按下处理已有 SetCapture。新增仅作用于宠物的 subclass 返回 MA_NOACTIVATE，并在 WM_NCDESTROY 移除；不能只依赖 NOACTIVATE 样式。
- `Context::observe_window_bounds` 可订阅窗口尺寸变化；`background_executor().now()` 使用可测试时钟。无需自行取系统时间或保留 180ms 常驻轮询。
- 原生 GPU 验收确认 `bounds_changed` 同步调用 bounds observers。在 PetView 的 render/update 中直接修改窗口会重入同一实体；移动和缩放都通过 `window.defer` 在释放视图借用后执行。`cx.defer_in` 仍会借用实体，不能代替它。
- `canvas` 支持逐个 paint_quad，窗口通过 raw-window-handle 0.6 获取 HWND。继续保留已有 bounds_changed 同步以避免原生无框尺寸与 GPUI 视口失配。
- 根 workspace 排除独立的 `vendor/gpui-kit` workspace，避免其 `workspace.package.edition` 错误继承根配置。

## 本轮实现

- `pet-core`：无外部依赖；身体、脸型、特效和半透明阴影；合成后按行合并；状态与结果期限；xorshift 动画；确定性的下一次唤醒。
- 动画采用同一个时间轴，迟到 tick 按事件时间顺序消耗随机数，保持与正常唤醒一致。Sleep 共用 1400ms 节拍。
- PetView：像素 canvas，保留已有鼓励语/气泡/菜单/隐藏；动画、状态、气泡、位置写入统一选最早期限。宿主状态未变时不重启动画。
- 宿主适配汇总所有运行和审批/清理阻塞；有效且未取消的 Message 终态发送成功/失败反馈。显示不会发起模型请求。
- Windows：NOACTIVATE 与 WM_MOUSEACTIVATE 配套；仅物理按下期间启用 16ms 拖动恢复检查，解决后台捕获不能保证窗口外松手的问题。松手/关闭即取消，没有空闲鼠标轮询。
- 像素尺寸按物理像素取整，DPI 变化重新计算窗口；保持已有逻辑坐标 Store 格式。没有新增 TCP 或 pet.json。

## 首轮像素重构自动化检查

- `cargo test --locked -p pet-core`：7 项通过，覆盖身体/七张脸快照、合法部件、全部特效边界和运动组合、状态期限、十分钟动画、眨眼范围、迟到 tick。核心 Clippy（all-targets、warnings 作为错误）通过。
- `cargo test --locked --features ui-tests --bin conduit desktop_pet -- --test-threads=1`：9 项通过、2 项原生测试按默认规则忽略。覆盖原有宠物与设置、后台任务、休眠、气泡暂停、实体释放和 DPI 尺寸。
- `cargo test --locked --features ui-tests --bin conduit desktop_pet_pixel_native_snapshots -- --ignored --nocapture --test-threads=1`：单独启用的真实 Windows/GPU 测试通过；使用隔离内存配置，无模型服务。八种状态均生成 150×114 PNG，透明区域超过一半、阴影存在半透明像素，WM_MOUSEACTIVATE 返回 MA_NOACTIVATE，窗口保留 NOACTIVATE 样式。
- 已目视检查 `target/pet-pixels/contact-sheet.png`：八种状态身体完整、脸型可区分，感叹号/爱心/Z 白边完整，无不透明底板；截图对应 100% 缩放。原始透明 PNG 同目录保存，全部属于 ignored 验收产物。
- 收尾的根包 all-targets Clippy（含 ui-tests、`-D warnings`）通过。vendor 校验通过（555 文件、11 项声明补丁）；中英文各 3427 条翻译校验通过；最终 `cargo fmt --all --check` 及 `git diff --check` 通过。
- `cargo build --locked --bin conduit --bin conduit-updater --bin conduit-launcher`：三个发布入口均编译通过，未启用测试专用 features。日志为 `target/pet-build-final.log`。

`scripts/check-core.ps1` 已加入 pet-core 检查。首轮全量执行中，库测试 61 通过、1 忽略；主程序 945 通过、3 失败、27 忽略。桌宠的两项失败分别来自测试在宠物窗口已借用时关闭同一窗口、以及把测试窗口缩放误当成 1，已修正测试上下文/物理像素断言并通过上述专项复跑。另一项既有 Git 冲突测试 `conflict_review_preserves_drafts_and_refuses_stale_save` 单独复跑通过（1/1），未修改其代码。未把这轮结果记为全量全绿；修正后未重复整套耗时 Git 回归。

## 尚需实机覆盖的范围

真实 125%/150% 显示缩放、不同显卡、多屏混合 DPI、热拔屏、拖动跨屏以及真实鼠标点击前后前台焦点需要单独记录。数学坐标测试和 WM_MOUSEACTIVATE 消息断言不能替代这些系统验收。此次 computer-use 原生管道连接失败，重试及重置后仍不可用，因此未将真实鼠标自动验收算作通过。

没有调用用户模型服务；实际凭据、网络和付费模型响应不是此轮像素重构测试的一部分。

## 回复和右键菜单闪烁修正（2026-11-23）

用户实测发现回复出现、右键菜单展开时闪烁，且文字有截断感。旧实现每次改变原生窗口尺寸，锁定的 Windows 渲染器随之调用 ResizeBuffers；视图还在旧尺寸下先绘制一帧，再 defer 调整窗口。改为固定 320×312 透明画布，只有 DPI 变化需要同步尺寸。角色与气泡的实际布局矩形通过 SetWindowRgn 限定可见及鼠标区域，收起时释放气泡区域，不保留覆盖桌面的透明大矩形。恢复和拖动时预留可见的气泡空间。

回复档位调整为 40／80／120 字符，并将具体上限放进模型指令；输出归一为单段，超长时优先结束于完整句子，否则在字形/单词边界保留省略号。气泡正文使用剩余高度滚动，按钮栏不可收缩。此轮未调用实际模型服务。

回归测试 `encourager`：12 项通过、2 项原生测试默认忽略；`desktop_pet`：10 项通过、2 项默认忽略。新增覆盖反复右键、回复出现和收起时画布/角色位置保持不变，以及文本和按钮的布局、句末收束与 emoji 字形完整性。

单独启用的原生 Windows/GPU 测试通过（1/1）：八种状态及回复/菜单/收起均已截图；反复切换断言原生窗口边界和持久化角色锚点不变；GetWindowRgn/PtInRegion 确认内容区域有效、空白及已收起气泡区域被排除。后续截图画布为 320×312，目视检查最长回复完整、按钮未被挤压。对照图为 `target/pet-pixels/bubble-contact-sheet.png`。这些检查不替代真实鼠标操作下的连续屏幕录像观察。

最终根包 all-targets Clippy（含 ui-tests、`-D warnings`）、全仓格式、i18n 和 diff 空白检查均通过；`conduit`、`conduit-updater`、`conduit-launcher` 三个非测试入口编译通过。日志分别保存在 ignored `target/pet-bubble-*.log`。
