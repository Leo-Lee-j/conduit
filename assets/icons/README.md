# Engine and model marks

All SVGs are embedded with `include_bytes!`; application rendering works offline.
Asset research/refresh date: **2026-10-28**.

## Sources

- All SVGs come from the LobeHub AI/model icon library: `https://lobehub.com/zh/icons?type=color`.
- Pinned distribution: **`@lobehub/icons-static-svg` 1.98.0**, refreshed 2026-10-28. Source repository: `https://github.com/lobehub/lobe-icons`. The upstream MIT notice is included in `LICENSE.lobe-icons`; trademarks remain the property of their respective owners.
- `claudecode.svg` is the Claude Code CLI engine mark; `claude.svg` is the Claude model mark. Codex likewise uses its own engine mark while GPT models use `openai.svg`. No brand marks are drawn or redesigned locally.

The reproducible source mapping is in `scripts/sync-brand-icons.py`. It downloads
`https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.98.0/icons/<source>.svg`.
`ai21.svg` uses `ai21-brand-color`; `opencode.svg`, `openai.svg` and `grok.svg` use
their corresponding monochrome names because this pinned library has no color variant; the rest use
`<name>-color`. Only title/style/role presentation attributes are stripped.
Original paths, proportions, fills and gradients are retained.

## Coverage

| Role | Embedded marks |
| --- | --- |
| Implemented CLI engines | Claude Code, Codex, OpenCode |
| Model families | Claude, GPT / OpenAI, Grok, DeepSeek, Qwen / QwQ, Kimi / Moonshot, GLM / Zhipu, MiniMax, Doubao / Seed, Hunyuan, Gemini, Step / Stepfun, Mistral / Mixtral / Codestral / Devstral / Magistral, Llama / Meta, Command / Cohere, Gemma, Nemotron / NVIDIA, Sonar / Perplexity, Jamba / AI21, Baichuan, Yi / 01.AI, Spark, Nova / Amazon, ERNIE / Wenxin |

Brand coverage does not imply a new runtime integration. Only the three
implemented engines are offered as CLI engines.

## Rendering and model recognition

Use GPUI `ImageFormat::Svg` / `img` for full-color rendering. The `Icon` / `svg`
path paints a single-color alpha mask and discards SVG fills and gradients.
Images are shared and cached for both themes. At cache construction, `1em`
intrinsic dimensions are resolved to 64 px for crisp high-DPI rendering, and
`currentColor` becomes the theme's foreground. Fixed colors are preserved.
OpenAI, OpenCode and Grok publish monochrome marks, so those retain their
authentic monochrome appearance with light/dark foreground adaptation.

`models::brand_of` recognizes model names within routed/provider-prefixed IDs
using token boundaries, including short families such as `step`, `yi`, `o1`,
`o3` and `o4`. Unknown IDs fall back to the selected engine mark. Icons are
shared by chat controls, replies, session lists and settings.

Verification includes rasterizing every embedded SVG in both themes, checking
for visible pixels and preserved brand colors, and checking both red and blue
regions of Gemini's gradient.
