//! Syntax highlight names for fenced code blocks drawn with line numbers.
//!
//! The base text view keeps its own highlighter private, so numbered blocks
//! run syntect themselves. Syntect names what it matched with TextMate scopes
//! (`keyword.control.rust`); they are mapped onto the highlight names the
//! component theme resolves to colors, which keeps the block following the
//! active light or dark theme.

use std::{collections::HashMap, ops::Range, sync::LazyLock};

use syntect::{
    parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

/// Loading the syntax set takes tens of milliseconds, so it happens once on
/// the first numbered block and is shared by every block after that.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// Highlight names for `code`, ordered and non-overlapping. Unknown or plain
/// text languages yield nothing.
pub(super) fn highlight_names(language: &str, code: &str) -> Vec<(Range<usize>, &'static str)> {
    let Some(syntax) = find_syntax(language) else {
        return Vec::new();
    };
    parse(syntax, code)
}

/// Infer file syntax without reading the file again or wrapping its contents in Markdown.
pub(super) fn highlight_file(
    path: &std::path::Path,
    code: &str,
) -> Vec<(Range<usize>, &'static str)> {
    let syntaxes = &*SYNTAXES;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let syntax = syntaxes
        .find_syntax_by_token(name)
        .or_else(|| {
            path.extension()
                .and_then(|e| e.to_str())
                .and_then(|e| find_syntax(e))
        })
        .or_else(|| syntaxes.find_syntax_by_first_line(code.lines().next().unwrap_or_default()));
    syntax.map(|s| parse(s, code)).unwrap_or_default()
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    let syntaxes = &*SYNTAXES;
    let canonical = canonical_language(language);
    let alias = match canonical.as_str() {
        "text" => return None,
        "csharp" => "cs",
        "make" => "Makefile",
        "markdown_inline" => "md",
        "embedded_template" | "ejs" => "erb",
        other => other,
    };
    [language, canonical.as_str(), alias]
        .into_iter()
        .find_map(|token| syntaxes.find_syntax_by_token(token))
        .filter(|syntax| syntax.name != "Plain Text")
}

/// The common short forms people write on a fence, spelled the way the
/// editing layer spells them.
fn canonical_language(name: &str) -> String {
    let name = name.to_lowercase();
    match name.as_str() {
        "plain" | "plaintext" => "text",
        "jsonc" => "json",
        "sh" => "bash",
        "c++" => "cpp",
        "cs" => "csharp",
        "scss" => "css",
        "ex" => "elixir",
        "js" => "javascript",
        "kt" | "kts" | "ktm" => "kotlin",
        "makefile" => "make",
        "md" | "mdx" => "markdown",
        "markdown-inline" => "markdown_inline",
        "php3" | "php4" | "php5" | "phtml" => "php",
        "protobuf" => "proto",
        "py" | "pyi" => "python",
        "rb" => "ruby",
        "rs" => "rust",
        "ts" => "typescript",
        "yml" => "yaml",
        other => other,
    }
    .to_string()
}

/// Runs syntect over `text` and keeps the spans that map to a highlight name.
fn parse(syntax: &SyntaxReference, text: &str) -> Vec<(Range<usize>, &'static str)> {
    let syntaxes = &*SYNTAXES;
    let mut parser = ParseState::new(syntax);
    let mut scopes = ScopeStack::new();
    let mut names = HashMap::<Scope, Option<&'static str>>::new();
    let mut highlights = Vec::new();
    let mut offset = 0;

    let mut push = |range: Range<usize>,
                    scopes: &ScopeStack,
                    highlights: &mut Vec<(Range<usize>, &'static str)>| {
        if range.is_empty() {
            return;
        }
        // The innermost scope that names something wins, so the quotes of a
        // string — `punctuation.definition.string` inside `string` — color as
        // the string rather than as punctuation.
        let name = scopes
            .scopes
            .iter()
            .rev()
            .find_map(|scope| *names.entry(*scope).or_insert_with(|| semantic_name(*scope)));
        if let Some(name) = name {
            match highlights.last_mut() {
                Some((last, last_name)) if last.end == range.start && *last_name == name => {
                    last.end = range.end;
                }
                _ => highlights.push((range, name)),
            }
        }
    };

    for line in LinesWithEndings::from(text) {
        // A line syntect cannot parse keeps the scopes it started with; the
        // next line starts over from them rather than losing the document.
        if let Ok(operations) = parser.parse_line(line, syntaxes) {
            let mut cursor = 0;
            for (index, operation) in operations {
                push(offset + cursor..offset + index, &scopes, &mut highlights);
                let _ = scopes.apply(&operation);
                cursor = index;
            }
            push(
                offset + cursor..offset + line.len(),
                &scopes,
                &mut highlights,
            );
        }
        offset += line.len();
    }
    highlights
}

/// The theme's highlight name for a TextMate scope, or `None` to let an
/// enclosing scope decide.
fn semantic_name(scope: Scope) -> Option<&'static str> {
    let scope = scope.build_string();
    let is = |prefix: &str| {
        scope == prefix
            || scope
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
    };

    let name = if is("punctuation.definition.string")
        || is("punctuation.definition.comment")
        || is("invalid")
        || is("meta")
        || is("source")
        || is("text")
    {
        // Delimiters belong to what they delimit; the rest are containers
        // with no color of their own.
        return None;
    } else if is("comment.block.documentation") || is("comment.line.documentation") {
        "comment.doc"
    } else if is("comment") {
        "comment"
    } else if is("constant.character.escape") {
        "string.escape"
    } else if is("string.regexp") {
        "string.regex"
    } else if is("string") {
        "string"
    } else if is("constant.numeric") {
        "number"
    } else if is("constant.language.boolean") {
        "boolean"
    } else if is("constant") || is("support.constant") || is("entity.name.constant") {
        "constant"
    } else if is("keyword.operator") {
        "operator"
    } else if is("keyword.control.import") || is("meta.preprocessor") {
        "preproc"
    } else if is("keyword") || is("storage.modifier") {
        "keyword"
    } else if is("storage.type") {
        // `fn`, `class` and `let` are declarations, `int` and `bool` are types.
        if [
            "class",
            "enum",
            "function",
            "impl",
            "interface",
            "module",
            "namespace",
            "struct",
            "trait",
            "type",
            "let",
            "var",
            "const",
        ]
        .iter()
        .any(|word| scope.contains(word))
        {
            "keyword"
        } else {
            "type"
        }
    } else if is("entity.name.function")
        || is("support.function")
        || is("variable.function")
        || is("meta.function-call")
    {
        "function"
    } else if is("entity.name.type")
        || is("entity.name.class")
        || is("entity.name.struct")
        || is("entity.name.enum")
        || is("entity.name.trait")
        || is("entity.name.interface")
        || is("entity.other.inherited-class")
        || is("support.type")
        || is("support.class")
    {
        "type"
    } else if is("entity.name.tag") {
        "tag"
    } else if is("entity.other.attribute-name") {
        "attribute"
    } else if is("entity.name.label") {
        "label"
    } else if is("entity.name.section") || is("markup.heading") {
        "title"
    } else if is("variable.language") {
        "variable.special"
    } else if is("variable.other.member")
        || is("variable.other.property")
        || is("support.variable.property")
        || is("meta.mapping.key")
    {
        "property"
    } else if is("variable") {
        "variable"
    } else if is("markup.bold") {
        "emphasis.strong"
    } else if is("markup.italic") {
        "emphasis"
    } else if is("markup.underline.link") {
        "link_uri"
    } else if is("markup.raw") {
        "text.literal"
    } else if is("markup.list") {
        "punctuation.list_marker"
    } else if is("punctuation.section") || is("punctuation.definition.block") {
        "punctuation.bracket"
    } else if is("punctuation.separator") || is("punctuation.terminator") {
        "punctuation.delimiter"
    } else if is("punctuation") {
        "punctuation"
    } else {
        return None;
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(language: &str, code: &str) -> Vec<(String, String)> {
        highlight_names(language, code)
            .into_iter()
            .map(|(range, name)| (name.to_string(), code[range].to_string()))
            .collect()
    }

    #[test]
    fn common_fence_languages_resolve() {
        let _locale = crate::test_support::chinese();
        for language in [
            "rust",
            "python",
            "typescript",
            "javascript",
            "go",
            "java",
            "c",
            "cpp",
            "json",
            "yaml",
            "toml",
            "bash",
            "ruby",
            "html",
            "css",
            "sql",
            "diff",
            "make",
            "dockerfile",
            "zig",
            "lua",
            "php",
        ] {
            assert!(
                find_syntax(language).is_some(),
                "`{language}` has no syntax"
            );
        }
        assert!(find_syntax("text").is_none());
        assert!(find_syntax("no-such-language").is_none());
    }

    #[test]
    fn scopes_map_onto_the_theme_names() {
        let _locale = crate::test_support::chinese();
        let toml = spans(
            "toml",
            "[package]\nname = \"demo\" # the name\nversion = 1\n",
        );
        assert!(
            toml.contains(&("string".into(), "\"demo\"".into())),
            "{toml:?}"
        );
        assert!(
            toml.iter()
                .any(|(name, text)| name == "comment" && text.contains("the name")),
            "{toml:?}"
        );
        assert!(toml.contains(&("number".into(), "1".into())), "{toml:?}");

        let rust = spans("rust", "fn main() { let x = 42; }\n");
        assert!(rust.contains(&("keyword".into(), "fn".into())), "{rust:?}");
        assert!(
            rust.contains(&("function".into(), "main".into())),
            "{rust:?}"
        );
        assert!(rust.contains(&("number".into(), "42".into())), "{rust:?}");
    }

    #[test]
    fn names_stay_inside_char_boundaries() {
        let _locale = crate::test_support::chinese();
        let code = "let s = \"中文\";\n";
        for (range, _) in highlight_names("rust", code) {
            assert!(code.is_char_boundary(range.start), "{range:?}");
            assert!(code.is_char_boundary(range.end), "{range:?}");
        }
    }
    #[test]
    fn file_syntax_infers_extensions_and_unknown_files_stay_plain() {
        let _locale = crate::test_support::chinese();
        use std::path::Path;
        assert!(!super::highlight_file(Path::new("src/main.rs"), "fn main() {}\n").is_empty());
        assert!(
            !super::highlight_file(Path::new("script.py"), "def main():\n    print('中文')\n")
                .is_empty()
        );
        assert!(
            super::highlight_file(Path::new("file.unknown"), "plain text without syntax")
                .is_empty()
        );
    }
}
