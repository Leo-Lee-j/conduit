# Engine and model marks

Embedded locally with `include_bytes!`; rendering never downloads assets.

## Engine marks

- `claude.svg`: Claude symbol from the official `https://claude.com/favicon.svg`, retrieved 2026-10-23. Original paths, viewBox and color retained.
- `openai.svg`: OpenAI symbol from the first inline SVG in the official `https://developers.openai.com/codex/`, retrieved 2026-10-23. Original paths and viewBox retained; presentation-only HTML attributes removed. Used for the Codex CLI engine.

## Model marks

`grok.svg`, `kimi.svg`, `deepseek.svg`, `qwen.svg`, `zhipu.svg`, `minimax.svg`, `gemini.svg`, `doubao.svg`, `hunyuan.svg`: vendor symbols from `@lobehub/icons-static-svg` 1.98.0 (`https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.98.0/icons/<name>.svg`), retrieved 2026-10-25. Original paths and viewBox retained; `<title>` and presentation-only `style` / `role` attributes removed.

`models::brand_of` maps a model ID to one of these marks. Marks are `currentColor` and inherit the surrounding text color, except the Claude symbol, which keeps its published accent. Unmatched vendors — including proxy-specific IDs — fall back to their engine mark, so a row never shows an unrelated brand.

These marks identify their respective engines and models. They remain the property of their respective owners.
