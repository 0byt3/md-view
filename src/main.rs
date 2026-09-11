//! `md-view`: single-file Markdown viewer.
//!
//! Opens a GPUI window on Wayland showing the watched file, re-parses on
//! every debounced change, navigates with vim motions plus VISUAL selection,
//! and yanks to the Wayland clipboard. Code blocks get syntect token colors;
//! every other block (headings, lists, tables, quotes, marks, links, front
//! matter) gets its own rich rendering below.

mod highlight;
mod markdown;
mod math;
mod mermaid;
mod theme;
mod vim;
mod watch;

use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, AnyElement, App, Bounds, ClipboardItem, Context,
    Font, FontFallbacks, FontWeight, Keystroke, PathBuilder, ScrollHandle, SharedString,
    StrikethroughStyle, StyledText, TextRun, UnderlineStyle, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;
use std::path::PathBuf;
use std::time::Duration;

/// Debounce for file events; with the 100ms UI poll this re-renders a save
/// in well under the ~500ms budget.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// How often the UI task drains the watch channel.
const POLL: Duration = Duration::from_millis(100);
/// Ctrl-d/u stride used only before the scrollable view has painted once
/// (so `ScrollHandle` cannot yet report a visible row count).
const HALF_PAGE_FALLBACK: usize = 20;
/// Indent added per nested list level, in pixels.
const LIST_INDENT: f32 = 16.0;
/// Indent added per nested blockquote level, in pixels.
const QUOTE_INDENT: f32 = 12.0;

/// Document sans with color-emoji fallback so GFM emoji in `example.md` render.
fn ui_font() -> Font {
    Font {
        family: SharedString::from("Noto Sans"),
        fallbacks: Some(FontFallbacks::from_fonts(vec![
            "Noto Color Emoji".to_string(),
            "Noto Sans Symbols 2".to_string(),
        ])),
        ..Font::default()
    }
}

/// A vector stroke: polyline points, whether it's dashed, whether it's thick.
type Stroke = (Vec<(f32, f32)>, bool, bool);
/// Filled triangle corners for one arrowhead.
type ArrowHead = [(f32, f32); 3];

struct Viewer {
    path_display: SharedString,
    path: PathBuf,
    doc: markdown::Document,
    lines: Vec<vim::Line>,
    code: std::collections::HashMap<usize, Vec<Vec<highlight::Token>>>,
    diagrams: std::collections::HashMap<usize, mermaid::Diagram>,
    vim: vim::Vim,
    missing: bool,
    rx: std::sync::mpsc::Receiver<watch::WatchEvent>,
    _watch: Option<watch::FileWatch>,
    /// Tracks the scroll offset of the document body so the cursor row can
    /// be kept in view and `Ctrl-d/u` can measure the real viewport.
    scroll: ScrollHandle,
}

/// Map a platform keystroke to a vim key. Anything with modifiers other
/// than a bare Ctrl-d/u/Ctrl-c is ignored.
fn map_keystroke(stroke: &Keystroke) -> Option<vim::Key> {
    if stroke.modifiers.control || stroke.modifiers.alt || stroke.modifiers.platform {
        if stroke.modifiers.control
            && !stroke.modifiers.alt
            && !stroke.modifiers.platform
            && !stroke.modifiers.shift
        {
            return match stroke.key.as_str() {
                "d" => Some(vim::Key::Ctrl('d')),
                "u" => Some(vim::Key::Ctrl('u')),
                "c" => Some(vim::Key::Esc),
                _ => None,
            };
        }
        return None;
    }
    match stroke.key.as_str() {
        "escape" => Some(vim::Key::Esc),
        "enter" => Some(vim::Key::Enter),
        "backspace" => Some(vim::Key::Backspace),
        "up" => Some(vim::Key::Up),
        "down" => Some(vim::Key::Down),
        " " | "space" => Some(vim::Key::Char(' ')),
        key if key.chars().count() == 1 => key.chars().next().map(vim::Key::Char),
        _ => None,
    }
}

