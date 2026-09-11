//! Markdown pipeline: comrak AST to viewer layout model.
//!
//! `parse_markdown` enables GFM tables, task lists, strikethrough,
//! autolinks, superscript, highlight (`==mark==`), insert (`++ins++`) and
//! `---` front matter. Unknown blocks are skipped; unknown inlines are
//! flattened so no text is lost.

use comrak::nodes::{AstNode, ListType, NodeList, NodeTable, NodeValue, TableAlignment};
use comrak::{parse_document, Arena, Options};
use std::path::Path;

/// Read `path` to a string. Never panics: failures become display text so
/// the window can show a banner instead of crashing.
pub fn load_file(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    std::fs::read_to_string(path)
        .unwrap_or_else(|err| format!("md-view: cannot read {}: {err}", path.display()))
}

/// A parsed document: front matter plus top-level blocks.
pub struct Document {
    pub front_matter: Vec<(String, String)>,
    pub blocks: Vec<Block>,
}

/// Block-level layout element.
#[derive(Debug, Clone)]
pub enum Block {
    Heading {
        level: u8,
        text: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Code {
        language: String,
        text: String,
    },
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: usize,
        items: Vec<ListItem>,
    },
    Table {
        alignments: Vec<Align>,
        header: Row,
        rows: Vec<Row>,
    },
    Rule,
}

/// One table row: a cell per column, each cell a run of inlines.
pub type Row = Vec<Vec<Inline>>;

/// One list item. `checked` is `Some` for `- [ ]` / `- [x]` items.
#[derive(Debug, Clone)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// Inline-level layout element.
#[derive(Debug, Clone)]
pub enum Inline {
    Text(String),
    Code(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    /// `==highlighted==` text.
    Mark(Vec<Inline>),
    /// `++inserted++` text, rendered underlined.
    Insert(Vec<Inline>),
    Link {
        label: Vec<Inline>,
        url: String,
    },
    Image {
        alt: String,
        url: String,
    },
    /// `$...$` / `$$...$$` TeX fragment. `display` is true for `$$`.
    Math {
        display: bool,
        tex: String,
    },
    /// `^super^` or `<sup>`.
    Super(Vec<Inline>),
    /// `<sub>`.
    Sub(Vec<Inline>),
    Break,
}

/// Column alignment from the table delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    None,
}

/// Parse Markdown source into a [`Document`].
pub fn parse_markdown(text: &str) -> Document {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.superscript = true;
    options.extension.highlight = true;
    options.extension.insert = true;
    options.extension.math_dollars = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());

    let arena = Arena::new();
    let root = parse_document(&arena, text, &options);

    let mut doc = Document {
        front_matter: Vec::new(),
        blocks: Vec::new(),
    };
    for child in root.children() {
        if let NodeValue::FrontMatter(raw) = &child.data().value {
            doc.front_matter = parse_front_matter(raw);
        } else if let Some(block) = convert_block(child) {
            doc.blocks.push(block);
        }
    }
    doc
}

fn parse_front_matter(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Split a YAML/plain tags value into pill labels.
/// Accepts `a, b`, `[a, b]`, and `["a", "b"]`.
pub fn tag_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let body = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed);
    body.split(',')
        .map(|item| {
            item.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

fn children_blocks<'a>(node: &'a AstNode<'a>) -> Vec<Block> {
    node.children().filter_map(convert_block).collect()
}

fn convert_block<'a>(node: &'a AstNode<'a>) -> Option<Block> {
    match &node.data().value {
        NodeValue::Paragraph => Some(Block::Paragraph(inlines_of(node))),
        NodeValue::Heading(heading) => Some(Block::Heading {
            level: heading.level,
            text: inlines_of(node),
        }),
        NodeValue::CodeBlock(code) => Some(Block::Code {
            language: code
                .info
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase(),
            text: code.literal.clone(),
        }),
        NodeValue::BlockQuote => Some(Block::Quote(children_blocks(node))),
        NodeValue::List(list) => Some(convert_list(node, list)),
        NodeValue::Table(table) => Some(convert_table(node, table)),
        NodeValue::ThematicBreak => Some(Block::Rule),
        _ => None,
    }
}

