//! Vim-style navigation over a document's flattened lines.
//!
//! Pure state machine over [`Line`]s: no GPUI dependency, so every motion,
//! selection, search, and yank is unit-tested here. The viewer maps GPUI
//! keystrokes to [`Key`] and writes [`Outcome::Yanked`] text to the Wayland
//! clipboard itself.

use crate::markdown::{block_key, flatten, Align, Block, Document, Inline};

/// What kind of block a navigable row renders, carrying whatever structure
/// (heading level, inline spans, table cells/alignment, ...) the renderer
/// needs. `Line::text` stays the flattened plain form used for search/yank;
/// this is the parallel rich-rendering view of the same row.
#[derive(Debug, Clone)]
pub enum RowContent {
    Heading {
        level: u8,
        spans: Vec<Inline>,
    },
    Paragraph {
        spans: Vec<Inline>,
    },
    Code,
    ListItem {
        ordered: bool,
        number: usize,
        checked: Option<bool>,
        spans: Vec<Inline>,
    },
    TableRow {
        cells: Vec<Vec<Inline>>,
        alignments: Vec<Align>,
        header: bool,
    },
    Rule,
}

/// One navigable row: its source block, the block's content-derived key for
/// scroll retention, its flattened text, and the structure needed to render
/// it richly.
#[derive(Debug, Clone)]
pub struct Line {
    pub block: usize,
    pub key: String,
    pub text: String,
    /// Nested `>` levels this row sits inside, for left-border indent.
    pub quote_depth: usize,
    /// Nested list levels this row sits inside, for marker indent.
    pub list_depth: usize,
    pub content: RowContent,
}

/// One row produced while walking a block's structure.
struct Row {
    text: String,
    quote_depth: usize,
    list_depth: usize,
    content: RowContent,
}

/// Flatten a document into navigable lines, one entry per row.
pub fn build_lines(doc: &Document) -> Vec<Line> {
    let mut lines = Vec::new();
    for (block_index, block) in doc.blocks.iter().enumerate() {
        for row in block_rows(block, 0, 0) {
            lines.push(Line {
                block: block_index,
                key: block_key(block),
                text: row.text,
                quote_depth: row.quote_depth,
                list_depth: row.list_depth,
                content: row.content,
            });
        }
    }
    lines
}

/// Rows for one block: code keeps its lines, quotes are `> `-prefixed and
/// increment `quote_depth`, list items increment `list_depth` for their
/// nested content. Produces the exact same `.text` values as the previous
/// text-only flattening; `content`/depths are additional, render-only data.
fn block_rows(block: &Block, quote_depth: usize, list_depth: usize) -> Vec<Row> {
    match block {
        Block::Heading { level, text } => vec![Row {
            text: flatten(text),
            quote_depth,
            list_depth,
            content: RowContent::Heading {
                level: *level,
                spans: text.clone(),
            },
        }],
        Block::Paragraph(text) => vec![Row {
            text: flatten(text),
            quote_depth,
            list_depth,
            content: RowContent::Paragraph {
                spans: text.clone(),
            },
        }],
        Block::Code { text, .. } => {
            let texts: Vec<String> = text.lines().map(str::to_string).collect();
            let texts = if texts.is_empty() {
                vec![String::new()]
            } else {
                texts
            };
            texts
                .into_iter()
                .map(|text| Row {
                    text,
                    quote_depth,
                    list_depth,
                    content: RowContent::Code,
                })
                .collect()
        }
        Block::Quote(inner) => inner
            .iter()
            .flat_map(|block| block_rows(block, quote_depth + 1, list_depth))
            .map(|row| Row {
                text: format!("> {}", row.text),
                quote_depth: row.quote_depth,
                list_depth: row.list_depth,
                content: row.content,
            })
            .collect(),
        Block::List {
            ordered,
            start,
            items,
        } => {
            let mut rows = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", start + i)
                } else {
                    "- ".to_string()
                };
                let check = match item.checked {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };
                let mut item_rows: Vec<Row> = item
                    .blocks
                    .iter()
                    .flat_map(|nested| block_rows(nested, quote_depth, list_depth + 1))
                    .collect();
                if let Some((first, rest)) = item_rows.split_first_mut() {
                    let spans = match &first.content {
                        RowContent::Paragraph { spans } => spans.clone(),
                        RowContent::Heading { spans, .. } => spans.clone(),
                        _ => vec![Inline::Text(first.text.clone())],
                    };
                    first.text = format!("{marker}{check}{}", first.text);
                    first.list_depth = list_depth;
                    first.content = RowContent::ListItem {
                        ordered: *ordered,
                        number: start + i,
                        checked: item.checked,
                        spans,
                    };
                    // Matches the previous flattening's continuation indent.
                    for row in rest {
                        row.text = format!("  {}", row.text);
                    }
                } else {
                    item_rows.push(Row {
                        text: format!("{marker}{check}"),
                        quote_depth,
                        list_depth,
                        content: RowContent::ListItem {
                            ordered: *ordered,
                            number: start + i,
                            checked: item.checked,
                            spans: Vec::new(),
                        },
                    });
                }
                rows.extend(item_rows);
            }
            rows
        }
        Block::Table {
            alignments,
            header,
            rows,
        } => {
            let mut out = vec![Row {
                text: join_row(header),
                quote_depth,
                list_depth,
                content: RowContent::TableRow {
                    cells: header.clone(),
                    alignments: alignments.clone(),
                    header: true,
                },
            }];
            out.extend(rows.iter().map(|row| Row {
                text: join_row(row),
                quote_depth,
                list_depth,
                content: RowContent::TableRow {
                    cells: row.clone(),
                    alignments: alignments.clone(),
                    header: false,
                },
            }));
            out
        }
        Block::Rule => vec![Row {
            text: "---".to_string(),
            quote_depth,
            list_depth,
            content: RowContent::Rule,
        }],
    }
}