impl Viewer {
    /// Drain pending watch events; true when the view must re-render.
    fn poll_once(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                watch::WatchEvent::Modified => {
                    let old_key = self
                        .lines
                        .get(self.vim.cursor.line)
                        .map(|line| line.key.clone());
                    self.doc = markdown::parse_markdown(&markdown::load_file(&self.path));
                    self.lines = vim::build_lines(&self.doc);
                    self.code = highlight::highlight_document(&self.doc);
                    self.diagrams = diagram_cache(&self.doc);
                    self.vim.rebase(old_key, &self.lines);
                    self.missing = false;
                    changed = true;
                }
                watch::WatchEvent::Gone => {
                    self.missing = true;
                    changed = true;
                }
            }
        }
        changed
    }

    fn banner(&self) -> String {
        if self.missing {
            format!("missing: {}", self.path_display)
        } else {
            String::new()
        }
    }

    fn status(&self) -> String {
        if self.vim.message.is_empty() {
            self.vim.status()
        } else {
            format!("{}  {}", self.vim.status(), self.vim.message)
        }
    }

    /// Ctrl-d/u stride: half of the rows currently visible in the scroll
    /// view, or [`HALF_PAGE_FALLBACK`] before the first paint has happened.
    fn half_page(&self) -> usize {
        let top = self.scroll.top_item();
        let bottom = self.scroll.bottom_item();
        let visible = bottom.saturating_sub(top);
        if visible == 0 {
            HALF_PAGE_FALLBACK
        } else {
            (visible / 2).max(1)
        }
    }
}

/// Parsed diagrams by block index. Mermaid blocks that fail to parse stay
/// out of this map and render as styled code instead.
fn diagram_cache(doc: &markdown::Document) -> std::collections::HashMap<usize, mermaid::Diagram> {
    doc.blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block {
            markdown::Block::Code { language, text } if language == "mermaid" => {
                mermaid::parse_diagram(text).map(|diagram| (index, diagram))
            }
            _ => None,
        })
        .collect()
}

/// Paint vector strokes and filled arrowheads on a canvas that fills its
/// relatively-positioned container.
///
/// `window.paint_path` takes window-absolute coordinates, but `strokes`/
/// `heads` are laid out in the diagram's own local space (0,0 at its top
/// left). The canvas paint callback's first argument is the element's
/// actual on-screen bounds, so every point must be offset by `bounds.origin`
/// -- otherwise every diagram paints its edges pinned to the window's
/// top-left corner instead of wherever the diagram actually scrolled to.
fn paint_strokes(
    fills: Vec<Vec<(f32, f32)>>,
    strokes: Vec<Stroke>,
    heads: Vec<ArrowHead>,
) -> impl IntoElement {
    let line = rgb(theme::DIAGRAM_LINE);
    let fill = rgb(theme::DIAGRAM_FILL);
    canvas(
        move |_, _, _| {},
        move |bounds, _, window, _| {
            let origin = bounds.origin;
            for points in &fills {
                if points.len() < 3 {
                    continue;
                }
                let mut builder = PathBuilder::fill();
                builder.move_to(origin + point(px(points[0].0), px(points[0].1)));
                for p in points.iter().skip(1) {
                    builder.line_to(origin + point(px(p.0), px(p.1)));
                }
                builder.close();
                if let Ok(path) = builder.build() {
                    window.paint_path(path, fill);
                }
            }
            for (points, dashed, thick) in &strokes {
                let mut builder = PathBuilder::stroke(px(if *thick { 3.0 } else { 2.0 }));
                if *dashed {
                    builder = builder.dash_array(&[px(6.0), px(4.0)]);
                }
                for (i, p) in points.iter().enumerate() {
                    let at = origin + point(px(p.0), px(p.1));
                    if i == 0 {
                        builder.move_to(at);
                    } else {
                        builder.line_to(at);
                    }
                }
                if let Ok(path) = builder.build() {
                    window.paint_path(path, line);
                }
            }
            for corners in &heads {
                let mut builder = PathBuilder::fill();
                builder.move_to(origin + point(px(corners[0].0), px(corners[0].1)));
                builder.line_to(origin + point(px(corners[1].0), px(corners[1].1)));
                builder.line_to(origin + point(px(corners[2].0), px(corners[2].1)));
                builder.close();
                if let Ok(path) = builder.build() {
                    window.paint_path(path, line);
                }
            }
        },
    )
    .absolute()
    .left(px(0.0))
    .top(px(0.0))
    .size_full()
}

