//! Syntax highlighting: syntect engine plus a handwritten theme.
//!
//! The theme maps syntect's scope families to the Neovim `tokyo_custom`
//! palette. TOML, INI, and CSV have no grammar in syntect's pinned defaults,
//! so small grammars ship under `syntaxes/` and load from embedded strings.
//! Unknown languages fall back to plain text, never an error.

use std::collections::HashMap;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{SyntaxDefinition, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::theme;

/// One styled run inside a code line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub color: u32,
    pub italic: bool,
}

struct Engine {
    syntaxes: SyntaxSet,
    theme: Theme,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
        for (source, name) in [
            (
                include_str!("../syntaxes/toml.sublime-syntax"),
                "TOML (md-view)",
            ),
            (
                include_str!("../syntaxes/ini.sublime-syntax"),
                "INI (md-view)",
            ),
            (
                include_str!("../syntaxes/csv.sublime-syntax"),
                "CSV (md-view)",
            ),
        ] {
            let definition = SyntaxDefinition::load_from_str(source, true, Some(name))
                .expect("bundled syntax must parse");
            builder.add(definition);
        }
        Engine {
            syntaxes: builder.build(),
            theme: tokyo_theme(),
        }
    })
}

fn rgb(color: u32) -> Color {
    Color {
        r: ((color >> 16) & 0xFF) as u8,
        g: ((color >> 8) & 0xFF) as u8,
        b: (color & 0xFF) as u8,
        a: 0xFF,
    }
}

fn hex(color: Color) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

fn item(selector: &str, color: u32, italic: bool) -> ThemeItem {
    ThemeItem {
        scope: selector.parse::<ScopeSelectors>().expect("static selector"),
        style: StyleModifier {
            foreground: Some(rgb(color)),
            background: None,
            font_style: Some(if italic {
                FontStyle::ITALIC
            } else {
                FontStyle::empty()
            }),
        },
    }
}

/// Handwritten equivalent of the Neovim `tokyo_custom` groups, ordered
/// general to specific so ties resolve toward the specific rule.
fn tokyo_theme() -> Theme {
    let mut theme = Theme::default();
    theme.name = Some("tokyo_custom".to_string());
    theme.settings = ThemeSettings {
        foreground: Some(rgb(theme::FG)),
        background: Some(rgb(theme::BLOCK_BG)),
        ..Default::default()
    };
    theme.scopes = vec![
        item("comment", theme::COMMENT, true),
        item("string", theme::STRING, false),
        item("constant", theme::CONSTANT, false),
        item("keyword.operator", theme::OPERATOR, false),
        item("keyword", theme::KEYWORD, true),
        item("storage.type", theme::TYPE, false),
        item("storage", theme::KEYWORD, true),
        // Declaration keywords (`fn`, `struct`, `enum`, `trait`, `impl`, `mod`)
        // share syntect's `storage.type.*` scope family with primitive type
        // names, but are keywords, not types. These selectors are more
        // specific than the plain `storage.type` rule above, so they win.
        item(
            "storage.type.function, storage.type.struct, storage.type.enum, \
             storage.type.trait, storage.type.impl, storage.type.module",
            theme::KEYWORD,
            true,
        ),
        item("entity.name.function", theme::FUNCTION, true),
        item("entity.name.method", theme::FUNCTION, true),
        item("entity.name.type", theme::TYPE, false),
        item("entity.name.class", theme::TYPE, false),
        item("entity.name.struct", theme::TYPE, false),
        item("support.type", theme::TYPE, false),
        item("support.class", theme::TYPE, false),
        item("entity.name.tag", theme::MAGENTA, false),
        item("entity.name.section", theme::MAGENTA, false),
        item("variable.member", theme::GREEN, false),
        item("variable.property", theme::GREEN, false),
        item("punctuation", theme::OPERATOR, false),
        item("variable", theme::FG, false),
    ];
    theme
}