fn convert_list<'a>(node: &'a AstNode<'a>, list: &NodeList) -> Block {
    let ordered = matches!(list.list_type, ListType::Ordered);
    let mut items = Vec::new();
    for child in node.children() {
        // comrak replaces an `Item` carrying a checkbox with a `TaskItem`
        // in place, keeping the remaining content as its children.
        let checked = match &child.data().value {
            NodeValue::Item(..) => None,
            NodeValue::TaskItem(task) => Some(task.symbol.is_some()),
            _ => continue,
        };
        items.push(ListItem {
            checked,
            blocks: children_blocks(child),
        });
    }
    Block::List {
        ordered,
        start: list.start,
        items,
    }
}

fn convert_table<'a>(node: &'a AstNode<'a>, table: &NodeTable) -> Block {
    let alignments = table
        .alignments
        .iter()
        .map(|alignment| match alignment {
            TableAlignment::Left => Align::Left,
            TableAlignment::Center => Align::Center,
            TableAlignment::Right => Align::Right,
            TableAlignment::None => Align::None,
        })
        .collect();
    let mut header = Vec::new();
    let mut rows = Vec::new();
    for child in node.children() {
        if let NodeValue::TableRow(is_header) = &child.data().value {
            let mut row = Vec::new();
            for cell in child.children() {
                if matches!(&cell.data().value, NodeValue::TableCell) {
                    row.push(inlines_of(cell));
                }
            }
            if *is_header {
                header = row;
            } else {
                rows.push(row);
            }
        }
    }
    Block::Table {
        alignments,
        header,
        rows,
    }
}

fn inlines_of<'a>(node: &'a AstNode<'a>) -> Vec<Inline> {
    let mut frames: Vec<(Option<HtmlWrap>, Vec<Inline>)> = vec![(None, Vec::new())];
    for child in node.children() {
        if let NodeValue::HtmlInline(tag) = &child.data().value {
            apply_html_tag(tag, &mut frames);
        } else {
            collect_inline(child, &mut frames.last_mut().unwrap().1);
        }
    }
    while frames.len() > 1 {
        close_html_frame(&mut frames);
    }
    frames.pop().unwrap().1
}

/// Opening/closing HTML tags that wrap subsequent sibling inlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HtmlWrap {
    Mark,
    Underline,
    Sub,
    Sup,
    Kbd,
}

fn apply_html_tag(tag: &str, frames: &mut Vec<(Option<HtmlWrap>, Vec<Inline>)>) {
    let trimmed = tag.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(kind) = html_open(&lower) {
        frames.push((Some(kind), Vec::new()));
        return;
    }
    if html_close(&lower).is_some() && frames.len() > 1 {
        close_html_frame(frames);
    }
}

fn html_open(tag: &str) -> Option<HtmlWrap> {
    match tag {
        "<mark>" => Some(HtmlWrap::Mark),
        "<u>" | "<ins>" => Some(HtmlWrap::Underline),
        "<sub>" => Some(HtmlWrap::Sub),
        "<sup>" => Some(HtmlWrap::Sup),
        "<kbd>" => Some(HtmlWrap::Kbd),
        _ => None,
    }
}

fn html_close(tag: &str) -> Option<HtmlWrap> {
    match tag {
        "</mark>" => Some(HtmlWrap::Mark),
        "</u>" | "</ins>" => Some(HtmlWrap::Underline),
        "</sub>" => Some(HtmlWrap::Sub),
        "</sup>" => Some(HtmlWrap::Sup),
        "</kbd>" => Some(HtmlWrap::Kbd),
        _ => None,
    }
}

fn close_html_frame(frames: &mut Vec<(Option<HtmlWrap>, Vec<Inline>)>) {
    let Some((kind, inner)) = frames.pop() else {
        return;
    };
    let wrapped = match kind {
        Some(HtmlWrap::Mark) => Inline::Mark(inner),
        Some(HtmlWrap::Underline) => Inline::Insert(inner),
        Some(HtmlWrap::Sub) => Inline::Sub(inner),
        Some(HtmlWrap::Sup) => Inline::Super(inner),
        Some(HtmlWrap::Kbd) => Inline::Code(flatten(&inner)),
        None => {
            if let Some((_, parent)) = frames.last_mut() {
                parent.extend(inner);
            }
            return;
        }
    };
    if let Some((_, parent)) = frames.last_mut() {
        parent.push(wrapped);
    }
}