impl Viewer {
    fn render_flow(&self, chart: &mermaid::Flowchart, selected: bool) -> AnyElement {
        let layout = mermaid::layout_flowchart(chart);
        let line = rgb(theme::DIAGRAM_LINE);
        let fill = rgb(theme::DIAGRAM_FILL);
        let strokes: Vec<Stroke> = layout
            .edges
            .iter()
            .map(|edge| (edge.points.clone(), edge.dashed, edge.thick))
            .collect();
        let mut outlines = strokes;
        for (index, node) in chart.nodes.iter().enumerate() {
            let poly = mermaid::shape_polygon(node.shape, &layout.boxes[index]);
            if !poly.is_empty() {
                outlines.push((poly, false, false));
            }
        }
        let heads: Vec<ArrowHead> = layout
            .edges
            .iter()
            .filter(|edge| edge.arrow)
            .map(|edge| mermaid::arrowhead(edge.tip, edge.tip_dir))
            .collect();
        let fills: Vec<Vec<(f32, f32)>> = chart
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let poly = mermaid::shape_polygon(node.shape, &layout.boxes[index]);
                if poly.is_empty() {
                    None
                } else {
                    Some(poly)
                }
            })
            .collect();
        let mut container = div()
            .relative()
            .w(px(layout.width))
            .h(px(layout.height))
            .child(paint_strokes(fills, outlines, heads));
        for (index, node) in chart.nodes.iter().enumerate() {
            let rect = &layout.boxes[index];
            let label = div()
                .flex()
                .justify_center()
                .items_center()
                .text_color(rgb(theme::BODY))
                .child(node.label.clone());
            // `.id(..)` gives each node a stable identity across re-renders,
            // keyed by its mermaid id rather than its position in the list.
            let placed = div()
                .id(SharedString::from(format!("node-{}", node.id)))
                .absolute()
                .left(px(rect.x))
                .top(px(rect.y))
                .w(px(rect.w))
                .h(px(rect.h));
            let node_el = match node.shape {
                mermaid::Shape::Round => placed
                    .border_1()
                    .border_color(line)
                    .bg(fill)
                    .rounded_md()
                    .child(label),
                mermaid::Shape::Rect | mermaid::Shape::Subroutine => {
                    placed.border_1().border_color(line).bg(fill).child(label)
                }
                _ => placed.child(label),
            };
            container = container.child(node_el);
        }
        for edge in &layout.edges {
            if edge.label.is_empty() {
                continue;
            }
            container = container.child(
                div()
                    .absolute()
                    .left(px(edge.label_at.0))
                    .top(px(edge.label_at.1 - 10.0))
                    .text_color(rgb(theme::BODY))
                    .child(edge.label.clone()),
            );
        }
        if selected {
            container = container.bg(rgb(theme::BG_SELECTION));
        }
        container.into_any_element()
    }

    fn render_sequence(&self, seq: &mermaid::Sequence, selected: bool) -> AnyElement {
        let layout = mermaid::layout_sequence(seq);
        let line = rgb(theme::DIAGRAM_LINE);
        let box_top = mermaid::PAD + 10.0;
        let box_h = 36.0;
        let lifeline_top = box_top + box_h;
        let lifeline_bottom = layout.height - mermaid::PAD - box_h;
        let mut strokes: Vec<Stroke> = seq
            .actors
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let x = layout.col_x[i] + layout.col_w / 2.0;
                (vec![(x, lifeline_top), (x, lifeline_bottom)], false, false)
            })
            .collect();
        let mut heads = Vec::new();
        for (row, event) in seq.events.iter().enumerate() {
            if let mermaid::SEvent::Message { from, to, .. } = event {
                let points = mermaid::message_points(&layout, *from, *to, row);
                let (tip, dir) = mermaid::arrow_tip(&points);
                let dotted = matches!(event, mermaid::SEvent::Message { dotted: true, .. });
                strokes.push((points, dotted, false));
                heads.push(mermaid::arrowhead(tip, dir));
            }
        }
        let mut container = div()
            .relative()
            .w(px(layout.width))
            .h(px(layout.height))
            .child(paint_strokes(Vec::new(), strokes, heads));
        for (i, actor) in seq.actors.iter().enumerate() {
            // Each actor gets two boxes (top/bottom); suffix keeps ids unique.
            for (place, top) in [("top", box_top), ("bottom", lifeline_bottom)] {
                container = container.child(
                    div()
                        .id(SharedString::from(format!("actor-{}-{place}", actor.id)))
                        .absolute()
                        .left(px(layout.col_x[i]))
                        .top(px(top))
                        .w(px(layout.col_w))
                        .h(px(box_h))
                        .border_1()
                        .border_color(line)
                        .bg(rgb(theme::DIAGRAM_FILL))
                        .flex()
                        .justify_center()
                        .items_center()
                        .text_color(rgb(theme::BODY))
                        .child(actor.label.clone()),
                );
            }
        }
        for (row, event) in seq.events.iter().enumerate() {
            let y = layout.row_y[row];
            match event {
                mermaid::SEvent::Message { from, to, text, .. } => {
                    if text.is_empty() {
                        continue;
                    }
                    let ax = layout.col_x[*from] + layout.col_w / 2.0;
                    let bx = layout.col_x[*to] + layout.col_w / 2.0;
                    container = container.child(
                        div()
                            .absolute()
                            .left(px(ax.min(bx) + 6.0))
                            .top(px(y + 2.0))
                            .text_color(rgb(theme::BODY))
                            .child(text.clone()),
                    );
                }
                mermaid::SEvent::Note {
                    actors,
                    place,
                    text,
                } => {
                    let first = layout.col_x[actors[0]];
                    let last = layout.col_x[*actors.last().unwrap()] + layout.col_w;
                    let (left, width) = match place {
                        mermaid::NotePlace::Over => (first, last - first),
                        mermaid::NotePlace::Left => ((first + last) / 2.0 - 168.0, 160.0),
                        mermaid::NotePlace::Right => ((first + last) / 2.0 + 8.0, 160.0),
                    };
                    container = container.child(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(y + 6.0))
                            .w(px(width))
                            .h(px(mermaid::SEQ_ROW_H - 12.0))
                            .border_1()
                            .border_color(line)
                            .bg(rgb(theme::BLOCK_BG))
                            .flex()
                            .justify_center()
                            .items_center()
                            .text_color(rgb(theme::BODY))
                            .child(text.clone()),
                    );
                }
                mermaid::SEvent::Divider { text } => {
                    container = container.child(
                        div()
                            .absolute()
                            .left(px(mermaid::PAD))
                            .top(px(y + 10.0))
                            .w(px(layout.width - mermaid::PAD * 2.0))
                            .flex()
                            .justify_center()
                            .text_color(rgb(theme::BODY))
                            .child(text.clone()),
                    );
                }
            }
        }
        if selected {
            container = container.bg(rgb(theme::BG_SELECTION));
        }
        container.into_any_element()
    }

    fn render_diagram(&self, block: usize, selected: bool) -> AnyElement {
        match &self.diagrams[&block] {
            mermaid::Diagram::Flow(chart) => self.render_flow(chart, selected),
            mermaid::Diagram::Seq(seq) => self.render_sequence(seq, selected),
        }
    }
}