fn join_row(row: &[Vec<crate::markdown::Inline>]) -> String {
    row.iter()
        .map(|cell| flatten(cell))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Cursor position: line index plus char offset within the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

/// Input key, already mapped from the platform keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Esc,
    Enter,
    Backspace,
    Ctrl(char),
}

/// Editing mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Visual { anchor: Pos, linewise: bool },
    Search { query: String },
}

/// Vim navigation state.
pub struct Vim {
    pub mode: Mode,
    pub cursor: Pos,
    want_col: usize,
    count: Option<usize>,
    pending_y: bool,
    pending_g: bool,
    last_search: String,
    pub message: String,
}

/// What a key did: re-render, ignore, or yank text to the clipboard.
pub enum Outcome {
    Changed,
    Ignored,
    Yanked(String),
}

impl Vim {
    pub fn new() -> Self {
        Vim {
            mode: Mode::Normal,
            cursor: Pos { line: 0, col: 0 },
            want_col: 0,
            count: None,
            pending_y: false,
            pending_g: false,
            last_search: String::new(),
            message: String::new(),
        }
    }

    /// Mode label for the status line.
    pub fn status(&self) -> String {
        let mode = match &self.mode {
            Mode::Normal => "-- NORMAL --",
            Mode::Visual { linewise, .. } if *linewise => "-- VISUAL LINE --",
            Mode::Visual { .. } => "-- VISUAL --",
            Mode::Search { query } => return format!("/{query}"),
        };
        match self.count {
            Some(n) => format!("{n}{mode}"),
            None => mode.to_string(),
        }
    }

    /// Handle one key against the current lines. `half` is the Ctrl-d/u
    /// stride in lines.
    pub fn handle_key(&mut self, lines: &[Line], key: Key, half: usize) -> Outcome {
        match self.mode.clone() {
            Mode::Search { .. } => self.search_key(lines, key),
            Mode::Visual { anchor, linewise } => self.visual_key(lines, key, anchor, linewise),
            Mode::Normal => self.normal_key(lines, key, half),
        }
    }