fn collect_inline<'a>(node: &'a AstNode<'a>, out: &mut Vec<Inline>) {
    match &node.data().value {
        NodeValue::Text(text) => out.push(Inline::Text(text.to_string())),
        NodeValue::Code(code) => out.push(Inline::Code(code.literal.clone())),
        NodeValue::Emph => out.push(Inline::Emph(inlines_of(node))),
        NodeValue::Strong => out.push(Inline::Strong(inlines_of(node))),
        NodeValue::Strikethrough => out.push(Inline::Strike(inlines_of(node))),
        NodeValue::Highlight => out.push(Inline::Mark(inlines_of(node))),
        NodeValue::Insert => out.push(Inline::Insert(inlines_of(node))),
        NodeValue::Superscript => out.push(Inline::Super(inlines_of(node))),
        NodeValue::Math(math) => out.push(Inline::Math {
            display: math.display_math,
            tex: math.literal.trim().to_string(),
        }),
        NodeValue::Link(link) => out.push(Inline::Link {
            label: inlines_of(node),
            url: link.url.clone(),
        }),
        NodeValue::Image(link) => out.push(Inline::Image {
            alt: inline_text(node),
            url: link.url.clone(),
        }),
        NodeValue::SoftBreak | NodeValue::LineBreak => out.push(Inline::Break),
        // Flatten anything else so text is never lost.
        _ => {
            for child in node.children() {
                collect_inline(child, out);
            }
        }
    }
}

/// Flattened text of inline runs: markup removed, breaks as spaces.
pub fn flatten(inlines: &[Inline]) -> String {
    let mut out = String::new();
    push_flat(inlines, &mut out);
    out
}

fn push_flat(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Emph(inner)
            | Inline::Strong(inner)
            | Inline::Strike(inner)
            | Inline::Mark(inner)
            | Inline::Insert(inner)
            | Inline::Super(inner)
            | Inline::Sub(inner) => {
                push_flat(inner, out);
            }
            Inline::Math { tex, .. } => out.push_str(&crate::math::flatten_tex(tex)),
            Inline::Link { label, url } => {
                push_flat(label, out);
                if !url.is_empty() {
                    out.push_str(" (");
                    out.push_str(url);
                    out.push(')');
                }
            }
            Inline::Image { alt, url } => {
                if !alt.is_empty() {
                    out.push_str(alt);
                }
                if !url.is_empty() {
                    out.push_str(" (");
                    out.push_str(url);
                    out.push(')');
                }
            }
            Inline::Break => out.push(' '),
        }
    }
}

/// Content-derived identity for a block. Keys survive edits elsewhere in
/// the file and drive scroll retention across reloads.
pub fn block_key(block: &Block) -> String {
    match block {
        Block::Heading { level, text } => format!("h{level}:{}", flatten(text)),
        Block::Paragraph(lines) => format!("p:{}", truncate(&flatten(lines))),
        Block::Code { language, text } => {
            format!("c:{language}:{}", text.lines().next().unwrap_or_default())
        }
        Block::Quote(inner) => {
            format!("q:{}", inner.first().map(block_key).unwrap_or_default())
        }
        Block::List { ordered, items, .. } => format!(
            "l:{ordered}:{}",
            items.first().map(first_item_line).unwrap_or_default()
        ),
        Block::Table { header, .. } => format!(
            "t:{}",
            header
                .iter()
                .map(|cell| flatten(cell))
                .collect::<Vec<_>>()
                .join("|")
        ),
        Block::Rule => "---".to_string(),
    }
}

fn first_item_line(item: &ListItem) -> String {
    item.blocks
        .iter()
        .find_map(|block| match block {
            Block::Paragraph(lines) => Some(truncate(&flatten(lines))),
            _ => None,
        })
        .unwrap_or_default()
}

fn truncate(text: &str) -> String {
    const MAX: usize = 64;
    if text.len() <= MAX {
        text.to_string()
    } else {
        text[..MAX].to_string()
    }
}

/// Flattened text of a subtree, used for image alt text.
fn inline_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    push_text(node, &mut text);
    text
}

fn push_text<'a>(node: &'a AstNode<'a>, text: &mut String) {
    match &node.data().value {
        NodeValue::Text(fragment) => text.push_str(fragment),
        NodeValue::Code(code) => text.push_str(&code.literal),
        _ => {
            for child in node.children() {
                push_text(child, text);
            }
        }
    }
}