/// Rich-text rendering: converting parsed markdown structure into styled
/// GPUI elements. Kept separate from the vector-diagram rendering above.
impl Viewer {
    /// Build one flowing, wrapped text element with per-run styling (bold,
    /// italic, strikethrough, links, `==mark==`, `++insert++`, inline code)
    /// from a document's style-preserving [`markdown::Span`]s.
    fn build_styled_text(&self, spans: &[markdown::Span]) -> StyledText {
        let mut text = String::new();
        let mut runs = Vec::with_capacity(spans.len());
        for span in spans {
            let start = text.len();
            text.push_str(&span.text);
            let len = text.len() - start;

            let mut font = ui_font();
            if span.bold {
                font = font.bold();
            }
            if span.italic {
                font = font.italic();
            }

            let mut color = rgb(theme::BODY).into();
            let mut background_color = None;
            let mut underline = None;
            let mut strikethrough = None;
            if span.code {
                background_color = Some(rgb(theme::BLOCK_BG).into());
            }
            if span.link {
                // GitHub-style: colored, not underlined by default.
                color = rgb(theme::LINK).into();
            }
            if span.insert {
                underline = Some(UnderlineStyle {
                    thickness: px(1.0),
                    ..Default::default()
                });
            }
            if span.mark {
                color = rgb(theme::MARK_FG).into();
                background_color = Some(rgb(theme::MARK_BG).into());
            }
            if span.strike {
                strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.0),
                    ..Default::default()
                });
            }
            runs.push(TextRun {
                len,
                font,
                color,
                background_color,
                underline,
                strikethrough,
            });
        }
        StyledText::new(text).with_runs(runs)
    }

    fn render_heading(&self, level: u8, spans: &[markdown::Inline]) -> AnyElement {
        let mut el = div()
            .mt_4()
            .mb_2()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(theme::BODY))
            .child(self.build_styled_text(&markdown::spans(spans)));
        el = match level {
            1 => el.text_3xl(),
            2 => el.text_2xl(),
            3 => el.text_xl(),
            _ => el.text_lg(),
        };
        if level <= 2 {
            el = el.pb_1().border_b_1().border_color(rgb(theme::BORDER));
        }
        el.into_any_element()
    }

    fn render_paragraph(&self, spans: &[markdown::Inline]) -> AnyElement {
        self.render_inline_flow(spans, true)
    }

    /// Flowing inlines: styled text, with display math broken out and centered
    /// like the GitHub-style reference PDF.
    fn render_inline_flow(&self, inlines: &[markdown::Inline], padded: bool) -> AnyElement {
        if inlines
            .iter()
            .all(|inline| !matches!(inline, markdown::Inline::Math { .. }))
        {
            return div()
                .when(padded, |el| el.mb_4())
                .child(self.build_styled_text(&markdown::spans(inlines)))
                .into_any_element();
        }
        let mut column = div().flex().flex_col().when(padded, |el| el.mb_4());
        let mut buf: Vec<markdown::Inline> = Vec::new();
        for inline in inlines {
            match inline {
                markdown::Inline::Math { display: true, tex } => {
                    if !buf.is_empty() {
                        column = column.child(self.render_inline_row(&std::mem::take(&mut buf)));
                    }
                    column = column.child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .py_3()
                            .child(self.render_math(tex, true)),
                    );
                }
                other => buf.push(other.clone()),
            }
        }
        if !buf.is_empty() {
            column = column.child(self.render_inline_row(&buf));
        }
        column.into_any_element()
    }

    fn render_inline_row(&self, inlines: &[markdown::Inline]) -> AnyElement {
        let mut row = div().flex().flex_row().flex_wrap().items_center().gap_1();
        let mut text_buf: Vec<markdown::Inline> = Vec::new();
        for inline in inlines {
            if let markdown::Inline::Math { display, tex } = inline {
                if !text_buf.is_empty() {
                    row = row.child(self.build_styled_text(&markdown::spans(&text_buf)));
                    text_buf.clear();
                }
                row = row.child(self.render_math(tex, *display));
            } else {
                text_buf.push(inline.clone());
            }
        }
        if !text_buf.is_empty() {
            row = row.child(self.build_styled_text(&markdown::spans(&text_buf)));
        }
        row.into_any_element()
    }

    fn render_math(&self, tex: &str, display: bool) -> AnyElement {
        let atom = math::parse_tex(tex);
        let inner = self.render_atom(&atom, display);
        if display {
            div().text_lg().child(inner).into_any_element()
        } else {
            inner
        }
    }

    fn render_atom(&self, atom: &math::Atom, display: bool) -> AnyElement {
        let color = rgb(theme::BODY);
        match atom {
            math::Atom::Text(text) => div()
                .text_color(color)
                .child(text.clone())
                .into_any_element(),
            math::Atom::Row(items) => {
                let mut row = div().flex().flex_row().items_center().gap_1();
                for item in items {
                    row = row.child(self.render_atom(item, display));
                }
                row.into_any_element()
            }
            math::Atom::Frac(num, den) => div()
                .flex()
                .flex_col()
                .items_center()
                .px_1()
                .child(self.render_atom(num, display))
                .child(div().h(px(1.0)).w_full().bg(color).my(px(2.0)))
                .child(self.render_atom(den, display))
                .into_any_element(),
            math::Atom::Scripts {
                base,
                sub,
                sup,
                limits,
            } => {
                if *limits && display {
                    let mut col = div().flex().flex_col().items_center().px_1();
                    if let Some(sup) = sup {
                        col = col.child(div().text_sm().child(self.render_atom(sup, false)));
                    }
                    col = col.child(self.render_atom(base, display));
                    if let Some(sub) = sub {
                        col = col.child(div().text_sm().child(self.render_atom(sub, false)));
                    }
                    col.into_any_element()
                } else {
                    let mut scripts = div().flex().flex_col().items_center().text_sm().ml_1();
                    if let Some(sup) = sup {
                        scripts = scripts.child(self.render_atom(sup, false));
                    }
                    if let Some(sub) = sub {
                        scripts = scripts.child(self.render_atom(sub, false));
                    }
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .child(self.render_atom(base, display))
                        .child(scripts)
                        .into_any_element()
                }
            }
        }
    }

    fn render_list_item(
        &self,
        ordered: bool,
        number: usize,
        checked: Option<bool>,
        spans: &[markdown::Inline],
        depth: usize,
        is_last: bool,
    ) -> AnyElement {
        let marker: AnyElement = match checked {
            Some(true) => div()
                .text_color(rgb(theme::GREEN))
                .child("\u{2611}")
                .into_any_element(),
            Some(false) => div()
                .text_color(rgb(theme::FG_GUTTER))
                .child("\u{2610}")
                .into_any_element(),
            None if ordered => div()
                .text_color(rgb(theme::FG_GUTTER))
                .child(format!("{number}."))
                .into_any_element(),
            None => {
                let mark = match depth {
                    0 => "\u{2022}",
                    1 => "\u{25E6}",
                    _ => "\u{25AA}",
                };
                div()
                    .text_color(rgb(theme::FG_GUTTER))
                    .child(mark)
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_2()
            .pl(px(LIST_INDENT * depth as f32))
            .mb_1()
            .when(is_last, |el| el.mb_4())
            .child(marker)
            .child(div().child(self.build_styled_text(&markdown::spans(spans))))
            .into_any_element()
    }

    fn render_table_row(
        &self,
        cells: &[Vec<markdown::Inline>],
        alignments: &[markdown::Align],
        header: bool,
        row_in_block: usize,
        is_last: bool,
    ) -> AnyElement {
        let mut row = div().flex().flex_row().when(is_last, |el| el.mb_4());
        if header {
            row = row.bg(rgb(theme::BLOCK_BG));
        } else if row_in_block.is_multiple_of(2) {
            // row_in_block 0 is always the header; even body rows tint.
            row = row.bg(rgb(theme::ROW_ALT_BG));
        }
        for (i, cell) in cells.iter().enumerate() {
            let align = alignments.get(i).copied().unwrap_or(markdown::Align::None);
            let mut cell_el = div()
                .flex_1()
                .border_1()
                .border_color(rgb(theme::BORDER))
                .px_2()
                .py_1()
                .child(self.build_styled_text(&markdown::spans(cell)));
            cell_el = match align {
                markdown::Align::Left | markdown::Align::None => cell_el.text_left(),
                markdown::Align::Center => cell_el.text_center(),
                markdown::Align::Right => cell_el.text_right(),
            };
            if header {
                cell_el = cell_el.font_weight(FontWeight::BOLD);
            }
            row = row.child(cell_el);
        }
        row.into_any_element()
    }

    fn render_rule(&self) -> AnyElement {
        div()
            .h(px(1.0))
            .w_full()
            .bg(rgb(theme::BORDER))
            .my_4()
            .into_any_element()
    }

    /// One document row: highlighted token spans for code lines, or the
    /// rich element matching its [`vim::RowContent`] otherwise.
    fn render_line(&self, index: usize, line: &vim::Line) -> AnyElement {
        let mut row_in_block = 0;
        for candidate in self.lines.iter().take(index) {
            if candidate.block == line.block {
                row_in_block += 1;
            }
        }
        let is_last_in_block = self.lines.get(index + 1).map(|l| l.block) != Some(line.block);
        let content = match &line.content {
            vim::RowContent::Code => {
                self.render_code_row(line, row_in_block, row_in_block == 0, is_last_in_block)
            }
            vim::RowContent::Heading { level, spans } => self.render_heading(*level, spans),
            vim::RowContent::Paragraph { spans } => self.render_paragraph(spans),
            vim::RowContent::ListItem {
                ordered,
                number,
                checked,
                spans,
            } => self.render_list_item(
                *ordered,
                *number,
                *checked,
                spans,
                line.list_depth,
                is_last_in_block,
            ),
            vim::RowContent::TableRow {
                cells,
                alignments,
                header,
            } => self.render_table_row(cells, alignments, *header, row_in_block, is_last_in_block),
            vim::RowContent::Rule => self.render_rule(),
        };
        let content = wrap_quote(line.quote_depth, content);
        if index == self.vim.cursor.line {
            div()
                .bg(rgb(theme::BG_SELECTION))
                .child(content)
                .into_any_element()
        } else {
            content
        }
    }

    /// Code rows keep the syntect token colors; anything without cached
    /// tokens (e.g. an empty fence) falls back to its flattened text. All
    /// lines belonging to the same fence share one background so the fence
    /// reads as a single bordered block, matching GitHub's `<pre>` chrome.
    fn render_code_row(
        &self,
        line: &vim::Line,
        row_in_block: usize,
        is_first: bool,
        is_last: bool,
    ) -> AnyElement {
        let mut row = div()
            .flex()
            .flex_row()
            .bg(rgb(theme::BLOCK_BG))
            .px_3()
            .when(is_first, |el| el.pt_2().mt_2().rounded_t_md())
            .when(is_last, |el| el.pb_2().mb_4().rounded_b_md());
        let mut spanned = false;
        if let Some(groups) = self.code.get(&line.block) {
            if let Some(tokens) = groups.get(row_in_block) {
                for token in tokens {
                    let span = div().text_color(rgb(token.color)).child(token.text.clone());
                    row = row.child(if token.italic { span.italic() } else { span });
                }
                spanned = true;
            }
        }
        if !spanned {
            row = row.child(line.text.clone());
        }
        row.into_any_element()
    }

    fn pill(&self, text: &str) -> AnyElement {
        div()
            .px_2()
            .rounded_full()
            .border_1()
            .border_color(rgb(theme::LINK))
            .bg(rgb(theme::BLOCK_BG))
            .text_color(rgb(theme::LINK))
            .text_sm()
            .child(text.to_string())
            .into_any_element()
    }

    /// Front matter renders as a static key/value panel above the
    /// scrollable body; it is metadata, not a navigable document block. A
    /// `tags` key (case-insensitive, comma-separated) renders as pills.
    fn render_front_matter(&self) -> Option<AnyElement> {
        if self.doc.front_matter.is_empty() {
            return None;
        }
        let last = self.doc.front_matter.len() - 1;
        let mut table = div()
            .flex()
            .flex_col()
            .mb_4()
            .border_1()
            .border_color(rgb(theme::BORDER))
            .rounded_md()
            .bg(rgb(theme::BLOCK_BG));
        for (i, (key, value)) in self.doc.front_matter.iter().enumerate() {
            let key_el = div()
                .w(px(120.0))
                .px_3()
                .py_2()
                .border_r_1()
                .border_color(rgb(theme::BORDER))
                .flex()
                .justify_end()
                .text_color(rgb(theme::FG_GUTTER))
                .child(key.clone());
            let value_el: AnyElement = if key.eq_ignore_ascii_case("tags") {
                let mut pills = div().flex().flex_row().flex_wrap().gap_2().px_3().py_2();
                for tag in markdown::tag_list(value) {
                    pills = pills.child(self.pill(&tag));
                }
                pills.into_any_element()
            } else {
                div()
                    .flex_1()
                    .px_3()
                    .py_2()
                    .text_color(rgb(theme::BODY))
                    .child(value.clone())
                    .into_any_element()
            };
            let mut row = div().flex().flex_row().items_stretch();
            if i != last {
                row = row.border_b_1().border_color(rgb(theme::BORDER));
            }
            table = table.child(row.child(key_el).child(value_el));
        }
        Some(table.into_any_element())
    }
}

/// Wrap a row in a left border/indent per nested `>` level. Depth 0 (not in
/// a quote) returns the content untouched.
fn wrap_quote(depth: usize, content: AnyElement) -> AnyElement {
    if depth == 0 {
        return content;
    }
    div()
        .border_l_2()
        .border_color(rgb(theme::BORDER))
        .pl_3()
        .ml(px(QUOTE_INDENT * (depth as f32 - 1.0)))
        .child(content)
        .into_any_element()
}

impl Render for Viewer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut row_for_line: Vec<usize> = Vec::with_capacity(self.lines.len());
        let mut diagram_block: Option<usize> = None;
        let mut diagram_row_ix = 0;
        for (index, line) in self.lines.iter().enumerate() {
            if self.diagrams.contains_key(&line.block) {
                if diagram_block == Some(line.block) {
                    row_for_line.push(diagram_row_ix);
                    continue;
                }
                diagram_block = Some(line.block);
                diagram_row_ix = rows.len();
                let selected = self
                    .lines
                    .get(self.vim.cursor.line)
                    .map(|cursor| cursor.block)
                    == Some(line.block);
                rows.push(self.render_diagram(line.block, selected));
                row_for_line.push(diagram_row_ix);
            } else {
                diagram_block = None;
                row_for_line.push(rows.len());
                rows.push(self.render_line(index, line));
            }
        }
        // Keep the cursor's row scrolled into view on every render (motion,
        // yank, or a file reload all end up here via `cx.notify()`).
        if let Some(&row_ix) = row_for_line.get(self.vim.cursor.line) {
            self.scroll.scroll_to_item(row_ix);
        }

        let doc_body = div()
            .id("doc")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .bg(rgb(theme::PAGE_BG))
            .p_4()
            .text_color(rgb(theme::BODY))
            .children(rows);

        div()
            .flex()
            .flex_col()
            .font_family("Noto Sans")
            .bg(rgb(theme::BG))
            .size_full()
            .child(
                div()
                    .px_4()
                    .pt_2()
                    .text_lg()
                    .text_color(rgb(theme::FG_GUTTER))
                    .child(self.banner()),
            )
            .child(
                div()
                    .px_4()
                    .text_color(rgb(theme::FG_GUTTER))
                    .child(self.path_display.clone()),
            )
            .children(self.render_front_matter())
            .child(doc_body)
            .child(
                div()
                    .px_4()
                    .py_1()
                    .text_color(rgb(theme::FG_GUTTER))
                    .child(self.status()),
            )
    }
}

