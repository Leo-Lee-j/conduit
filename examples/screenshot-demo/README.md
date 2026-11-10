# Conduit 宣传截图示例

一套可以反复打开的演示工作空间，配有可交互的 Orbit 项目看板。项目、成员、对话与工具记录均为专门编写的虚构示例，不是模型运行记录。

## 启动

在仓库根目录运行，使用 Python 3.10+、Git 与已构建的 Conduit：

```powershell
python scripts/screenshot-demo.py
```

脚本自动选择本地 `target/debug` 或 `target/release` 中修改时间较新的可执行文件，也可指定已安装版本：

```powershell
python scripts/screenshot-demo.py --exe 'C:\path\to\Conduit.exe'
```

为了让截图中的路径更简洁，可以使用一个新的中性目录：

```powershell
python scripts/screenshot-demo.py --root D:\Conduit-Demo
```

首次生成三个工作空间、八组对话与三个本地 Git 仓库；再次启动保留演示里的编辑与界面设置。默认目录是 `target/screenshot-demo`。已有非演示目录会被拒绝，脚本不会重置或删除数据。

生成后，也可以双击演示根目录下的 `Start-Conduit-Demo.cmd` 再次启动。启动器引用当前仓库脚本，移动仓库后请从新位置重新运行上述命令。

```powershell
# 生成一份新的带时间戳的演示，保留旧演示
python scripts/screenshot-demo.py --fresh

# 打开演示的 Git 面板（先关闭同一演示实例）
python scripts/screenshot-demo.py --view git

# 只生成文件
python scripts/screenshot-demo.py --prepare-only
```

## 内容

| 工作空间 | 演示素材 |
| --- | --- |
| orbit-web | 可交互网页，四种引擎的示例会话，代码与未提交 Git 改动 |
| orbit-api | 离线 API 草案、JSON 示例与接口设计会话 |
| orbit-design | 视觉规范、产品文案与设计会话 |

直接打开生成目录下的 `workspaces/orbit-web/index.html` 即可体验示例网页。支持搜索、负责人筛选、新建和修改任务、深浅主题、里程碑与文档入口，无需依赖或网络。原始网页也可从本目录的 `orbit-web/index.html` 打开。

## 数据隔离

- 演示进程使用自己的 `CONDUIT_DATA_DIR`、空白用户目录和应用配置目录。
- 子进程环境只保留必要的系统信息，不继承账号密钥、CLI 配置路径、CC Switch 数据、Git 身份或 SSH 代理。
- 引擎路径指向演示目录中不存在的可执行文件，避免发送时回退到真实 CLI；预置会话可以浏览，不能直接续聊。
- Git 仓库使用 `Orbit Demo <demo@example.invalid>`，无远端、无真实提交历史，不执行用户 Git hooks。
- 默认关闭演示实例的自动更新、自动命名和性能浮层。

这是独立演示配置，不是操作系统沙箱。主动添加真实目录或重新配置引擎后，内容就不再限定为这套演示数据。截图选择演示窗口本身；真实应用窗口、任务栏和浏览器其他标签页属于演示配置之外。

## 建议截图

1. **主界面**：选中「完善项目看板与任务筛选」，展示左侧工作空间、示例对话、文件改动和输入框。
2. **代码审阅**：打开 Git 面板，选择 `src/board.js`，展示任务搜索的差异。
3. **文档预览**：打开 `docs/design-notes.md`，展示 Markdown 标题和表格。
4. **项目成果**：打开 Orbit 网页，展示完整看板；再点击任务卡片拍摄详情。
5. **外观**：在演示实例设置中切换深浅主题，生成不同风格的宣传素材。

对外说明这些是示例项目与演示数据，避免将预置会话误标为真实模型性能或运行结果。