    /// Rebase the cursor onto reloaded lines via the previous block key.
    /// Drops to Normal: selections do not survive reloads.
    pub fn rebase(&mut self, old_key: Option<String>, new_lines: &[Line]) {
        self.mode = Mode::Normal;
        self.count = None;
        self.pending_y = false;
        self.pending_g = false;
        if let Some(key) = old_key {
            if let Some(index) = new_lines.iter().position(|line| line.key == key) {
                self.cursor.line = index;
            }
        }
        self.clamp(new_lines);
        self.want_col = self.cursor.col;
    }

    fn normal_key(&mut self, lines: &[Line], key: Key, half: usize) -> Outcome {
        match key {
            Key::Esc => {
                let had = self.count.is_some()
                    || self.pending_y
                    || self.pending_g
                    || !self.message.is_empty();
                self.count = None;
                self.pending_y = false;
                self.pending_g = false;
                self.message.clear();
                if had {
                    Outcome::Changed
                } else {
                    Outcome::Ignored
                }
            }
            Key::Char(c) if c.is_ascii_digit() => {
                if c == '0' && self.count.is_none() {
                    self.cursor.col = 0;
                    self.want_col = 0;
                    self.message.clear();
                    return Outcome::Changed;
                }
                let digit = (c as u8 - b'0') as usize;
                self.count = Some(
                    self.count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(digit),
                );
                self.pending_y = false;
                self.pending_g = false;
                Outcome::Changed
            }
            _ => {
                let repeat = self.count.take().unwrap_or(1);
                let was_y = std::mem::replace(&mut self.pending_y, false);
                let was_g = std::mem::replace(&mut self.pending_g, false);
                match key {
                    Key::Char('j') | Key::Down => self.move_by(lines, repeat as isize),
                    Key::Char('k') | Key::Up => self.move_by(lines, -(repeat as isize)),
                    Key::Char('g') => {
                        // Second `g` goes to the top; `gg` arrives as two keys.
                        if was_g {
                            self.goto_line(lines, repeat.saturating_sub(1));
                        } else {
                            self.pending_g = true;
                            if repeat != 1 {
                                self.count = Some(repeat);
                            }
                        }
                        self.message.clear();
                        Outcome::Changed
                    }
                    Key::Char('G') => {
                        self.message.clear();
                        if repeat == 1 {
                            self.goto_line(lines, lines.len().saturating_sub(1));
                        } else {
                            self.goto_line(lines, repeat.saturating_sub(1));
                        }
                        Outcome::Changed
                    }
                    Key::Ctrl('d') => self.move_by(lines, (half * repeat) as isize),
                    Key::Ctrl('u') => self.move_by(lines, -((half * repeat) as isize)),
                    Key::Char('v') => {
                        if lines.is_empty() {
                            return Outcome::Ignored;
                        }
                        self.message.clear();
                        self.pending_g = false;
                        self.mode = Mode::Visual {
                            anchor: self.cursor,
                            linewise: false,
                        };
                        Outcome::Changed
                    }
                    Key::Char('V') => {
                        if lines.is_empty() {
                            return Outcome::Ignored;
                        }
                        self.message.clear();
                        self.pending_g = false;
                        self.mode = Mode::Visual {
                            anchor: self.cursor,
                            linewise: true,
                        };
                        Outcome::Changed
                    }
                    Key::Char('/') => {
                        self.message.clear();
                        self.pending_g = false;
                        self.mode = Mode::Search {
                            query: String::new(),
                        };
                        Outcome::Changed
                    }
                    Key::Char('n') => self.jump_search(lines, true),
                    Key::Char('N') => self.jump_search(lines, false),
                    Key::Char('y') => {
                        if was_y {
                            self.yank_current_line(lines)
                        } else if lines.is_empty() {
                            self.message = "nothing to yank".to_string();
                            Outcome::Changed
                        } else {
                            self.pending_y = true;
                            Outcome::Changed
                        }
                    }
                    _ => {
                        self.message.clear();
                        Outcome::Ignored
                    }
                }
            }
        }
    }

