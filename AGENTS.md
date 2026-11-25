<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->

## 发布版本与更新说明

当用户要求 release、发布版本、打版本标签或准备发布时，先阅读
[发布文档](docs/releasing.md)和[更新说明生成提示词](docs/release-notes-prompt.md)。

- 由执行发布任务的 AI 根据本次实际发布的变更生成 `release-notes/vX.Y.Z.md`；
  不需要用户手写，也不在 GitHub Actions 或客户端调用模型 API。
- 使用 [Markdown 模板](release-notes/TEMPLATE.md)，面向用户描述功能与体验变化，
  合并同类变更，省略纯内部调整，不直接粘贴提交历史。
- 文件名、首行标题和 Cargo 版本必须对应目标标签。生成后运行
  `./scripts/prepare-release-notes.ps1 -Tag vX.Y.Z`，并按发布文档执行相应验证。
- 在创建标签前，将更新说明与实际要发布的版本变更一同纳入发布提交；仅把文件留在
  工作区或未打标签的后续提交中，发布工作流无法读取。
- 保留已有的人工编辑；已发布版本的更新说明不可改写。用户只要求准备发布时，
  完成文件和本地验证即可；创建标签、推送和发布遵循用户本次任务的授权范围。
