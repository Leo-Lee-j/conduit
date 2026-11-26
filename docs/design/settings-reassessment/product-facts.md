# 事实与证据

- 当前工作区 Cargo.toml：Conduit 0.3.22；README.zh-CN.md：Windows 原生 AI 编程工作台。
- `src/app/settings/navigation.rs`：四域、11 个可见导航项、12 个兼容路由，224px 基础导航宽，低于导航加 480px 内容宽时转分类下拉。
- `src/app/settings.rs`：页面标题隐藏，分组采用带边框圆角容器；说明 12px；表单正文由组件生成。
- `src/app/settings/workspace.rs`：内容顶部固定全局搜索、底部自动保存状态。
- `src/app/settings/appearance.rs`：主题、工作区行为、界面/聊天/代码字体，以及背景等共处外观页。
- `docs/design/settings-ui-native.md` 与 `settings-ui-prototype.md`：上一轮的实现目标与行为边界。
- 已目视查看的原生历史截图：`target/settings-redesign-continuation-native/out/grouped-settings-appearance.png`，文件时间 2026-11-23。它是历史证据，不能当作本次即时截图。
- 已目视查看的旧 HTML 原型截图：`docs/design/settings-ui/previews/appearance-normal-1200.png`。它是设计原型，不能当作原生应用截图。
- 2026-11-26 桌面截图助手两次返回 native pipe unavailable，不能获得本次原生即时画面。评审根据当前源码和以上明确标注的截图，不能断言今天运行窗口的像素表现。
- 外部参考：VS Code 官方 Settings 文档已联网返回 200 并获取官方 Settings 截图，ref_board 与 ref_card 输出在忽略目录 target/settings-reassessment-refs/。Windows Terminal 官方 Appearance 页面也已读取，但没有取得合适界面图，未作为入选参照。web 工具本轮返回空内容，改用直接读取官方页面核验；不引用无法读取的 Zed 示例。

页面中的状态均为设计示例，不代表当前用户的设置与连接情况。
