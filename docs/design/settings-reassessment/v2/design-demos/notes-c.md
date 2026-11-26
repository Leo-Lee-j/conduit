# C 参数表

横向领域/分类，取消纵向侧栏。13 个外观参数以名称、当前值、作用范围横向对齐；5 个组；底部短预览同时显示界面、聊天和代码。

- `#search`：筛选名称/当前值/说明；Ctrl K 聚焦，Enter 聚焦首个结果，Esc 清除筛选。
- `[data-filter=chat]` 等分组筛选；`#modified` 仅看已修改；清空状态带恢复按钮。
- 字号 number：`#uiSize`、`#chatSize`、`#codeSize`；行高 `#lineHeight`。
- 字体 select：`#uiFont`、`#chatFont`、`#codeFont`。
- `.preview-ui`、`.preview-chat`、`.code-sample` 分别对应三个独立预览。
- `[data-reset=chat]` 等：分组重置。
- `#lineNumbers` / `#highlight` switch；`[data-mode=深色]` / `[data-mode=浅色]`。
- 主题选择只有真实的演示内置主题；浏览服务、引擎和会话页为明确的结构示例。
