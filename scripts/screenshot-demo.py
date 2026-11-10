"""Prepare and launch a fictional Conduit workspace with an isolated profile.

Only the Python standard library and Git are required. Existing demo directories
are reused without reseeding; --fresh creates another directory without deletion.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


REPO = Path(__file__).resolve().parents[1]
ASSETS = REPO / "examples" / "screenshot-demo" / "orbit-web"
MARKER = ".conduit-screenshot-demo.json"
ENGINES = ("Claude", "Codex", "OpenCode", "Qoder")


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8", newline="\n")


def create_launcher(root: Path, app: Path | None) -> None:
    """A reusable, double-clickable entry; never edit the caller's environment."""
    if os.name != "nt":
        return
    launcher = root / "Start-Conduit-Demo.cmd"
    if launcher.exists():
        return
    # Percent expansion still happens inside cmd quotes. Delayed expansion is
    # disabled so exclamation marks in an installation path remain literal.
    def quote(value: Path) -> str:
        return '"' + str(value).replace("%", "%%") + '"'
    command = f"{quote(Path(sys.executable))} {quote(Path(__file__).resolve())} --root {quote(root)}"
    if app is not None:
        command += f" --exe {quote(app)}"
    write(launcher, f"@echo off\nchcp 65001 >nul\nsetlocal DisableDelayedExpansion\n{command}\nif errorlevel 1 pause\n")


def isolated_environment(root: Path, git_executable: str) -> dict[str, str]:
    # Construct a child-only environment. Never inherit HOME, CODEX_HOME,
    # CLAUDE_CONFIG_DIR, API keys, endpoints, SSH agents, or Git config overrides.
    system_keys = {
        "SYSTEMROOT", "WINDIR", "COMSPEC", "SYSTEMDRIVE", "OS", "PATHEXT",
        "PROGRAMFILES", "PROGRAMFILES(X86)", "PROGRAMW6432",
        "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_IDENTIFIER", "PROCESSOR_LEVEL", "PROCESSOR_REVISION",
    }
    env = {key: value for key, value in os.environ.items() if key.upper() in system_keys}
    profile = root / "profile"
    env.update({
        "CONDUIT_DATA_DIR": str(root / "data"),
        "USERPROFILE": str(profile),
        "APPDATA": str(profile / "AppData" / "Roaming"),
        "LOCALAPPDATA": str(profile / "AppData" / "Local"),
        "XDG_CONFIG_HOME": str(profile / ".config"),
        "XDG_DATA_HOME": str(profile / ".local" / "share"),
        "TEMP": str(root / "temp"), "TMP": str(root / "temp"),
        "USERNAME": "demo", "USERDOMAIN": "ORBIT", "COMPUTERNAME": "DEMO-PC",
        "HOMEDRIVE": profile.drive, "HOMEPATH": str(profile)[len(profile.drive):],
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": str(profile / ".gitconfig"),
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_AUTHOR_NAME": "Orbit Demo", "GIT_AUTHOR_EMAIL": "demo@example.invalid",
        "GIT_COMMITTER_NAME": "Orbit Demo", "GIT_COMMITTER_EMAIL": "demo@example.invalid",
    })
    system = Path(os.environ.get("SystemRoot", "C:/Windows"))
    paths = [system / "System32", system, system / "System32/WindowsPowerShell/v1.0", Path(git_executable).parent]
    env["PATH"] = os.pathsep.join(map(str, paths))
    return env


def git(workspace: Path, env: dict[str, str], executable: str, *args: str) -> str:
    result = subprocess.run(
        [executable, "-C", str(workspace), *args], env=env, check=True,
        text=True, encoding="utf-8", capture_output=True,
        creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
    )
    return result.stdout.strip()


def item(key: str, kind: str, text: str, timestamp: int, **extra: object) -> dict:
    return {"key": key, "kind": kind, "text": text, "created_at": timestamp, **extra}


def conversation(session_id: int, provider: str, title: str, prompt: str, reply: str,
                 timestamp: int, tools: list[dict] | None = None, pinned: bool = False) -> dict:
    return {
        "id": session_id, "provider": provider, "title": title,
        "created_at": timestamp, "exit_code": 0, "pinned": pinned,
        "messages": [item(f"demo-{session_id}-user", "User", prompt, timestamp),
                     *(tools or []),
                     item(f"demo-{session_id}-reply", "Assistant", reply, timestamp + 60)],
    }


