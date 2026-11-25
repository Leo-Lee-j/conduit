"""Regenerate the checked-in sharing card: python site/scripts/generate-social.py.

Requires Pillow. Uses the existing app screenshot and logo; no generated UI.
Run from any directory. Pass a font path for other operating systems.
"""

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--font", default="C:/Windows/Fonts/segoeui.ttf")
args = parser.parse_args()
repo = Path(__file__).resolve().parents[2]
canvas = Image.new("RGB", (1200, 630), "#f6f5f0")
draw = ImageDraw.Draw(canvas)
font = lambda size: ImageFont.truetype(args.font, size)

logo = Image.open(repo / "assets/app-icons/128x128.png").convert("RGBA")
logo.thumbnail((40, 40), Image.Resampling.LANCZOS)
canvas.paste(logo, (56, 45), logo)
draw.text((110, 42), "Conduit", fill="#202b28", font=font(30))
draw.line((56, 112, 1144, 112), fill="#d9ddd4", width=2)
draw.text((56, 150), "Your agents.", fill="#202b28", font=font(60))
draw.text((56, 224), "One workspace.", fill="#245b4b", font=font(60))
draw.text((56, 334), "Conversations, code, Git,", fill="#5c6661", font=font(24))
draw.text((56, 370), "and a terminal. Together.", fill="#5c6661", font=font(24))
draw.text((56, 535), "Claude Code / Codex / OpenCode / Qoder CLI", fill="#245b4b", font=font(18))

shot = Image.open(repo / "assets/screenshots/workspace.png").convert("RGB")
shot = shot.resize((670, 415), Image.Resampling.LANCZOS)
draw.rounded_rectangle((566, 145, 1252, 578), radius=12, fill="#dee7df")
canvas.paste(shot, (574, 154))

output = repo / "site/public/social.png"
output.parent.mkdir(parents=True, exist_ok=True)
canvas.save(output, optimize=True)
print(output)
