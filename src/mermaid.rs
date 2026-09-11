//! Native Mermaid subset: flowcharts and sequence diagrams.
//!
//! Supported flowchart input: `graph`/`flowchart` with TD/TB/BT/LR/RL,
//! rect/round/diamond/circle/stadium/subroutine/hexagon nodes, the
//! `-->`, `---`, `==>`, and `-.->` edges with `|label|` or `-- label --`
//! labels, chains, `;` separators, and `%%` comments. Supported sequence
//! input: `sequenceDiagram` with participant/actor declarations (including
//! `as` aliases), `->`, `-->`, `->>`, `-->>` messages, over/left/right
//! notes, and loop/alt/else/end dividers. Anything else parses to `None`
//! and the caller renders the block as code, so diagrams never corrupt.
//!
//! Layout is layered ranks for flowcharts and columns/rows for sequences.
//! All geometry is pure and unit-tested; rendering lives in `main`.

use std::collections::{HashMap, HashSet};

pub const CHAR_W: f32 = 8.0;
pub const NODE_H: f32 = 36.0;
pub const NODE_PAD_X: f32 = 12.0;
pub const NODE_MIN_W: f32 = 72.0;
pub const GAP_X: f32 = 48.0;
pub const GAP_Y: f32 = 56.0;
pub const PAD: f32 = 16.0;
pub const SEQ_COL_MIN_W: f32 = 120.0;
pub const SEQ_COL_GAP: f32 = 56.0;
pub const SEQ_TOP_H: f32 = 56.0;
pub const SEQ_ROW_H: f32 = 44.0;
pub const ARROW_LEN: f32 = 10.0;
pub const ARROW_HALF: f32 = 4.0;