/// One styled run of flattened inline text: unlike [`flatten`], style flags
/// are preserved per run instead of stripped, so a rich renderer can turn
/// each span into a colored/weighted text run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub mark: bool,
    pub insert: bool,
    pub code: bool,
    pub link: bool,
}

/// Style flags carried down through recursive inline runs.
#[derive(Debug, Clone, Copy, Default)]
struct SpanStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    mark: bool,
    insert: bool,
    code: bool,
    link: bool,
}

impl SpanStyle {
    fn span(self, text: String) -> Span {
        Span {
            text,
            bold: self.bold,
            italic: self.italic,
            strike: self.strike,
            mark: self.mark,
            insert: self.insert,
            code: self.code,
            link: self.link,
        }
    }
}

/// True when any inline is display (`$$`) math.
#[cfg(test)]
pub fn has_display_math(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inline| match inline {
        Inline::Math { display: true, .. } => true,
        Inline::Emph(inner)
        | Inline::Strong(inner)
        | Inline::Strike(inner)
        | Inline::Mark(inner)
        | Inline::Insert(inner)
        | Inline::Super(inner)
        | Inline::Sub(inner) => has_display_math(inner),
        Inline::Link { label, .. } => has_display_math(label),
        _ => false,
    })
}

/// Flatten inline runs into styled [`Span`]s for rich rendering. Adjacent
/// runs that end up with identical styling are merged into one span.
pub fn spans(inlines: &[Inline]) -> Vec<Span> {
    let mut out = Vec::new();
    push_spans(inlines, SpanStyle::default(), &mut out);
    out
}

fn push_span(out: &mut Vec<Span>, style: SpanStyle, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        let same_style = last.bold == style.bold
            && last.italic == style.italic
            && last.strike == style.strike
            && last.mark == style.mark
            && last.insert == style.insert
            && last.code == style.code
            && last.link == style.link;
        if same_style {
            last.text.push_str(&text);
            return;
        }
    }
    out.push(style.span(text));
}