    fn visual_key(&mut self, lines: &[Line], key: Key, anchor: Pos, linewise: bool) -> Outcome {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.message.clear();
                Outcome::Changed
            }
            Key::Char('v') if !linewise => {
                self.mode = Mode::Normal;
                Outcome::Changed
            }
            Key::Char('V') => {
                self.mode = Mode::Visual {
                    anchor,
                    linewise: !linewise,
                };
                Outcome::Changed
            }
            Key::Char('y') => {
                let (start, end) = ordered(anchor, self.cursor);
                let text = yank_range(lines, start, end, linewise);
                let count = if linewise {
                    end.line - start.line + 1
                } else {
                    text.chars().count()
                };
                let unit = if linewise { "line" } else { "char" };
                let plural = if count == 1 { "" } else { "s" };
                self.message = format!("yanked {count} {unit}{plural}");
                self.cursor = start;
                self.want_col = start.col;
                self.mode = Mode::Normal;
                Outcome::Yanked(text)
            }
            motion => {
                // Motions extend the selection; digits and search work too.
                let saved = self.cursor;
                let outcome = self.normal_key(lines, motion, 20);
                match outcome {
                    Outcome::Changed => {
                        self.mode = Mode::Visual { anchor, linewise };
                        Outcome::Changed
                    }
                    other => {
                        self.cursor = saved;
                        self.mode = Mode::Visual { anchor, linewise };
                        other
                    }
                }
            }
        }
    }

    fn search_key(&mut self, lines: &[Line], key: Key) -> Outcome {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.message.clear();
                Outcome::Changed
            }
            Key::Enter => {
                let query = match &self.mode {
                    Mode::Search { query } if !query.is_empty() => query.clone(),
                    _ => self.last_search.clone(),
                };
                if query.is_empty() {
                    self.mode = Mode::Normal;
                    self.message = "no search pattern".to_string();
                    return Outcome::Changed;
                }
                self.last_search = query;
                self.mode = Mode::Normal;
                self.jump_search(lines, true)
            }
            Key::Backspace => {
                if let Mode::Search { query } = &mut self.mode {
                    query.pop();
                }
                Outcome::Changed
            }
            Key::Char(c) => {
                if let Mode::Search { query } = &mut self.mode {
                    query.push(c);
                }
                Outcome::Changed
            }
            _ => Outcome::Ignored,
        }
    }

    fn move_by(&mut self, lines: &[Line], delta: isize) -> Outcome {
        if lines.is_empty() {
            return Outcome::Ignored;
        }
        let next = (self.cursor.line as isize + delta).clamp(0, lines.len() as isize - 1) as usize;
        if next == self.cursor.line {
            return Outcome::Ignored;
        }
        self.cursor.line = next;
        self.cursor.col = self.want_col.min(char_len(&lines[next].text));
        self.message.clear();
        Outcome::Changed
    }

    fn goto_line(&mut self, lines: &[Line], index: usize) {
        if lines.is_empty() {
            self.cursor = Pos { line: 0, col: 0 };
            self.want_col = 0;
            return;
        }
        self.cursor.line = index.min(lines.len() - 1);
        self.cursor.col = self.want_col.min(char_len(&lines[self.cursor.line].text));
    }

    fn yank_current_line(&mut self, lines: &[Line]) -> Outcome {
        let Some(line) = lines.get(self.cursor.line) else {
            self.message = "nothing to yank".to_string();
            return Outcome::Changed;
        };
        self.message = "yanked 1 line".to_string();
        Outcome::Yanked(line.text.clone())
    }

    fn jump_search(&mut self, lines: &[Line], forward: bool) -> Outcome {
        if self.last_search.is_empty() {
            self.message = "no search pattern".to_string();
            return Outcome::Changed;
        }
        let matches = find_matches(lines, &self.last_search);
        if matches.is_empty() {
            self.message = format!("pattern not found: {}", self.last_search);
            return Outcome::Changed;
        }
        let at = (self.cursor.line, self.cursor.col);
        let next = if forward {
            matches
                .iter()
                .find(|pos| (pos.line, pos.col) > at)
                .or(matches.first())
        } else {
            matches
                .iter()
                .rev()
                .find(|pos| (pos.line, pos.col) < at)
                .or(matches.last())
        };
        if let Some(pos) = next {
            self.cursor = *pos;
            self.want_col = pos.col;
            self.message.clear();
        }
        Outcome::Changed
    }

    fn clamp(&mut self, lines: &[Line]) {
        if lines.is_empty() {
            self.cursor = Pos { line: 0, col: 0 };
            return;
        }
        self.cursor.line = self.cursor.line.min(lines.len() - 1);
        self.cursor.col = self.cursor.col.min(char_len(&lines[self.cursor.line].text));
    }
}

fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if (a.line, a.col) <= (b.line, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn char_slice(text: &str, from: usize, to: usize) -> String {
    text.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}

/// Yank text for a normalized [start, end) range (end-exclusive).
fn yank_range(lines: &[Line], start: Pos, end: Pos, linewise: bool) -> String {
    if lines.is_empty() || start.line >= lines.len() {
        return String::new();
    }
    if linewise {
        let last = end.line.min(lines.len() - 1);
        return lines[start.line..=last]
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }
    let last = end.line.min(lines.len() - 1);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate().take(last + 1).skip(start.line) {
        let len = char_len(&line.text);
        let from = if i == start.line {
            start.col.min(len)
        } else {
            0
        };
        let to = if i == last { end.col.min(len) } else { len };
        out.push(char_slice(&line.text, from, to));
    }
    out.join("\n")
}

/// All literal substring matches as (line, char col), top to bottom.
fn find_matches(lines: &[Line], query: &str) -> Vec<Pos> {
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let mut rest = line.text.as_str();
        let mut byte_base = 0;
        while let Some(byte) = rest.find(query) {
            let col = line.text[..byte_base + byte].chars().count();
            out.push(Pos { line: i, col });
            let next = byte + query.len().max(1);
            byte_base += next;
            rest = &rest[next.min(rest.len())..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::parse_markdown;

    fn doc_lines(source: &str) -> Vec<Line> {
        build_lines(&parse_markdown(source))
    }

    fn press(vim: &mut Vim, lines: &[Line], key: Key) -> Outcome {
        vim.handle_key(lines, key, 20)
    }

    fn chars(s: &str) -> Vec<Key> {
        s.chars().map(Key::Char).collect()
    }

    #[test]
    fn jk_move_and_clamp_with_count() {
        let lines = doc_lines("# a\n\n# b\n\n# c\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('3'));
        press(&mut vim, &lines, Key::Char('j'));
        assert_eq!(vim.cursor.line, 2);
        assert!(matches!(
            press(&mut vim, &lines, Key::Char('j')),
            Outcome::Ignored
        ));
        press(&mut vim, &lines, Key::Char('2'));
        press(&mut vim, &lines, Key::Char('k'));
        assert_eq!(vim.cursor.line, 0);
    }

    #[test]
    fn gg_g_and_goto_line() {
        let lines = doc_lines("# a\n\n# b\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('G'));
        assert_eq!(vim.cursor.line, 1);
        press(&mut vim, &lines, Key::Char('2'));
        press(&mut vim, &lines, Key::Char('G'));
        assert_eq!(vim.cursor.line, 1);
        for key in chars("gg") {
            press(&mut vim, &lines, key);
        }
        assert_eq!(vim.cursor.line, 0);
    }

    #[test]
    fn stray_g_does_not_latch() {
        let lines = doc_lines("# a\n\n# b\n\n# c\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('g'));
        press(&mut vim, &lines, Key::Char('j'));
        assert_eq!(vim.cursor.line, 1);
        // The earlier lone `g` was cleared by `j`: this `g` only arms.
        press(&mut vim, &lines, Key::Char('g'));
        assert_eq!(vim.cursor.line, 1);
        press(&mut vim, &lines, Key::Char('g'));
        assert_eq!(vim.cursor.line, 0);
    }

    #[test]
    fn count_survives_first_g() {
        let lines = doc_lines("# 1\n\n# 2\n\n# 3\n\n# 4\n\n# 5\n");
        let mut vim = Vim::new();
        for key in [Key::Char('5'), Key::Char('g'), Key::Char('g')] {
            press(&mut vim, &lines, key);
        }
        assert_eq!(vim.cursor.line, 4);
    }

    #[test]
    fn ctrl_d_u_scroll_by_half_page() {
        let lines = doc_lines("# 1\n\n# 2\n\n# 3\n\n# 4\n\n# 5\n");
        let mut vim = Vim::new();
        vim.handle_key(&lines, Key::Ctrl('d'), 2);
        assert_eq!(vim.cursor.line, 2);
        vim.handle_key(&lines, Key::Ctrl('u'), 2);
        assert_eq!(vim.cursor.line, 0);
    }

    #[test]
    fn visual_char_yank_across_lines() {
        let lines = doc_lines("hello world\n\nsecond line\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('v'));
        press(&mut vim, &lines, Key::Char('j'));
        let Outcome::Yanked(text) = press(&mut vim, &lines, Key::Char('y')) else {
            panic!("expected yank");
        };
        assert_eq!(text, "hello world\n");
        assert_eq!(vim.message, "yanked 12 chars");
        assert_eq!(vim.mode, Mode::Normal);
    }

    #[test]
    fn visual_line_yank_whole_lines() {
        let lines = doc_lines("# a\n\n# b\n\n# c\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('V'));
        press(&mut vim, &lines, Key::Char('j'));
        let Outcome::Yanked(text) = press(&mut vim, &lines, Key::Char('y')) else {
            panic!("expected yank");
        };
        assert_eq!(text, "a\nb");
        assert_eq!(vim.message, "yanked 2 lines");
    }

    #[test]
    fn yy_yanks_current_line() {
        let lines = doc_lines("# a\n\n# b\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('G'));
        press(&mut vim, &lines, Key::Char('y'));
        let Outcome::Yanked(text) = press(&mut vim, &lines, Key::Char('y')) else {
            panic!("expected yank");
        };
        assert_eq!(text, "b");
        // A stray `y` arms nothing permanent: next motion clears it.
        press(&mut vim, &lines, Key::Char('y'));
        press(&mut vim, &lines, Key::Char('k'));
        assert_eq!(vim.cursor.line, 0);
    }

    #[test]
    fn search_wrap_n_and_not_found() {
        let lines = doc_lines("alpha\n\nbeta alpha\n");
        let mut vim = Vim::new();
        for key in chars("/alpha") {
            press(&mut vim, &lines, key);
        }
        press(&mut vim, &lines, Key::Enter);
        assert_eq!(vim.cursor, Pos { line: 1, col: 5 });
        press(&mut vim, &lines, Key::Char('n'));
        assert_eq!(vim.cursor, Pos { line: 0, col: 0 });
        press(&mut vim, &lines, Key::Char('n'));
        assert_eq!(vim.cursor, Pos { line: 1, col: 5 });
        press(&mut vim, &lines, Key::Char('N'));
        assert_eq!(vim.cursor, Pos { line: 0, col: 0 });
        for key in chars("/zzz") {
            press(&mut vim, &lines, key);
        }
        press(&mut vim, &lines, Key::Enter);
        assert!(vim.message.contains("pattern not found"));
    }

    #[test]
    fn rebase_tracks_moved_block_and_clamps() {
        let old = build_lines(&parse_markdown("# A\n\n# B\n"));
        let new = build_lines(&parse_markdown("# New\n\n# A\n\n# B\n"));
        let mut vim = Vim::new();
        vim.cursor.line = 1;
        let key = old[1].key.clone();
        vim.rebase(Some(key), &new);
        assert_eq!(vim.cursor.line, 2);
        assert_eq!(vim.mode, Mode::Normal);
        // Gone block: keep the clamped line.
        let shorter = build_lines(&parse_markdown("# A\n"));
        vim.cursor.line = 1;
        vim.rebase(Some("nope".to_string()), &shorter);
        assert_eq!(vim.cursor.line, 0);
    }

    #[test]
    fn code_blocks_keep_lines_and_quote_prefix() {
        let lines = build_lines(&parse_markdown(
            "> hi\n\n```rs\nlet x = 1;\nlet y = 2;\n```\n",
        ));
        assert_eq!(lines[0].text, "> hi");
        assert_eq!(lines[1].text, "let x = 1;");
        assert_eq!(lines[2].text, "let y = 2;");
        // Block ownership underpins per-block highlight/diagram rendering.
        assert_ne!(lines[0].block, lines[1].block);
        assert_eq!(lines[1].block, lines[2].block);
    }

    #[test]
    fn empty_doc_is_safe() {
        let lines: Vec<Line> = Vec::new();
        let mut vim = Vim::new();
        assert!(matches!(
            press(&mut vim, &lines, Key::Char('j')),
            Outcome::Ignored
        ));
        assert!(matches!(
            press(&mut vim, &lines, Key::Char('v')),
            Outcome::Ignored
        ));
        press(&mut vim, &lines, Key::Char('y'));
        press(&mut vim, &lines, Key::Char('y'));
        assert_eq!(vim.message, "nothing to yank");
    }

    #[test]
    fn esc_cancels_modes_and_zero_goes_home() {
        let lines = doc_lines("hello\n");
        let mut vim = Vim::new();
        press(&mut vim, &lines, Key::Char('v'));
        press(&mut vim, &lines, Key::Esc);
        assert_eq!(vim.mode, Mode::Normal);
        for key in chars("/") {
            press(&mut vim, &lines, key);
        }
        press(&mut vim, &lines, Key::Esc);
        assert_eq!(vim.mode, Mode::Normal);
        vim.cursor.col = 3;
        vim.want_col = 3;
        press(&mut vim, &lines, Key::Char('0'));
        assert_eq!(vim.cursor.col, 0);
    }

    #[test]
    fn heading_row_carries_level_and_spans() {
        let lines = doc_lines("### Hi\n");
        let RowContent::Heading { level, spans } = &lines[0].content else {
            panic!("expected heading, got {:?}", lines[0].content);
        };
        assert_eq!(*level, 3);
        assert_eq!(flatten(spans), "Hi");
        assert_eq!(lines[0].quote_depth, 0);
        assert_eq!(lines[0].list_depth, 0);
    }

    #[test]
    fn list_item_row_carries_marker_fields_and_nested_depth() {
        let lines = doc_lines("1. first\n   - nested\n2. second\n- [x] done\n");
        let RowContent::ListItem {
            ordered,
            number,
            checked,
            spans,
        } = &lines[0].content
        else {
            panic!("expected list item, got {:?}", lines[0].content);
        };
        assert!(ordered);
        assert_eq!(*number, 1);
        assert_eq!(*checked, None);
        assert_eq!(flatten(spans), "first");
        assert_eq!(lines[0].list_depth, 0);
        // The nested bullet under item 1 renders one list level deeper.
        assert!(matches!(lines[1].content, RowContent::ListItem { .. }));
        assert_eq!(lines[1].list_depth, 1);
        // A checked task item further down reports its checked state.
        let RowContent::ListItem { checked, .. } = &lines[3].content else {
            panic!("expected list item, got {:?}", lines[3].content);
        };
        assert_eq!(*checked, Some(true));
    }

    #[test]
    fn table_row_carries_cells_alignment_and_header_flag() {
        let lines = doc_lines("| a | b |\n|---|--:|\n| 1 | 2 |\n");
        let RowContent::TableRow {
            cells,
            alignments,
            header,
        } = &lines[0].content
        else {
            panic!("expected table row, got {:?}", lines[0].content);
        };
        assert!(header);
        assert_eq!(cells.len(), 2);
        assert_eq!(alignments, &vec![Align::None, Align::Right]);
        let RowContent::TableRow { header, .. } = &lines[1].content else {
            panic!("expected table row, got {:?}", lines[1].content);
        };
        assert!(!header);
    }

    #[test]
    fn nested_quote_increments_quote_depth() {
        let lines = doc_lines("> > deep\n");
        assert_eq!(lines[0].text, "> > deep");
        assert_eq!(lines[0].quote_depth, 2);
        assert!(matches!(lines[0].content, RowContent::Paragraph { .. }));
    }

    #[test]
    fn rule_and_code_rows_carry_matching_content() {
        let lines = doc_lines("---\n\n```\nx\n```\n");
        assert!(matches!(lines[0].content, RowContent::Rule));
        assert!(matches!(lines[1].content, RowContent::Code));
    }
}
