# 方案 A 原生实现

方向与密度约束见 [direction-approved.md](direction-approved.md)。本轮修改 Rust/GPUI 设置界面，原型仍保留为方向依据。

## 统一范围

通用、外观与字体、快捷键、AI 引擎、模型与服务、会话设置、Git 工作流、Skills、MCP 服务、插件、数据与更新，共 11 个入口使用同一设置外壳。

- 设置在同一窗口内独立展示；顶部为返回工作台和全局搜索，中间为分类及当前页面，底部为真实保存状态。
- 页面正文左对齐、最大宽度 1120 px；搜索最大宽度 480 px。导航默认宽 184 px，空间不足时换为分类下拉框。
- 正文基准 14 px，辅助信息 12 px，常用按钮 32 px。窗口尺寸不驱动字号和按钮缩放；用户明确调整界面字体时按字体适配。
- 偏好使用紧凑分组；引擎和服务使用列表/详情；扩展使用资源行及按需展开；快捷键使用命令、绑定、范围列。
- 表单使用 `settings/controls.rs` 的共享规范：单行输入和选择器基准高 28 px、字号 14 px、水平内边距 8 px、圆角 4 px。选择器统一右侧箭头；尺寸只随界面字号缩放。原生主题继续负责背景、边框、焦点、禁用与展开状态，多行文本保留独立编辑高度。
- 外观预览默认关闭。全局搜索定位分组；页内提供分组、已修改项筛选与恢复本组。工作区排序/折叠移到通用。

## 行为保留

不改变配置格式、引擎能力或保存边界。浏览服务不改变默认值；服务编辑仍有保存/取消与引用保护；MCP 配置、连接检测和当前会话加载状态分别表述。只读下仍可浏览、搜索和检测。关闭设置恢复原工作台面板、宽度、草稿和焦点。

## 原生验证入口

`src/app/settings/workspace_tests.rs` 中的忽略测试 `settings_workspace_native_snapshots` 调用实际 GPUI 渲染器输出页面图像。使用合成服务和扩展数据、无操作的持久化写入器；外部配置读取通过 `scripts/screenshot-demo.py` 的子进程隔离环境限制在演示目录。

覆盖 1200×800、850×500、640×720、1600×900，以及 1200×800 的 20 px 浅色界面；除了 11 页首屏，还覆盖引擎/服务详情、外观预览、CLI 高级配置、会话命名/Git 生成配置、服务新建/编辑、MCP 新建和 Skill 编辑，共 105 张。输出位于 `target/settings-native-snapshots/`。

原生看图记录见 [polish-log.md](polish-log.md)，与已有 HTML 原型评审区分。最终图集为 `target/settings-native-snapshots/index.html`，全部页面总览和前后对比保存在同目录。可运行预览入口为 `target/settings-native-preview/Preview-Settings-A.cmd`，使用独立演示配置和本轮构建的程序。

## 首次完整重构的验证结果

- `cargo fmt --all --check` 通过。
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings` 通过；日志 `target/settings-native-final-clippy.log`。
- 不启用测试特性的 `conduit`、`conduit-updater`、`conduit-launcher` 构建通过；日志 `target/settings-native-final-shipping.log`。
- 两种语言各 3630 条文本校验通过；GPUI 供应商依赖一致性检查通过。
- 完整核心检查首次执行时，主程序 1100 项通过、2 项失败、29 项忽略。失败来自工作区设置迁移后的旧搜索路由断言，以及窄窗引擎详情返回路径的旧假设，均已修正；库测试 62 项及 pet-core 7 项通过。
- 最新源码的全部设置回归与上述两个修正点再次执行：46 passed、0 failed、2 ignored。忽略项为需要真实桌面/GPU 的截图测试，其中本设置截图测试另行执行成功。日志 `target/settings-native-final-tests.log`。
- 原生渲染测试通过，70 张 PNG 的像素尺寸与目标视口一致；日志 `target/settings-native-snapshots.log`。共完成三轮看图复核。

系统桌面操控管道不可用，未完成系统级逐项手动点击验收。键盘、焦点、只读、默认值边界、未保存表单和切页行为由原生 GPUI 自动测试验证；截图使用实际 Windows GPU 渲染器。全量核心检查在修正后没有整体重复运行，已对改动影响范围和失败项定向复验。


## 控件一致性补修验证（2026-11-26）

- 11 页及相关编辑弹窗的输入、下拉、数字步进器使用共享样式；旧的无边框命名/Git 选择器已迁移。常规按钮仍保留各自的操作语义。
- `cargo fmt --all --check` 通过；日志 `target/settings-controls-fmt.log`。
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings` 通过；日志 `target/settings-controls-clippy.log`。
- `conduit`、`conduit-updater`、`conduit-launcher` 常规构建通过；日志 `target/settings-controls-shipping.log`。
- 设置、生成路由、模型选择、连接切换和工作区折叠定向回归：48 passed、0 failed、2 ignored；日志 `target/settings-controls-tests.log`。本轮未重跑完整核心套件。
- 设置原生截图测试另行通过，21 个页面/编辑状态 × 5 种视口字号组合，共 105 张。第三轮完整查看 11 页总览并复核关键控件、浅色大字号和窄窗弹窗。
- 最终截图时系统为 200% 显示缩放；原图保留物理像素。宽窗受系统工作区约束，物理尺寸详见图集内 `capture-dimensions.json`；前两轮保留完整 1600×900 的 100% 缩放截图。比较图按相同逻辑视口归一，不修改原图。
- 已将常规构建复制为 `target/settings-native-preview/conduit-settings-controls.exe`，并更新 `Preview-Settings-A.cmd` 指向新程序。独立演示配置预览已启动；未关闭正在运行的正式客户端。