fn main() {
    let path_arg = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: md-view <file.md>");
        std::process::exit(2);
    });
    let path = PathBuf::from(&path_arg);
    let missing = !path.exists();
    let doc = markdown::parse_markdown(&markdown::load_file(&path));
    let lines = vim::build_lines(&doc);
    let code = highlight::highlight_document(&doc);
    let diagrams = diagram_cache(&doc);

    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = match watch::watch_file(&path, DEBOUNCE, tx) {
        Ok(watcher) => Some(watcher),
        Err(err) => {
            eprintln!("md-view: watching disabled: {err}");
            None
        }
    };

    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.), px(700.)), cx);
        let view = cx.new(|_| Viewer {
            path_display: path_arg.clone().into(),
            path,
            doc,
            lines,
            code,
            diagrams,
            vim: vim::Vim::new(),
            missing,
            rx,
            _watch: watcher,
            scroll: ScrollHandle::new(),
        });
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                {
                    let view = view.clone();
                    move |_, _| view.clone()
                },
            )
            .unwrap();
        cx.observe_keystrokes(move |event, _, cx| {
            view.update(cx, |view, cx| {
                if let Some(key) = map_keystroke(&event.keystroke) {
                    let half = view.half_page();
                    match view.vim.handle_key(&view.lines, key, half) {
                        vim::Outcome::Changed => cx.notify(),
                        vim::Outcome::Ignored => {}
                        vim::Outcome::Yanked(text) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            cx.notify();
                        }
                    }
                }
            })
        })
        .detach();
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(POLL).await;
            let alive = window.update(cx, |view, _, cx| {
                if view.poll_once() {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        cx.activate(true);
    });
}