fn push_spans(inlines: &[Inline], style: SpanStyle, out: &mut Vec<Span>) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_span(out, style, text.clone()),
            Inline::Code(text) => {
                push_span(
                    out,
                    SpanStyle {
                        code: true,
                        ..style
                    },
                    text.clone(),
                );
            }
            Inline::Emph(inner) => push_spans(
                inner,
                SpanStyle {
                    italic: true,
                    ..style
                },
                out,
            ),
            Inline::Strong(inner) => push_spans(
                inner,
                SpanStyle {
                    bold: true,
                    ..style
                },
                out,
            ),
            Inline::Strike(inner) => push_spans(
                inner,
                SpanStyle {
                    strike: true,
                    ..style
                },
                out,
            ),
            Inline::Mark(inner) => push_spans(
                inner,
                SpanStyle {
                    mark: true,
                    ..style
                },
                out,
            ),
            Inline::Insert(inner) => push_spans(
                inner,
                SpanStyle {
                    insert: true,
                    ..style
                },
                out,
            ),
            Inline::Super(inner) => {
                let text = flatten(inner);
                push_span(out, style, crate::math::unicode_script(&text, true));
            }
            Inline::Sub(inner) => {
                let text = flatten(inner);
                push_span(out, style, crate::math::unicode_script(&text, false));
            }
            Inline::Math { tex, .. } => {
                push_span(out, style, crate::math::flatten_tex(tex));
            }
            Inline::Link { label, url: _ } => {
                push_spans(
                    label,
                    SpanStyle {
                        link: true,
                        ..style
                    },
                    out,
                );
            }
            Inline::Image { alt, url } => {
                if !alt.is_empty() {
                    push_span(out, style, alt.clone());
                }
                if !url.is_empty() {
                    push_span(out, style, format!(" ({url})"));
                }
            }
            Inline::Break => push_span(out, style, " ".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("md-view-test-{name}-{}.md", std::process::id()))
    }

    #[test]
    fn roundtrips_file_contents() {
        let path = scratch("roundtrip");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "# hello").unwrap();
        drop(file);
        let body = load_file(&path);
        assert!(
            body.contains("# hello"),
            "unexpected body: {body:?}",
            body = body
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_file_becomes_display_text() {
        let body = load_file("/nonexistent/md-view-missing-file.md");
        assert!(
            body.starts_with("md-view: cannot read"),
            "unexpected body: {body:?}",
            body = body
        );
    }

    fn first_block(source: &str) -> Block {
        let mut doc = parse_markdown(source);
        assert_eq!(doc.blocks.len(), 1, "expected one block");
        doc.blocks.pop().unwrap()
    }

    #[test]
    fn heading_levels_and_inline_nesting() {
        let Block::Heading { level, text } = first_block("### Hi *there* **you**\n") else {
            panic!("expected heading");
        };
        assert_eq!(level, 3);
        assert_eq!(text.len(), 4);
        assert!(matches!(text[0], Inline::Text(_)));
        assert!(matches!(text[1], Inline::Emph(_)));
        assert!(matches!(text[3], Inline::Strong(_)));
    }

    #[test]
    fn strikethrough_and_link() {
        let Block::Paragraph(inlines) = first_block("a ~~gone~~ [up](https://x.test/y)\n") else {
            panic!("expected paragraph");
        };
        assert!(matches!(inlines[1], Inline::Strike(_)));
        let Inline::Link { label, url } = &inlines[3] else {
            panic!("expected link, got {inlines:?}");
        };
        assert_eq!(url, "https://x.test/y");
        assert!(matches!(label[0], Inline::Text(_)));
    }

    #[test]
    fn fenced_code_keeps_language_and_literal() {
        let Block::Code { language, text } = first_block("```mermaid\ngraph A-->B\n```\n") else {
            panic!("expected code block");
        };
        assert_eq!(language, "mermaid");
        assert!(text.contains("graph A-->B"), "literal: {text:?}");
    }

    #[test]
    fn task_list_checked_states() {
        let Block::List { ordered, items, .. } = first_block("- [x] done\n- [ ] todo\n- plain\n")
        else {
            panic!("expected list");
        };
        assert!(!ordered);
        let states: Vec<_> = items.iter().map(|item| item.checked).collect();
        assert_eq!(states, vec![Some(true), Some(false), None]);
        assert!(matches!(items[0].blocks[0], Block::Paragraph(_)));
    }

    #[test]
    fn ordered_list_start_number() {
        let Block::List { ordered, start, .. } = first_block("3. three\n4. four\n") else {
            panic!("expected list");
        };
        assert!(ordered);
        assert_eq!(start, 3);
    }

    #[test]
    fn table_header_rows_and_alignment() {
        let Block::Table {
            alignments,
            header,
            rows,
        } = first_block("| a | b | c | d |\n|---|:---|---:|:-:|\n| 1 | 2 | 3 | 4 |\n")
        else {
            panic!("expected table");
        };
        assert_eq!(
            alignments,
            vec![Align::None, Align::Left, Align::Right, Align::Center]
        );
        assert_eq!(header.len(), 4);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 4);
    }

    #[test]
    fn front_matter_parses_pairs() {
        let doc = parse_markdown("---\ntitle: Hi\ntags: a, b\n---\n\nBody\n");
        assert_eq!(
            doc.front_matter,
            vec![
                ("title".to_string(), "Hi".to_string()),
                ("tags".to_string(), "a, b".to_string()),
            ]
        );
        assert!(matches!(doc.blocks[0], Block::Paragraph(_)));
    }

    #[test]
    fn yaml_tag_arrays_strip_brackets_and_quotes() {
        assert_eq!(
            tag_list(r#"["markdown", "live-preview", "gfm"]"#),
            vec!["markdown", "live-preview", "gfm"]
        );
        assert_eq!(tag_list("a, b"), vec!["a", "b"]);
    }

    #[test]
    fn html_mark_underline_sub_and_sup() {
        let doc = parse_markdown(
            "use <mark>highlighted text</mark> or <u>underlines</u> H<sub>2</sub>O x<sup>2</sup>\n",
        );
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(
            inlines.iter().any(|i| matches!(i, Inline::Mark(_))),
            "{inlines:?}"
        );
        assert!(
            inlines.iter().any(|i| matches!(i, Inline::Insert(_))),
            "{inlines:?}"
        );
        assert!(
            inlines.iter().any(|i| matches!(i, Inline::Sub(_))),
            "{inlines:?}"
        );
        assert!(
            inlines.iter().any(|i| matches!(i, Inline::Super(_))),
            "{inlines:?}"
        );
        let runs = spans(inlines);
        assert!(
            runs.iter()
                .any(|s| s.mark && s.text.contains("highlighted")),
            "{runs:?}"
        );
        assert!(
            runs.iter()
                .any(|s| s.insert && s.text.contains("underlines")),
            "{runs:?}"
        );
        assert!(
            runs.iter().any(|s| s.text.contains('₂')),
            "subscript: {runs:?}"
        );
        assert!(
            runs.iter().any(|s| s.text.contains('²')),
            "superscript: {runs:?}"
        );
    }

    #[test]
    fn dollar_math_is_a_math_inline() {
        let doc = parse_markdown("Inline equation: $$E = mc^2$$\n");
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(
            inlines.iter().any(|i| matches!(
                i,
                Inline::Math {
                    display: true,
                    tex
                } if tex.contains("E = mc^2")
            )),
            "{inlines:?}"
        );
        assert!(has_display_math(inlines));
    }

    #[test]
    fn quote_rule_and_skipped_html() {
        let doc = parse_markdown("> quoted\n\n---\n\n<div>x</div>\n");
        assert!(matches!(doc.blocks[0], Block::Quote(_)));
        assert!(matches!(doc.blocks[1], Block::Rule));
        assert_eq!(doc.blocks.len(), 2, "html block must be skipped");
    }

    #[test]
    fn flatten_strips_markup() {
        let doc = parse_markdown("Hi *there* **you** `code`\n");
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        assert_eq!(flatten(inlines), "Hi there you code");
    }

    #[test]
    fn flatten_keeps_link_destination() {
        let doc = parse_markdown("See [docs](https://x.test/y) now\n");
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        let text = flatten(inlines);
        assert!(
            text.contains("docs (https://x.test/y)"),
            "missing destination: {text:?}"
        );
    }

    #[test]
    fn flatten_covers_heading_task_and_table_cells() {
        let doc = parse_markdown("# T\n\n- [x] a\n\n| h |\n|---|\n| b |\n");
        let Block::Heading { text, .. } = &doc.blocks[0] else {
            panic!("expected heading");
        };
        assert_eq!(flatten(text), "T");
        let Block::List { items, .. } = &doc.blocks[1] else {
            panic!("expected list");
        };
        let Block::Paragraph(inlines) = &items[0].blocks[0] else {
            panic!("expected paragraph");
        };
        assert_eq!(flatten(inlines), "a");
        let Block::Table { header, rows, .. } = &doc.blocks[2] else {
            panic!("expected table");
        };
        assert_eq!(flatten(&header[0]), "h");
        assert_eq!(flatten(&rows[0][0]), "b");
    }

    #[test]
    fn spans_preserve_mark_and_insert() {
        let doc = parse_markdown("a ==mark== b ++ins++ c\n");
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        let runs = spans(inlines);
        assert!(
            runs.iter().any(|s| s.mark && s.text == "mark"),
            "runs: {runs:?}"
        );
        assert!(
            runs.iter().any(|s| s.insert && s.text == "ins"),
            "runs: {runs:?}"
        );
        assert!(runs.iter().any(|s| !s.mark && !s.insert && s.text == "a "));
    }

    #[test]
    fn spans_flag_bold_italic_code_and_link() {
        let doc = parse_markdown("**bold** *it* `code` [x](https://y.test)\n");
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        let runs = spans(inlines);
        assert!(runs.iter().any(|s| s.bold && s.text == "bold"));
        assert!(runs.iter().any(|s| s.italic && s.text == "it"));
        assert!(runs.iter().any(|s| s.code && s.text == "code"));
        assert!(runs.iter().any(|s| s.link && s.text == "x"));
        assert!(
            runs.iter().all(|s| !s.text.contains("https://y.test")),
            "visual spans must not append the URL: {runs:?}"
        );
    }

    #[test]
    fn spans_merge_adjacent_same_style_runs() {
        let doc = parse_markdown("hello\nworld\n");
        let Block::Paragraph(inlines) = &doc.blocks[0] else {
            panic!("expected paragraph");
        };
        // A soft break sits between the two text runs; spans() should merge
        // them since neither carries any style.
        let runs = spans(inlines);
        assert_eq!(
            runs,
            vec![Span {
                text: "hello world".to_string(),
                ..Span::default()
            }]
        );
    }
}