def make_state(root: Path, timestamp: int, diff_stat: str) -> dict:
    web = root / "workspaces" / "orbit-web"
    activities = [
        item("demo-101-read", "Tool", "Read", timestamp + 5,
             input=json.dumps({"file_path": "README.md"}),
             detail=(web / "README.md").read_text(encoding="utf-8"), status="已完成"),
        item("demo-101-edit", "Tool", "Edit", timestamp + 20,
             input=json.dumps({"file_path": "src/board.js",
                               "old_string": "    task.title.toLocaleLowerCase().includes(query) &&",
                               "new_string": "    `${task.title} ${task.description} ORB-${task.id}`.toLocaleLowerCase().includes(query) &&"}, ensure_ascii=False),
             detail="支持同时搜索任务名称、说明与编号。", status="已完成"),
        item("demo-101-git", "Tool", "Bash", timestamp + 35,
             input=json.dumps({"command": "git diff --stat"}), detail=diff_stat, status="已完成"),
    ]
    primary = conversation(101, "Claude", "完善项目看板与任务筛选",
        "帮我完善 Orbit 的项目看板：支持关键词搜索和负责人筛选，补充任务详情，再整理一下发布前的检查清单。整体保持清爽，适合小团队日常协作。",
        "Orbit 的核心体验已经串起来了。\n\n"
        "### 现在可以做什么\n\n"
        "- **快速找到任务**：任务名称、说明和编号都可以搜索，也能叠加负责人筛选。\n"
        "- **直接推进工作**：点击卡片查看详情，修改负责人和状态；新任务会出现在对应列。\n"
        "- **随时切换主题**：松绿色搭配中性色，深浅模式保留同样的信息层次。\n\n"
        "### 文件与预览\n\n"
        "| 文件 | 内容 |\n| --- | --- |\n"
        "| `src/board.js` | 任务状态、组合筛选与本地保存 |\n"
        "| `src/styles.css` | 看板布局、主题与移动端适配 |\n"
        "| `docs/release-checklist.md` | 发布前的检查清单 |\n\n"
        "直接打开 `index.html` 就能体验。当前变更已留在 Git 面板，方便逐个审阅。",
        timestamp, activities, pinned=True)
    web_sessions = [primary,
        conversation(102, "Codex", "检查空状态与键盘操作",
                     "检查任务筛选没有结果时的提示，以及新建任务弹窗的键盘操作。",
                     "已补充空结果提示，并保留可见的键盘焦点。\n\n- 搜索与成员条件同时生效。\n- 弹窗打开后聚焦任务名称。\n- Escape 关闭弹窗，Tab 在弹窗内移动。\n- 纯空格名称会提示重新填写。\n\n可在 `src/board.js` 查看交互逻辑。", timestamp - 1800),
        conversation(103, "OpenCode", "调整移动端看板布局",
                     "让 Orbit 在手机上也可以清楚地浏览任务。",
                     "小屏幕采用单列任务分组；中等宽度采用两列，桌面保留四列。\n\n搜索输入框在手机上独占一行，筛选和新建按钮并排显示。任务说明允许自然换行，操作保持可见。", timestamp - 3600),
        conversation(104, "Qoder", "整理项目介绍与交付说明",
                     "整理一份简洁的 README，让第一次打开项目的人知道怎么使用。",
                     "README 已包含运行方式、主要交互和项目结构。\n\n这个示例无需依赖安装，打开 `index.html` 即可。所有成员与任务均为虚构内容，适合用于产品演示。", timestamp - 5400),
    ]
    api_sessions = [
        conversation(201, "Codex", "设计任务接口与分页格式",
                     "给 Orbit 的任务列表拟一份简单的接口约定，先用本地示例数据。",
                     "接口草案已整理到 `docs/api-contract.md`。\n\n```json\n{\n  \"items\": [{ \"id\": \"ORB-24\", \"title\": \"实现任务筛选与搜索\" }],\n  \"next_cursor\": null\n}\n```\n\n列表支持 `status`、`owner` 和 `query`。这只是离线接口草案，当前网页不依赖后端服务。", timestamp - 7200),
        conversation(202, "Claude", "规划输入校验与错误提示",
                     "列出创建任务时需要处理的边界情况。",
                     "建议校验任务名称、状态和负责人，并返回清楚的字段级错误。\n\n| 字段 | 约束 |\n| --- | --- |\n| title | 去除首尾空格，1–80 字符 |\n| description | 最多 400 字符 |\n| status | 使用约定的四种状态 |\n\n本地样例保存在 `fixtures/tasks.json`。", timestamp - 10800),
    ]
    design_sessions = [
        conversation(301, "Claude", "确定 Orbit 的视觉方向",
                     "给小团队项目工具确定一套克制、清晰的视觉方向。",
                     "以松绿色作为行动色，温润灰白作为背景，保留充分的阅读空间。\n\n- 用布局和字重建立层次。\n- 状态同时使用文字和颜色。\n- 数字使用等宽数字，便于对齐。\n- 中文使用系统字体，保证离线可用。\n\n设计原则已整理在 `brand.md`。", timestamp - 14400, pinned=True),
        conversation(302, "OpenCode", "准备产品发布文案",
                     "为 Orbit 写一组简洁的首页文案。",
                     "**把想法变成下一步。**\n\n清晰的优先级，专注的协作，让好作品按时发生。\n\n**少一点切换，多一点创造。**\n\n文案围绕实际工作场景展开，避免夸大功能和编造用户数据。", timestamp - 18000),
    ]
    version = re.search(r'^version = "([^"]+)"', (REPO / "Cargo.toml").read_text(encoding="utf-8"), re.M).group(1)
    return {
        "version": 2,
        "workspaces": [{"path": str(root / "workspaces" / name), "sessions": sessions}
                       for name, sessions in [("orbit-web", web_sessions), ("orbit-api", api_sessions), ("orbit-design", design_sessions)]],
        "ui": {"language": "zh-CN", "appearance": "dark", "performance_monitor": False,
               "auto_rename": False, "collapse_reasoning": True, "sessions_per_workspace": 5,
               "ui_font_size": 16, "chat_font_size": 15, "code_font_size": 13,
               "code_line_numbers": True, "recent_sessions": [101, 201, 301]},
        "updates": {"auto_check": False, "auto_download": False, "auto_install": False, "last_seen_version": version},
        # Invalid explicit executables prevent fallback to a real authenticated CLI.
        "cli": {"engines": {engine: {"path": str(root / "preview-only" / f"{engine.lower()}.exe")}
                            for engine in ENGINES}},
        "providers": [],
    }


