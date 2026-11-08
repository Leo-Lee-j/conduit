"""Refresh pinned vendor SVGs; no network access is needed by the application."""

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import re
from urllib.request import urlopen

VERSION = "1.98.0"
ICONS = {
    name: f"{name}-color"
    for name in (
        "claude", "claudecode", "codex", "qoder", "deepseek", "qwen", "kimi", "zhipu", "minimax", "doubao",
        "hunyuan", "gemini", "stepfun", "mistral", "meta", "cohere", "gemma",
        "nvidia", "perplexity", "baichuan", "yi", "spark", "nova", "wenxin",
    )
}
ICONS.update({"ai21": "ai21-brand-color", "opencode": "opencode", "openai": "openai", "grok": "grok"})
ROOT = Path(__file__).resolve().parents[1] / "assets" / "icons"


def download(item):
    name, source = item
    url = f"https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@{VERSION}/icons/{source}.svg"
    with urlopen(url, timeout=30) as response:
        svg = response.read().decode("utf-8")
    if "<svg" not in svg:
        raise ValueError(f"Invalid SVG: {url}")
    # Keep original paths, fills and gradients. Presentation attributes only.
    svg = re.sub(r"<title>.*?</title>", "", svg)
    svg = re.sub(r' (?:style|role)="[^"]*"', "", svg)
    (ROOT / f"{name}.svg").write_text(svg + "\n", encoding="utf-8")
    return name


if __name__ == "__main__":
    with ThreadPoolExecutor(max_workers=6) as pool:
        print("Updated: " + ", ".join(pool.map(download, ICONS.items())))