#[derive(Debug, Clone)]
pub enum Diagram {
    Flow(Flowchart),
    Seq(Sequence),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowDir {
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

#[derive(Debug, Clone)]
pub struct Flowchart {
    pub direction: FlowDir,
    pub nodes: Vec<FNode>,
    pub edges: Vec<FEdge>,
}

#[derive(Debug, Clone)]
pub struct FNode {
    pub id: String,
    pub label: String,
    pub shape: Shape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Round,
    Diamond,
    Circle,
    Stadium,
    Subroutine,
    Hexagon,
}

#[derive(Debug, Clone)]
pub struct FEdge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub dashed: bool,
    pub thick: bool,
    /// Whether the target end carries an arrowhead (`-->` yes, `---` no).
    pub arrow: bool,
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub actors: Vec<Actor>,
    pub events: Vec<SEvent>,
}

#[derive(Debug, Clone)]
pub struct Actor {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub enum SEvent {
    Message {
        from: usize,
        to: usize,
        text: String,
        dotted: bool,
    },
    Note {
        actors: Vec<usize>,
        place: NotePlace,
        text: String,
    },
    Divider {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePlace {
    Over,
    Left,
    Right,
}

/// Axis-aligned rectangle in diagram pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Positioned flowchart: node boxes plus edge polylines with label anchors.
#[derive(Debug, Clone)]
pub struct FlowLayout {
    pub boxes: Vec<Rect>,
    pub edges: Vec<EdgeGeom>,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct EdgeGeom {
    /// Polyline from source border to target border.
    pub points: Vec<(f32, f32)>,
    /// Arrow tip and unit direction at the tip.
    pub tip: (f32, f32),
    pub tip_dir: (f32, f32),
    pub label: String,
    pub label_at: (f32, f32),
    pub dashed: bool,
    pub thick: bool,
    pub arrow: bool,
}

/// Positioned sequence diagram: actor columns and event rows.
#[derive(Debug, Clone)]
pub struct SeqLayout {
    pub col_x: Vec<f32>,
    pub col_w: f32,
    pub row_y: Vec<f32>,
    pub width: f32,
    pub height: f32,
}

/// Parse a fenced `mermaid` block into a diagram, or `None` for the styled
/// code fallback.
pub fn parse_diagram(text: &str) -> Option<Diagram> {
    let mut lines = text.lines();
    let head = lines
        .next()?
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();
    let rest: Vec<&str> = lines.collect();
    if let Some(direction) = parse_flow_head(&head) {
        parse_flowchart(direction, &rest).map(Diagram::Flow)
    } else if head == "sequenceDiagram" {
        parse_sequence(&rest).map(Diagram::Seq)
    } else {
        None
    }
}

fn parse_flow_head(head: &str) -> Option<FlowDir> {
    let mut words = head.split_whitespace();
    let kind = words.next()?;
    let direction = words.next().unwrap_or("TD");
    if words.next().is_some() {
        return None;
    }
    if kind != "graph" && kind != "flowchart" {
        return None;
    }
    match direction {
        "TD" | "TB" => Some(FlowDir::TopDown),
        "BT" => Some(FlowDir::BottomUp),
        "LR" => Some(FlowDir::LeftRight),
        "RL" => Some(FlowDir::RightLeft),
        _ => None,
    }
}

struct FlowBuilder {
    nodes: Vec<FNode>,
    ids: HashMap<String, usize>,
    edges: Vec<FEdge>,
}

impl FlowBuilder {
    fn node_idx(&mut self, id: &str) -> usize {
        if let Some(&index) = self.ids.get(id) {
            return index;
        }
        let index = self.nodes.len();
        self.nodes.push(FNode {
            id: id.to_string(),
            label: id.to_string(),
            shape: Shape::Rect,
        });
        self.ids.insert(id.to_string(), index);
        index
    }
}

/// Split a line into statements on `;`, honoring double quotes, after
/// stripping a `%%` comment (also quote-aware).
fn split_statements(line: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if !in_quotes && c == '%' && chars.peek() == Some(&'%') {
            break;
        } else if !in_quotes && c == ';' {
            statements.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    statements.push(current);
    statements
}

const EDGE_OPS: [&str; 5] = ["-.->", "-->", "==>", "---", "--"];

/// Byte offset and operator of the first edge operator outside quotes.
fn find_op(text: &str) -> Option<(usize, &'static str)> {
    let mut in_quotes = false;
    for (i, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            for op in EDGE_OPS {
                if text[i..].starts_with(op) {
                    return Some((i, op));
                }
            }
        }
    }
    None
}

fn parse_flowchart(direction: FlowDir, lines: &[&str]) -> Option<Flowchart> {
    let mut builder = FlowBuilder {
        nodes: Vec::new(),
        ids: HashMap::new(),
        edges: Vec::new(),
    };
    for line in lines {
        for statement in split_statements(line) {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            parse_chain(statement, &mut builder)?;
        }
    }
    if builder.nodes.is_empty() {
        return None;
    }
    Some(Flowchart {
        direction,
        nodes: builder.nodes,
        edges: builder.edges,
    })
}

/// Parse `left op [label] right [op ...]` chains, recursing on the tail.
/// A statement with no edge operator is a bare node declaration.
fn parse_chain(text: &str, builder: &mut FlowBuilder) -> Option<()> {
    let Some((pos, op)) = find_op(text) else {
        return parse_node(text, builder).map(|_| ());
    };
    let left = text[..pos].trim();
    if left.is_empty() {
        return None;
    }
    let mut rest = text[pos + op.len()..].trim_start();
    let mut label = String::new();
    if let Some(after) = rest.strip_prefix('|') {
        let end = after.find('|')?;
        label = after[..end].trim().to_string();
        rest = after[end + 1..].trim_start();
    }
    let mut edge_op = op;
    if op == "--" {
        if let Some((close_pos, closer)) = find_op(rest) {
            if closer == "--" {
                return None;
            }
            if label.is_empty() {
                label = rest[..close_pos].trim().to_string();
            } else if !rest[..close_pos].trim().is_empty() {
                return None;
            }
            edge_op = closer;
            rest = rest[close_pos + closer.len()..].trim_start();
        }
        // Otherwise bare `A -- B`: a plain edge with an empty label.
    }
    let (target, tail) = match find_op(rest) {
        Some((target_pos, _)) => (rest[..target_pos].trim(), rest[target_pos..].trim_start()),
        None => (rest.trim(), ""),
    };
    if target.is_empty() {
        return None;
    }
    let from = parse_node(left, builder)?;
    let to = parse_node(target, builder)?;
    builder.edges.push(FEdge {
        from,
        to,
        label,
        dashed: edge_op == "-.->",
        thick: edge_op == "==>",
        arrow: !matches!(edge_op, "---" | "--"),
    });
    if tail.is_empty() {
        Some(())
    } else {
        // The tail starts with another edge operator; re-parse with the
        // target as the new left side (it holds no operator by construction).
        let mut next = String::with_capacity(target.len() + 1 + tail.len());
        next.push_str(target);
        next.push(' ');
        next.push_str(tail);
        parse_chain(&next, builder)
    }
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn parse_node(spec: &str, builder: &mut FlowBuilder) -> Option<usize> {
    let spec = spec.trim();
    let id_len: usize = spec
        .char_indices()
        .take_while(|(_, c)| is_id_char(*c))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    if id_len == 0 {
        return None;
    }
    let (id, rest) = spec.split_at(id_len);
    let rest = rest.trim_start();
    let index = builder.node_idx(id);
    if rest.is_empty() {
        return Some(index);
    }
    let (shape, label) = parse_shape(rest)?;
    let node = &mut builder.nodes[index];
    node.label = label;
    node.shape = shape;
    Some(index)
}

/// Split one bracket layer: leading opener run, trailing closer run.
fn parse_shape(text: &str) -> Option<(Shape, String)> {
    let (shape, opener_len, closer) = match text.get(..2) {
        Some("((") => (Shape::Circle, 2, "))"),
        Some("([") => (Shape::Stadium, 2, "])"),
        Some("[[") => (Shape::Subroutine, 2, "]]"),
        Some("{{") => (Shape::Hexagon, 2, "}}"),
        Some("[(") => (Shape::Round, 2, ")]"),
        Some("[/") => (Shape::Rect, 2, "/]"),
        Some("[\\") => (Shape::Rect, 2, "\\]"),
        _ => match text.chars().next()? {
            '[' => (Shape::Rect, 1, "]"),
            '(' => (Shape::Round, 1, ")"),
            '{' => (Shape::Diamond, 1, "}"),
            '>' => (Shape::Rect, 1, "]"),
            _ => return None,
        },
    };
    let body = text[opener_len..].trim_end();
    let inner = body.strip_suffix(closer)?;
    let mut label = inner.trim().to_string();
    // Peel one nested pair layer for double shapes like `((x))`.
    for _ in 0..2 {
        match strip_pair(&label) {
            Some(stripped) => label = stripped,
            None => break,
        }
    }
    Some((shape, label))
}

/// Remove one surrounding bracket pair, if present.
fn strip_pair(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let closer = match first {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => return None,
    };
    if !text.ends_with(closer) || text.len() < 2 {
        return None;
    }
    Some(
        text[first.len_utf8()..text.len() - closer.len_utf8()]
            .trim()
            .to_string(),
    )
}

struct SeqBuilder {
    actors: Vec<Actor>,
    ids: HashMap<String, usize>,
    events: Vec<SEvent>,
}

impl SeqBuilder {
    fn actor_idx(&mut self, id: &str, label: Option<&str>) -> usize {
        if let Some(&index) = self.ids.get(id) {
            return index;
        }
        let index = self.actors.len();
        self.actors.push(Actor {
            id: id.to_string(),
            label: label.unwrap_or(id).to_string(),
        });
        self.ids.insert(id.to_string(), index);
        index
    }
}

const SEQ_OPS: [&str; 4] = ["-->>", "->>", "-->", "->"];

fn find_seq_op(text: &str) -> Option<(usize, &'static str)> {
    for (i, _) in text.char_indices() {
        for op in SEQ_OPS {
            if text[i..].starts_with(op) {
                return Some((i, op));
            }
        }
    }
    None
}

fn valid_actor_id(id: &str) -> bool {
    !id.is_empty() && !id.chars().any(char::is_whitespace)
}

fn parse_sequence(lines: &[&str]) -> Option<Sequence> {
    let mut builder = SeqBuilder {
        actors: Vec::new(),
        ids: HashMap::new(),
        events: Vec::new(),
    };
    for line in lines {
        let content = line.split("%%").next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        parse_seq_line(content, &mut builder)?;
    }
    if builder.actors.is_empty() && builder.events.is_empty() {
        return None;
    }
    Some(Sequence {
        actors: builder.actors,
        events: builder.events,
    })
}

fn parse_seq_line(line: &str, builder: &mut SeqBuilder) -> Option<()> {
    let keyword = line.split_whitespace().next().unwrap_or("");
    match keyword {
        "participant" | "actor" => {
            let rest = line[keyword.len()..].trim();
            let (id, label) = match rest.split_once(" as ") {
                Some((id, label)) => (id.trim(), Some(label.trim())),
                None => (rest, None),
            };
            if !valid_actor_id(id) {
                return None;
            }
            builder.actor_idx(id, label);
            Some(())
        }
        "autonumber" => Some(()),
        "activate" | "deactivate" => {
            // Lifelines stay active in this renderer; validated only.
            let id = line[keyword.len()..].trim();
            if !valid_actor_id(id) {
                return None;
            }
            builder.actor_idx(id, None);
            Some(())
        }
        "loop" | "alt" | "opt" => {
            let text = line[keyword.len()..].trim();
            if text.is_empty() {
                return None;
            }
            builder.events.push(SEvent::Divider {
                text: format!("{keyword}: {text}"),
            });
            Some(())
        }
        "else" => {
            let text = line["else".len()..].trim();
            builder.events.push(SEvent::Divider {
                text: if text.is_empty() {
                    "else".to_string()
                } else {
                    format!("else: {text}")
                },
            });
            Some(())
        }
        "end" => Some(()),
        "Note" => parse_note(line, builder),
        _ => parse_message(line, builder),
    }
}

fn parse_note(line: &str, builder: &mut SeqBuilder) -> Option<()> {
    let rest = line["Note".len()..].trim_start();
    let (place, rest) = if let Some(after) = rest.strip_prefix("over ") {
        (NotePlace::Over, after)
    } else if let Some(after) = rest.strip_prefix("left of ") {
        (NotePlace::Left, after)
    } else {
        let after = rest.strip_prefix("right of ")?;
        (NotePlace::Right, after)
    };
    let colon = rest.find(':')?;
    let (targets, text) = rest.split_at(colon);
    let mut actors = Vec::new();
    for id in targets.split(',') {
        let id = id.trim();
        if !valid_actor_id(id) {
            return None;
        }
        actors.push(builder.actor_idx(id, None));
    }
    if actors.is_empty() {
        return None;
    }
    builder.events.push(SEvent::Note {
        actors,
        place,
        text: text[1..].trim().to_string(),
    });
    Some(())
}

fn parse_message(line: &str, builder: &mut SeqBuilder) -> Option<()> {
    let (pos, op) = find_seq_op(line)?;
    let from = line[..pos].trim();
    let rest = line[pos + op.len()..].trim();
    let (to, text) = match rest.find(':') {
        Some(colon) => (rest[..colon].trim(), rest[colon + 1..].trim().to_string()),
        None => (rest, String::new()),
    };
    if !valid_actor_id(from) || !valid_actor_id(to) {
        return None;
    }
    let from = builder.actor_idx(from, None);
    let to = builder.actor_idx(to, None);
    builder.events.push(SEvent::Message {
        from,
        to,
        text,
        dotted: op.starts_with("--"),
    });
    Some(())
}

/// Width of a node from its label, in pixels.
pub fn node_width(label: &str) -> f32 {
    let chars = label.chars().count() as f32;
    (NODE_MIN_W).max(NODE_PAD_X * 2.0 + chars * CHAR_W)
}

/// Longest-path ranks on the DAG of forward edges. Back-edges (cycles such
/// as a Debug→diamond loop) are ignored so they cannot inflate ranks.
fn flow_ranks(count: usize, edges: &[FEdge]) -> Vec<usize> {
    let mut adj = vec![Vec::new(); count];
    for edge in edges {
        adj[edge.from].push(edge.to);
    }
    let back = back_edges(count, &adj);
    let mut fwd = vec![Vec::new(); count];
    let mut indeg = vec![0usize; count];
    for edge in edges {
        if back.contains(&(edge.from, edge.to)) {
            continue;
        }
        fwd[edge.from].push(edge.to);
        indeg[edge.to] += 1;
    }
    let mut rank = vec![0usize; count];
    let mut queue: Vec<usize> = (0..count).filter(|&i| indeg[i] == 0).collect();
    let mut i = 0;
    while i < queue.len() {
        let u = queue[i];
        i += 1;
        for &v in &fwd[u] {
            rank[v] = rank[v].max(rank[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    rank
}

fn back_edges(count: usize, adj: &[Vec<usize>]) -> HashSet<(usize, usize)> {
    let mut color = vec![0u8; count];
    let mut back = HashSet::new();
    fn dfs(u: usize, adj: &[Vec<usize>], color: &mut [u8], back: &mut HashSet<(usize, usize)>) {
        color[u] = 1;
        for &v in &adj[u] {
            match color[v] {
                0 => dfs(v, adj, color, back),
                1 => {
                    back.insert((u, v));
                }
                _ => {}
            }
        }
        color[u] = 2;
    }
    for i in 0..count {
        if color[i] == 0 {
            dfs(i, adj, &mut color, &mut back);
        }
    }
    back
}

fn node_box_size(node: &FNode) -> (f32, f32) {
    let mut width = node_width(&node.label);
    let mut height = NODE_H;
    match node.shape {
        Shape::Circle => {
            width = width.max(NODE_H);
        }
        Shape::Diamond => {
            width = (width * 1.4).max(148.0);
            height = NODE_H * 2.2;
        }
        _ => {}
    }
    (width, height)
}

/// Layered layout: rank by longest path from sources, centered rows.
pub fn layout_flowchart(chart: &Flowchart) -> FlowLayout {
    let count = chart.nodes.len();
    let rank = flow_ranks(count, &chart.edges);
    let depth = rank.iter().copied().max().unwrap_or(0);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); depth + 1];
    for (index, _) in chart.nodes.iter().enumerate() {
        rows[rank[index]].push(index);
    }
    let sizes: Vec<(f32, f32)> = chart.nodes.iter().map(node_box_size).collect();
    let row_width = |row: &[usize]| {
        row.iter().map(|&i| sizes[i].0).sum::<f32>() + GAP_X * row.len().saturating_sub(1) as f32
    };
    let row_heights: Vec<f32> = rows
        .iter()
        .map(|row| row.iter().map(|&i| sizes[i].1).fold(NODE_H, f32::max))
        .collect();
    let total_w = rows.iter().map(|row| row_width(row)).fold(0.0f32, f32::max);
    let total_h = row_heights.iter().sum::<f32>() + GAP_Y * depth as f32;
    let mut boxes = vec![
        Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        };
        count
    ];
    let mut y = PAD;
    for (depth_index, row) in rows.iter().enumerate() {
        let mut x = PAD + (total_w - row_width(row)) / 2.0;
        let row_h = row_heights[depth_index];
        for &index in row {
            let (w, h) = sizes[index];
            boxes[index] = Rect {
                x,
                y: y + (row_h - h) / 2.0,
                w,
                h,
            };
            x += w + GAP_X;
        }
        y += row_h + GAP_Y;
    }
    let mut layout = FlowLayout {
        boxes,
        edges: Vec::new(),
        width: total_w + PAD * 2.0,
        height: total_h + PAD * 2.0,
    };
    // Orient: BottomUp flips vertically, LeftRight/RightLeft transpose.
    match chart.direction {
        FlowDir::TopDown => {}
        FlowDir::BottomUp => {
            for rect in &mut layout.boxes {
                rect.y = layout.height - rect.y - rect.h;
            }
        }
        FlowDir::LeftRight | FlowDir::RightLeft => {
            // Swap axes so ranks run left-to-right and same-rank nodes
            // stack top-to-bottom, without rotating each node's own box
            // (labels keep their natural width and fixed height).
            for rect in &mut layout.boxes {
                std::mem::swap(&mut rect.x, &mut rect.y);
            }
            std::mem::swap(&mut layout.width, &mut layout.height);
            if chart.direction == FlowDir::RightLeft {
                for rect in &mut layout.boxes {
                    rect.x = layout.width - rect.x - rect.w;
                }
            }
        }
    }
    layout.edges = chart
        .edges
        .iter()
        .map(|edge| layout_edge(&layout.boxes[edge.from], &layout.boxes[edge.to], edge))
        .collect();
    layout
}

/// Border point of `rect` on the ray from its center toward a target.
pub fn border_point(rect: &Rect, tx: f32, ty: f32) -> (f32, f32) {
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let dx = tx - cx;
    let dy = ty - cy;
    if dx == 0.0 && dy == 0.0 {
        return (cx, cy);
    }
    let mut t = f32::INFINITY;
    if dx != 0.0 {
        t = t.min((rect.w / 2.0) / dx.abs());
    }
    if dy != 0.0 {
        t = t.min((rect.h / 2.0) / dy.abs());
    }
    (cx + dx * t, cy + dy * t)
}

fn center(rect: &Rect) -> (f32, f32) {
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

fn layout_edge(from: &Rect, to: &Rect, edge: &FEdge) -> EdgeGeom {
    let (fcx, fcy) = center(from);
    let (tcx, tcy) = center(to);
    let start = border_point(from, tcx, tcy);
    let end = border_point(to, fcx, fcy);
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let mut points = vec![start];
    if dx != 0.0 && dy != 0.0 {
        // One Manhattan bend along the dominant axis.
        if dy.abs() >= dx.abs() {
            points.push((start.0, end.1));
        } else {
            points.push((end.0, start.1));
        }
    }
    points.push(end);
    let bend = points[points.len() / 2];
    let (tip, tip_dir) = arrow_tip(&points);
    EdgeGeom {
        points,
        tip,
        tip_dir,
        label: edge.label.clone(),
        label_at: bend,
        dashed: edge.dashed,
        thick: edge.thick,
        arrow: edge.arrow,
    }
}

/// Arrow tip (last point) plus unit direction of the final segment.
pub fn arrow_tip(points: &[(f32, f32)]) -> ((f32, f32), (f32, f32)) {
    let tip = *points.last().unwrap_or(&(0.0, 0.0));
    let mut base = points.first().copied().unwrap_or(tip);
    for point in points.iter().rev().skip(1) {
        if *point != tip {
            base = *point;
            break;
        }
    }
    let dx = tip.0 - base.0;
    let dy = tip.1 - base.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        (tip, (1.0, 0.0))
    } else {
        (tip, (dx / len, dy / len))
    }
}

/// Filled triangle corners for an arrowhead at `tip` along `dir`.
pub fn arrowhead(tip: (f32, f32), dir: (f32, f32)) -> [(f32, f32); 3] {
    let base = (tip.0 - dir.0 * ARROW_LEN, tip.1 - dir.1 * ARROW_LEN);
    let perp = (-dir.1, dir.0);
    [
        tip,
        (base.0 + perp.0 * ARROW_HALF, base.1 + perp.1 * ARROW_HALF),
        (base.0 - perp.0 * ARROW_HALF, base.1 - perp.1 * ARROW_HALF),
    ]
}

/// Column/row layout for a sequence diagram.
pub fn layout_sequence(seq: &Sequence) -> SeqLayout {
    let widest = seq
        .actors
        .iter()
        .map(|actor| actor.label.chars().count() as f32 * CHAR_W + 40.0)
        .fold(0.0f32, f32::max);
    let col_w = widest.max(SEQ_COL_MIN_W);
    let col_x: Vec<f32> = (0..seq.actors.len())
        .map(|i| PAD + i as f32 * (col_w + SEQ_COL_GAP))
        .collect();
    let row_y: Vec<f32> = (0..seq.events.len())
        .map(|j| PAD + SEQ_TOP_H + SEQ_COL_GAP / 2.0 + j as f32 * SEQ_ROW_H)
        .collect();
    let width = PAD * 2.0
        + seq.actors.len() as f32 * col_w
        + seq.actors.len().saturating_sub(1) as f32 * SEQ_COL_GAP;
    let height =
        PAD + SEQ_TOP_H + SEQ_COL_GAP / 2.0 + seq.events.len() as f32 * SEQ_ROW_H + SEQ_TOP_H + PAD;
    SeqLayout {
        col_x,
        col_w,
        row_y,
        width,
        height,
    }
}

/// Closed outline polyline for canvas-drawn node shapes, first point
/// repeated at the end. Rect-like shapes return empty: they render as divs.
pub fn shape_polygon(shape: Shape, rect: &Rect) -> Vec<(f32, f32)> {
    let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    match shape {
        Shape::Diamond => vec![(cx, y), (x + w, cy), (cx, y + h), (x, cy), (cx, y)],
        Shape::Hexagon => vec![
            (x + w * 0.25, y),
            (x + w * 0.75, y),
            (x + w, cy),
            (x + w * 0.75, y + h),
            (x + w * 0.25, y + h),
            (x, cy),
            (x + w * 0.25, y),
        ],
        Shape::Circle => {
            let mut points: Vec<(f32, f32)> = (0..12)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 12.0;
                    (cx + w / 2.0 * angle.cos(), cy + h / 2.0 * angle.sin())
                })
                .collect();
            points.push((cx + w / 2.0, cy));
            points
        }
        Shape::Stadium => {
            let r = h / 2.0;
            let mut points = vec![(x + r, y), (x + w - r, y)];
            for i in 1..4 {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 4.0;
                points.push((x + w - r + r * angle.cos(), cy + r * angle.sin()));
            }
            points.push((x + w - r, y + h));
            points.push((x + r, y + h));
            for i in 1..4 {
                let angle = std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 4.0;
                points.push((x + r + r * angle.cos(), cy + r * angle.sin()));
            }
            points.push((x + r, y));
            points
        }
        Shape::Rect | Shape::Round | Shape::Subroutine => Vec::new(),
    }
}

/// Horizontal message polyline between two actor columns at one row.
/// Self-messages loop out and back.
pub fn message_points(layout: &SeqLayout, from: usize, to: usize, row: usize) -> Vec<(f32, f32)> {
    let y = layout.row_y[row] + SEQ_ROW_H / 2.0;
    let ax = layout.col_x[from] + layout.col_w / 2.0;
    let bx = layout.col_x[to] + layout.col_w / 2.0;
    if from == to {
        vec![
            (ax, y - 8.0),
            (ax + 28.0, y - 8.0),
            (ax + 28.0, y + 8.0),
            (ax, y + 8.0),
        ]
    } else {
        vec![(ax, y), (bx, y)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(text: &str) -> Flowchart {
        match parse_diagram(text) {
            Some(Diagram::Flow(chart)) => chart,
            other => panic!("expected flowchart, got {other:?}"),
        }
    }

    fn seq(text: &str) -> Sequence {
        match parse_diagram(text) {
            Some(Diagram::Seq(sequence)) => sequence,
            other => panic!("expected sequence, got {other:?}"),
        }
    }

    #[test]
    fn directions_and_head_forms() {
        assert_eq!(flow("graph TD\nA-->B\n").direction, FlowDir::TopDown);
        assert_eq!(flow("graph TB\nA-->B\n").direction, FlowDir::TopDown);
        assert_eq!(flow("graph BT\nA-->B\n").direction, FlowDir::BottomUp);
        assert_eq!(flow("graph LR\nA-->B\n").direction, FlowDir::LeftRight);
        assert_eq!(flow("graph RL\nA-->B\n").direction, FlowDir::RightLeft);
        assert_eq!(flow("flowchart LR\nA-->B\n").direction, FlowDir::LeftRight);
        assert_eq!(flow("graph\nA-->B\n").direction, FlowDir::TopDown);
        assert!(parse_diagram("graph XX\nA-->B\n").is_none());
        assert!(parse_diagram("flowchart-elk TD\nA-->B\n").is_none());
        assert!(parse_diagram("gantt\nA-->B\n").is_none());
    }

    #[test]
    fn node_shapes_and_redefinition() {
        let chart = flow(
            "graph TD\nA[rect]\nB(round)\nC{diamond}\nD((circle))\nE([stadium])\nF[[sub]]\nG{{hex}}\nH[/para/]\nI[\\alt\\]\nJ>asym]\nK[(cyl)]\nL\n",
        );
        let shapes: Vec<Shape> = chart.nodes.iter().map(|node| node.shape).collect();
        assert_eq!(
            shapes,
            vec![
                Shape::Rect,
                Shape::Round,
                Shape::Diamond,
                Shape::Circle,
                Shape::Stadium,
                Shape::Subroutine,
                Shape::Hexagon,
                Shape::Rect,
                Shape::Rect,
                Shape::Rect,
                Shape::Round,
                Shape::Rect,
            ]
        );
        assert_eq!(chart.nodes[0].label, "rect");
        assert_eq!(chart.nodes[11].label, "L");
        // Ids survive parsing: edges resolve through them.
        assert_eq!(chart.nodes[0].id, "A");
        assert_eq!(chart.nodes[11].id, "L");
        // Later definitions override the label.
        let chart = flow("graph TD\nA[first]\nA[second]\n");
        assert_eq!(chart.nodes.len(), 1);
        assert_eq!(chart.nodes[0].label, "second");
    }

    #[test]
    fn edge_forms_labels_and_chains() {
        let chart = flow(
            "graph LR\nA --> B\nC --- D\nE ==> F\nG -.-> H\nI -->|yes| J\nK -- maybe --> L\nM-->N-->O\n",
        );
        assert_eq!(chart.edges.len(), 8);
        let flags: Vec<(bool, bool, bool)> = chart
            .edges
            .iter()
            .map(|edge| (edge.dashed, edge.thick, edge.arrow))
            .collect();
        assert_eq!(
            flags,
            vec![
                (false, false, true),
                (false, false, false),
                (false, true, true),
                (true, false, true),
                (false, false, true),
                (false, false, true),
                (false, false, true),
                (false, false, true),
            ]
        );
        assert_eq!(chart.edges[4].label, "yes");
        assert_eq!(chart.edges[5].label, "maybe");
        assert_eq!(chart.nodes.len(), 15);
    }

    #[test]
    fn comments_semicolons_and_strictness() {
        let chart = flow("graph TD\n%% a comment\nA --> B; C --> D %% trailing\n");
        assert_eq!(chart.edges.len(), 2);
        assert_eq!(chart.nodes.len(), 4);
        assert!(parse_diagram("graph TD\nA -->\n").is_none());
        assert!(parse_diagram("graph TD\n???\n").is_none());
        assert!(parse_diagram("graph TD\n").is_none());
        assert!(parse_diagram("classDiagram\nA --> B\n").is_none());
    }

    #[test]
    fn chain_descends_in_top_down() {
        let chart = flow("graph TD\nA-->B-->C\n");
        let layout = layout_flowchart(&chart);
        let y: Vec<f32> = layout.boxes.iter().map(|rect| rect.y).collect();
        assert!(y[0] < y[1] && y[1] < y[2]);
        assert_eq!(layout.boxes[0].x, layout.boxes[1].x);
    }

    #[test]
    fn left_right_orders_columns_and_widths_grow() {
        let chart = flow("graph LR\nA-->B\nLong[label with more words]\n");
        let layout = layout_flowchart(&chart);
        assert!(layout.boxes[0].x < layout.boxes[1].x);
        assert!(layout.boxes[2].w > layout.boxes[0].w);
        assert!(layout.width > 0.0 && layout.height > 0.0);
    }

    #[test]
    fn bottom_up_flips_vertically() {
        let down = layout_flowchart(&flow("graph TD\nA-->B\n"));
        let up = layout_flowchart(&flow("graph BT\nA-->B\n"));
        assert!(down.boxes[0].y < down.boxes[1].y);
        assert!(up.boxes[0].y > up.boxes[1].y);
    }

    #[test]
    fn edges_stay_on_borders_and_in_bounds() {
        let chart = flow("graph TD\nStart --> Is{ok?} -->|yes| Great\nIs -->|no| Debug\n");
        let layout = layout_flowchart(&chart);
        for rect in &layout.boxes {
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.w <= layout.width);
            assert!(rect.y + rect.h <= layout.height);
        }
        for edge in &layout.edges {
            assert!(edge.points.len() >= 2);
            let first = edge.points[0];
            let last = *edge.points.last().unwrap();
            assert!(first.0 >= 0.0 && first.1 >= 0.0);
            assert!(last.0 >= 0.0 && last.1 >= 0.0);
            assert_eq!(edge.tip, last);
        }
        // Diamond branch targets share a rank below the diamond.
        assert_eq!(layout.boxes[2].y, layout.boxes[3].y);
    }

    #[test]
    fn cycle_back_edge_does_not_inflate_lr_ranks() {
        let chart = flow(
            "flowchart LR\nA[Start] --> B{Is it working?}\nB -->|Yes| C[Great!]\nB -->|No| D[Debug]\nC --> E[Deploy]\nD --> B\n",
        );
        let layout = layout_flowchart(&chart);
        // Start left of the diamond; Yes/No targets share a column; Deploy further right.
        assert!(layout.boxes[0].x < layout.boxes[1].x);
        assert!(layout.boxes[1].x < layout.boxes[2].x);
        assert!((layout.boxes[2].x - layout.boxes[3].x).abs() < 1.0);
        assert!(layout.boxes[4].x > layout.boxes[2].x);
        assert!(layout.boxes[2].y < layout.boxes[3].y);
        assert!(
            layout.width < 900.0,
            "cycle inflated width: {}",
            layout.width
        );
    }

    #[test]
    fn border_point_and_arrowhead_geometry() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 40.0,
        };
        assert_eq!(border_point(&rect, 50.0, 200.0), (50.0, 40.0));
        assert_eq!(border_point(&rect, 200.0, 20.0), (100.0, 20.0));
        let (tip, dir) = arrow_tip(&[(0.0, 0.0), (10.0, 0.0)]);
        assert_eq!(tip, (10.0, 0.0));
        assert!((dir.0 - 1.0).abs() < 1e-6 && dir.1.abs() < 1e-6);
        let corners = arrowhead((10.0, 0.0), (1.0, 0.0));
        assert_eq!(corners[0], (10.0, 0.0));
        assert_eq!(corners[1].0, corners[2].0);
        assert_eq!(corners[1].1, -corners[2].1);
    }

    #[test]
    fn sequence_actors_messages_and_notes() {
        let sequence = seq(
            "sequenceDiagram\nparticipant User\nparticipant Editor as Ed\nUser->Editor: Type\nEditor-->>Preview: Render\nNote over User,Editor: hi\nloop Keys\nUser->User: think\nend\n",
        );
        assert_eq!(sequence.actors.len(), 3);
        assert_eq!(sequence.actors[1].label, "Ed");
        assert_eq!(sequence.actors[2].id, "Preview");
        assert_eq!(sequence.events.len(), 5);
        match &sequence.events[0] {
            SEvent::Message {
                from,
                to,
                text,
                dotted,
            } => {
                assert_eq!((*from, *to), (0, 1));
                assert_eq!(text, "Type");
                assert!(!dotted);
            }
            other => panic!("expected message, got {other:?}"),
        }
        match &sequence.events[1] {
            SEvent::Message { dotted, .. } => assert!(dotted),
            other => panic!("expected message, got {other:?}"),
        }
        match &sequence.events[2] {
            SEvent::Note {
                actors,
                place,
                text,
            } => {
                assert_eq!(actors.len(), 2);
                assert_eq!(*place, NotePlace::Over);
                assert_eq!(text, "hi");
            }
            other => panic!("expected note, got {other:?}"),
        }
        assert!(matches!(sequence.events[3], SEvent::Divider { .. }));
    }

    #[test]
    fn sequence_strictness_and_aliases() {
        assert!(parse_diagram("sequenceDiagram\nUser frobnicate Editor\n").is_none());
        assert!(parse_diagram("sequenceDiagram\n-> B: x\n").is_none());
        assert!(parse_diagram("sequenceDiagram\nNote sideways A: x\n").is_none());
        assert!(parse_diagram("sequenceDiagram\nrect rgb(1,2,3)\n").is_none());
        let sequence = seq("sequenceDiagram\nactor A as Alice\nA->>B: hi\n");
        assert_eq!(sequence.actors[0].label, "Alice");
        assert_eq!(sequence.actors[1].id, "B");
    }

    #[test]
    fn shape_polygons_close_and_center() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 40.0,
        };
        let diamond = shape_polygon(Shape::Diamond, &rect);
        assert_eq!(diamond.len(), 5);
        assert_eq!(diamond[0], diamond[4]);
        assert!(diamond.contains(&(60.0, 20.0)));
        assert_eq!(shape_polygon(Shape::Hexagon, &rect).len(), 7);
        let circle = shape_polygon(Shape::Circle, &rect);
        assert_eq!(circle.len(), 13);
        assert_eq!(circle[0], circle[12]);
        let stadium = shape_polygon(Shape::Stadium, &rect);
        assert_eq!(stadium[0], stadium[stadium.len() - 1]);
        assert!(shape_polygon(Shape::Rect, &rect).is_empty());
        assert!(shape_polygon(Shape::Round, &rect).is_empty());
    }

    #[test]
    fn sequence_layout_columns_and_rows() {
        let sequence = seq("sequenceDiagram\nA->B: one\nB->A: two\nA->A: self\n");
        let layout = layout_sequence(&sequence);
        assert_eq!(layout.col_x.len(), 2);
        assert_eq!(layout.row_y.len(), 3);
        assert!(layout.col_x[0] < layout.col_x[1]);
        assert!(layout.row_y[0] < layout.row_y[1]);
        let across = message_points(&layout, 0, 1, 0);
        assert!(across[1].0 > across[0].0);
        let back = message_points(&layout, 1, 0, 1);
        assert!(back[1].0 < back[0].0);
        assert_eq!(message_points(&layout, 0, 0, 2).len(), 4);
        assert!(layout.width > 0.0 && layout.height > 0.0);
    }
}