def prepare(root: Path, executable: str) -> bool:
    marker = root / MARKER
    if root.exists() and any(root.iterdir()):
        if not marker.is_file() or json.loads(marker.read_text(encoding="utf-8")).get("kind") != "conduit-screenshot-demo":
            raise ValueError(f"Refusing to overwrite an existing directory: {root}. Choose a new --root or use --fresh.")
        if not (root / "data/state.json").is_file():
            raise ValueError("Demo state is missing. Use --fresh to create a new demo.")
        return False
    root.mkdir(parents=True, exist_ok=True)
    env = isolated_environment(root, executable)
    for key in ("USERPROFILE", "APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "TEMP"):
        Path(env[key]).mkdir(parents=True, exist_ok=True)
    write(root / "profile/.gitconfig", "[user]\n\tname = Orbit Demo\n\temail = demo@example.invalid\n[commit]\n\tgpgsign = false\n[tag]\n\tgpgsign = false\n")
    web = root / "workspaces/orbit-web"
    shutil.copytree(ASSETS, web)
    write(web / ".gitignore", "*.log\n.DS_Store\n")
    api = root / "workspaces/orbit-api"
    write(api / "README.md", "# Orbit API\n\n离线接口草案，用于演示文件浏览与文档预览。没有运行中的后端服务。\n\n参见 `docs/api-contract.md` 与 `fixtures/tasks.json`。\n")
    write(api / "docs/api-contract.md", "# 任务接口约定\n\n所有数据均为虚构示例。\n\n## GET /tasks\n\n支持 query、owner、status 与 cursor 参数。\n\n```json\n{\n  \"items\": [{\"id\": \"ORB-24\", \"title\": \"实现任务筛选与搜索\"}],\n  \"next_cursor\": null\n}\n```\n\n## POST /tasks\n\n任务名称 1–80 字符，说明最多 400 字符；状态为 backlog、progress、review 或 done。\n")
    write(api / "fixtures/tasks.json", json.dumps({"items": [{"id": "ORB-24", "title": "实现任务筛选与搜索", "owner": "陈一", "status": "progress"}], "next_cursor": None}, ensure_ascii=False, indent=2) + "\n")
    design = root / "workspaces/orbit-design"
    write(design / "README.md", "# Orbit Design\n\n虚构产品的视觉规范与文案素材。\n\n- `brand.md`：视觉方向\n- `copy.md`：产品文案\n- `tokens.json`：基础颜色\n")
    write(design / "brand.md", "# 让工作清晰，让界面安静\n\n松绿色用于关键操作，中性色用于大面积阅读。\n\n| 角色 | 颜色 |\n| --- | --- |\n| 行动色 | #176b58 |\n| 页面 | #f8f9f6 |\n| 正文 | #202e28 |\n\n用字号、间距和字重区分信息层级。所有状态都有文字标签。\n")
    write(design / "copy.md", "# 产品文案\n\n把想法变成下一步。\n\n清晰的优先级，专注的协作，让好作品按时发生。\n\n少一点切换，多一点创造。\n")
    write(design / "tokens.json", json.dumps({"accent": "#176b58", "background": "#f8f9f6", "text": "#202e28"}, indent=2) + "\n")
    board_path, style_path = web / "src/board.js", web / "src/styles.css"
    board, style = board_path.read_text(encoding="utf-8"), style_path.read_text(encoding="utf-8")
    write(board_path, board.replace("`${task.title} ${task.description} ORB-${task.id}`.toLocaleLowerCase()", "task.title.toLocaleLowerCase()"))
    write(style_path, style.replace("grid-template-columns:repeat(4,minmax(0,1fr))", "grid-template-columns:repeat(3,minmax(0,1fr))"))
    for workspace in (web, api, design):
        git(workspace, env, executable, "init", "-q", "-b", "main", "--template=")
        git(workspace, env, executable, "config", "core.autocrlf", "false")
        git(workspace, env, executable, "config", "core.hooksPath", str(root / "empty-hooks"))
        git(workspace, env, executable, "add", ".")
        git(workspace, env, executable, "commit", "-q", "-m", "chore: create fictional Orbit demo")
    git(web, env, executable, "switch", "-q", "-c", "feat/task-filters")
    write(board_path, board)
    write(style_path, style)
    write(web / "docs/preview-notes.md", "# 本轮交付\n\n- 搜索任务名称、说明与编号。\n- 按负责人组合筛选。\n- 桌面采用四列布局。\n\n打开 index.html 体验；在 Git 面板逐项审阅。\n")
    diff_stat = git(web, env, executable, "diff", "--stat")
    write(root / "data/state.json", json.dumps(make_state(root, int(time.time()) - 120, diff_stat), ensure_ascii=False, indent=2) + "\n")
    write(marker, json.dumps({"kind": "conduit-screenshot-demo", "version": 1}, indent=2) + "\n")
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=REPO / "target/screenshot-demo")
    parser.add_argument("--exe", type=Path, help="Conduit executable; defaults to the newest local debug/release build")
    parser.add_argument("--prepare-only", action="store_true", help="Generate demo files without launching Conduit")
    parser.add_argument("--fresh", action="store_true", help="Create a new timestamped demo; preserve the previous one")
    parser.add_argument("--view", choices=("chat", "git", "settings"), default="chat")
    args = parser.parse_args()
    root = args.root.resolve()
    if args.fresh:
        root = root.with_name(f"{root.name}-{time.strftime('%Y%m%d-%H%M%S')}-{time.time_ns() % 1000000:06}")
    executable = shutil.which("git")
    if executable is None:
        parser.error("Git is required to create the sample repositories.")
    app = None
    if not args.prepare_only:
        if os.name != "nt":
            parser.error("Conduit runs on Windows. Use --prepare-only to generate files elsewhere.")
        builds = [REPO / "target/debug/conduit.exe", REPO / "target/release/conduit.exe"]
        app = args.exe.resolve() if args.exe else max((path for path in builds if path.is_file()), key=lambda path: path.stat().st_mtime, default=None)
        if app is None or not app.is_file():
            parser.error("No Conduit executable found. Pass --exe or build Conduit first.")
    try:
        created = prepare(root, executable)
        create_launcher(root, app)
        print(f"{'Created' if created else 'Reusing'} screenshot demo: {root}")
        print(f"Sample app: {root / 'workspaces/orbit-web/index.html'}")
        if not args.prepare_only:
            arguments = [str(app)] + ([] if args.view == "chat" else [f"--{args.view}"])
            log_path = root / "conduit-demo.log"
            with log_path.open("ab") as log:
                process = subprocess.Popen(arguments, cwd=root / "workspaces/orbit-web",
                    env=isolated_environment(root, executable), stdin=subprocess.DEVNULL,
                    stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                code = process.wait(timeout=2)
                raise RuntimeError(f"Conduit exited during startup ({code}); see {log_path}. This demo may already be open.")
            except subprocess.TimeoutExpired:
                print(f"Conduit demo started (PID {process.pid}). Prepared conversations are fictional; live AI engines are unavailable in this profile.")
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Demo setup failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stderr, file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
