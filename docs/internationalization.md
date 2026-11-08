# Interface languages

Conduit includes English and Simplified Chinese. Choose **Settings → General → Language → Application language** to switch immediately. The selection is saved on this device.

New installations start in English. Existing settings without a language preference continue in Simplified Chinese, including older files with no `ui` section. Unknown language identifiers fall back to English.

Changing language updates interface labels, settings search, placeholders, and built-in component messages. It keeps drafts, queued messages, running tasks, form values, and font preferences. Messages written by users or models, terminal output, paths, model identifiers, and custom prompts keep their original contents. New installations use an English default commit prompt; existing and edited prompts are preserved.

## Add a language

1. Copy `locales/en.json` to a new catalog such as `locales/fr.json`.
2. Translate the values. Keep the keys and `%{parameter}` names unchanged; parameters can be reordered. `_version` remains `1`.
3. Add `{ "id": "fr", "name": "Français" }` to `assets/languages.json`. The name is the language's own name, displayed in the selector.
4. Run `python scripts/check-i18n.py` and `cargo test --locked --lib i18n:: -- --test-threads=1`.
5. Check the interface at narrow window sizes and with large interface fonts. Check settings search, menus, dialogs, tool details, notification history, and both directions of language switching.

Catalogs and the language registry are embedded in the executable. Adding a language does not require a Rust enum, a new settings control, or runtime downloads. Built-in GPUI components use the same `rust-i18n` locale. A locale that is not supplied by GPUI also needs component translations; otherwise those component strings use their English fallback. Right-to-left layout requires additional layout work.

## Add or change text

Use a stable descriptive key, grouped by feature. Change translated values without renaming existing keys. Add each key to every catalog.

```rust
let label = conduit::i18n::text_static("language.title");
let message = conduit::i18n::format(
    "chat.reasoning_effort",
    &[("effort", selected_effort.to_string())],
);
```

Use `text_static` for render-time labels and `text` when an owned string is needed. Use named parameters instead of concatenating translated sentence fragments. Format numbers and durations before substitution; parameters are substituted once and never recursively interpreted. Count labels avoid English singular/plural assumptions, for example `Files: 1` and `Files: 2`.

`assets/i18n-sources.json` supports existing static descriptors and legacy application labels. It is a compatibility map, not the primary API for new text. Do not translate arbitrary user strings through this map. Internal states, permission decisions, configuration keys, protocol values, and resource IDs must remain independent of the displayed language.

Stored notification titles and fixed application notices can retain localization metadata. Diagnostic details and older dynamic messages keep the text recorded at the time; raw CLI diagnostics are not rewritten. `i18n::Message` provides key, parameters, and fallback text for structured presentation that must survive persistence.

Language changes must refresh retained presentation explicitly. Update an input's placeholder on the existing entity; do not recreate the input or set its value. Notify cached views and preserve selection, focus, scroll positions, and running operations. Font choices stay independent of locale; Windows text fallback supplies glyphs absent from the selected font.

## Tests

Catalog validation checks matching keys, named parameters, registry entries, and compatibility aliases. Application tests cover legacy settings, unknown locale fallback, switching while a task is active, draft preservation, and bilingual settings search.

Existing UI fixtures describe the original Chinese interface and use a scoped test locale. New language tests should use `crate::test_support::scoped("en")` or `scoped("zh-CN")`, restoring the previous locale when they finish. The locale is process-wide; keep the existing single-threaded test commands.
