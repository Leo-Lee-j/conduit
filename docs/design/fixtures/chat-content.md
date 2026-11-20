# 聊天正文验收样例

用 Conduit 的 Markdown 预览打开此文件，切换明暗主题、窗口宽度和用户字号，检查以下内容。

## 文件链接与选择

普通文件链接：[typography.rs](src/app/typography.rs:1)。引用式链接：[聊天样式入口][typography]。带行内代码的链接：[`src/app/typography.rs`](src/app/typography.rs:1)。强调链接：**[样式实现](src/app/typography.rs:1)**。

长标签：[src/app/typography/中文目录/用于检查窄窗口换行及跨行部分选择的长文件名.rs](src/app/typography.rs:1)。选中标签中的一部分后复制，剪贴板应只包含所选文字；点击应打开对应文件。

文件链接前应有一个随字号缩放的文件图标，跨行只出现一次。图标与标签首字符应在同一行；点击图标或文字均打开原目标，从图标开始拖选不应跳转，复制不含图标或额外空格。

相邻链接：[样式文件](src/app/typography.rs:1)[链接解析](src/app/file_links.rs:1)。混合标签：[样式 **重点** 与 `code`](src/app/typography.rs:1)。每个链接只出现一个图标；以下外部链接不显示文件图标：[外部文档](https://example.com/guide)。

## 引用与列表

> 引用中的正文保持可选择，**重点文字**和 `inline_code` 保留自己的样式。
>
> 3. 保留有序列表的起始编号。
>    - [x] 已完成的子任务。
>    - [ ] 待处理的子任务，长中文应自然换行并对齐正文。
> 4. 下一项。
>
>    这一续段应与列表正文对齐。

## 表格

| 文件 | 用途 | 状态 |
| --- | --- | --- |
| [typography.rs](src/app/typography.rs:1) | 正文、引用、列表、表格和链接的统一样式入口 | 可打开 |
| `numbered_code.rs` | 代码块独立顶栏、选择和复制 | 可选择 |
| 中文长名称与英文字母 mixed_text | 缩小窗口检查换行、列宽和搜索定位 | 可搜索 |

首列正文应比其他列更醒目，表头保持独立样式。搜索“中文长名称”并核对高亮位置。

## 代码与图表

```rust
fn main() {
    println!("中文代码复制");
}
```

```
没有语言标记的代码，顶栏应显示 text。
```

```mermaid
flowchart LR
    A[任务] --> B[执行] --> C[结果]
```

开关代码行号并分别复制两个代码块；复制内容不应包含顶栏、行号或相邻块，Mermaid 仍应渲染为图表。

[typography]: src/app/typography.rs:1