/// Highlight a fenced code block into token rows, one group per line.
/// Newlines terminate groups and never appear inside tokens.
pub fn highlight_code(language: &str, text: &str) -> Vec<Vec<Token>> {
    let engine = engine();
    let tag = language.trim().to_lowercase();
    let syntax = engine
        .syntaxes
        .find_syntax_by_token(&tag)
        .or_else(|| engine.syntaxes.find_syntax_by_extension(&tag))
        .unwrap_or_else(|| engine.syntaxes.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, &engine.theme);
    let mut rows = Vec::new();
    for line in LinesWithEndings::from(text) {
        let mut tokens = Vec::new();
        if let Ok(spans) = highlighter.highlight_line(line, &engine.syntaxes) {
            for (style, fragment) in spans {
                let fragment = fragment.strip_suffix('\n').unwrap_or(fragment);
                if fragment.is_empty() {
                    continue;
                }
                tokens.push(Token {
                    text: fragment.to_string(),
                    color: hex(style.foreground),
                    italic: style.font_style.contains(FontStyle::ITALIC),
                });
            }
        }
        if tokens.is_empty() && !line.trim_end_matches('\n').is_empty() {
            tokens.push(Token {
                text: line.trim_end_matches('\n').to_string(),
                color: theme::FG,
                italic: false,
            });
        }
        rows.push(tokens);
    }
    rows
}

/// Highlight every fenced code block in a document, keyed by block index.
pub fn highlight_document(doc: &crate::markdown::Document) -> HashMap<usize, Vec<Vec<Token>>> {
    doc.blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block {
            crate::markdown::Block::Code { language, text } => {
                Some((index, highlight_code(language, text)))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(rows: &[Vec<Token>]) -> Vec<u32> {
        rows.iter()
            .flat_map(|row| row.iter().map(|token| token.color))
            .collect()
    }

    fn joined(rows: &[Vec<Token>]) -> String {
        rows.iter()
            .map(|row| {
                row.iter()
                    .map(|token| token.text.clone())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn json_strings_numbers_and_booleans() {
        let rows = highlight_code("json", "{\"name\": \"x\", \"n\": 12, \"ok\": true}");
        let palette = colors(&rows);
        assert!(palette.contains(&theme::STRING), "strings: {rows:?}");
        assert!(palette.contains(&theme::CONSTANT), "numbers: {rows:?}");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn rust_comment_keyword_and_function() {
        let rows = highlight_code("rust", "// note\nfn main() {}");
        assert_eq!(rows.len(), 2);
        assert!(rows[0]
            .iter()
            .any(|token| token.color == theme::COMMENT && token.italic));
        assert!(
            rows[1]
                .iter()
                .any(|token| token.color == theme::KEYWORD && token.text == "fn"),
            "keyword: {rows:?}"
        );
        assert!(
            rows[1]
                .iter()
                .any(|token| token.color == theme::FUNCTION && token.text.contains("main")),
            "function: {rows:?}"
        );
    }

    #[test]
    fn unknown_language_falls_back_to_plain() {
        let rows = highlight_code("mermaid", "graph A-->B");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].iter().all(|token| token.color == theme::FG));
        assert_eq!(joined(&rows), "graph A-->B");
    }

    #[test]
    fn toml_sections_keys_strings_numbers() {
        let rows = highlight_code("toml", "# c\n[table]\nkey = 1\nname = \"x\"");
        assert_eq!(rows.len(), 4);
        assert!(rows[0].iter().any(|token| token.color == theme::COMMENT));
        assert!(rows[1].iter().any(|token| token.color == theme::MAGENTA));
        assert!(rows[2].iter().any(|token| token.color == theme::CONSTANT));
        assert!(rows[3].iter().any(|token| token.color == theme::STRING));
    }

    #[test]
    fn ini_sections_and_values() {
        let rows = highlight_code("ini", "; c\n[section]\nkey = 8080");
        assert_eq!(rows.len(), 3);
        assert!(rows[0].iter().any(|token| token.color == theme::COMMENT));
        assert!(rows[1].iter().any(|token| token.color == theme::MAGENTA));
        assert!(rows[2].iter().any(|token| token.color == theme::CONSTANT));
    }

    #[test]
    fn csv_quotes_numbers_and_commas() {
        let rows = highlight_code("csv", "\"a\",1\n\"b\",2");
        assert_eq!(rows.len(), 2);
        let palette = colors(&rows);
        assert!(palette.contains(&theme::STRING), "quoted: {rows:?}");
        assert!(palette.contains(&theme::CONSTANT), "numbers: {rows:?}");
        assert!(palette.contains(&theme::OPERATOR), "commas: {rows:?}");
    }

    #[test]
    fn blank_and_empty_inputs_keep_shape() {
        assert!(highlight_code("rust", "").is_empty());
        let rows = highlight_code("rust", "a\n\nb\n");
        assert_eq!(rows.len(), 3);
        assert!(rows[1].is_empty());
        assert_eq!(joined(&rows), "a\n\nb");
    }
}
