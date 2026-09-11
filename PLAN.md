## Goal

Build `md-view`, a Rust desktop Markdown viewer for Linux/Wayland: open one
Markdown file, render dark-mode GFM, auto-reload on save, navigate with
vim-like motions plus VISUAL selection, copy (no editing), highlight code
(JSON/CSV/TOML/INI plus scripting/programming languages), and draw Mermaid
flowcharts/sequences natively with GPUI (no external renderer).

## Success Criteria

- `md-view file.md` opens a GPUI window and renders headings, lists, tables,
  task lists, code, links, blockquotes, and tag pills matching the dark
  reference PDF.
- Saving the watched file re-renders within ~500ms, preserves scroll where
  possible, and survives atomic-save replace/rename.
- Normal mode (`j/k/gg/G/Ctrl-d/u`, `/` + `n/N`) and VISUAL char/line modes
  work; `y` copies to the Wayland clipboard.
- Code blocks highlight with the Neovim `tokyo_custom` mapping by default.
- `mermaid` flowchart and sequence subsets render as native dark diagrams;
  unsupported diagrams fall back to a styled code block, never an error.
- No Node.js/`mmdc` or network required at runtime.

## Context And Current Facts

- Visual fixture (use this to check that the viewer renders Markdown as
  expected; the PDF is a multi-page export of the same file, not a
  requirement that the app paginate): `example.md` is the source covering
  YAML front matter, GFM, fenced code, `$$` math, mermaid flowchart +
  sequence, task lists, comparison tables, mark/underline/sub/sup, nested
  lists, links, and blockquotes. `example-rendered.pdf` is the 4-page dark
  GitHub-style reference of that file (frontmatter table, tag pills, H2
  rules, code chrome, LaTeX-style math, diagrams, tasks, tables, formatting).
  Open with `md-view example.md` and compare the window to those four pages.
- Workspace also has `README.md` (viewer, vim bindings, re-render on change,
  mermaid, highlighting), `demo.md`, and the Cargo/GPUI project under `src/`.
- Neovim theme is `tokyo_custom` at
  `~/.config/nvim/lua/colors/tokyo_custom.lua`: bg `#1a1b26`, fg `#c0caf5`,
  comment `#565f89` italic, string `#9ece6a`, const/number `#ff9e64`,
  func `#eb6f92` italic, keyword `#9d7cd8` italic, type `#2ac3de`,
  operator `#89ddff`.
- Prior user decisions: GPUI UI, single watched file, native-GUI image-capable
  path replaced by native GPUI diagram drawing, Wayland-only target.

## Constraints And Non-goals

- Rust only; GPUI pinned by `Cargo.lock` git rev (pre-1.0, breaking changes
  expected).
- Linux/Wayland only for v1; no X11/macOS/Windows testing.
- View + copy only; no editing, sync-scroll editor, export, sharing, LaTeX,
  PlantUML, or full Mermaid grammar in v1.
- No external diagram binary and no vendored JS engine in v1.

## Key Decisions

- UI: GPUI + `gpui_platform` with `wayland` feature. It is a hybrid
  immediate/retained GPU framework for Rust, pre-1.0, and documents the
  `wayland` backend feature explicitly. Alternatives (egui/iced/Tauri)
  rejected per explicit user choice.
- Markdown: `comrak` AST to GPUI elements. It is a CommonMark + GFM
  compatible parser/renderer in Rust. Chosen over hand-rolled parsing;
  HTML rendering path not used.
- Highlighting: `syntect` with a handwritten theme built from
  `tokyo_custom.lua`, not a bundled Sublime theme. `syntect` is a Rust
  highlighting library using Sublime syntax definitions, which covers the
  required data + scripting languages from one engine.
- Watching: `notify` + `notify-debouncer-mini` (~200-500ms). It is the
  cross-platform notification library with mini/full debouncers; mini fits
  single-file atomic saves.
- Mermaid: hand parser for `graph/flowchart` + `sequenceDiagram` subsets to
  GPUI boxes/arrows/lifelines; all other diagram kinds fall back to code
  blocks. Rejected `mmdc`/webview per user constraint.

## Recommended Approach

Single GPUI app: CLI path -> read file -> `comrak` AST -> layout model
(blocks + inline spans + code tokens + diagram scenes) -> GPUI render.
Background watcher thread debounces saves and posts reloads; scroll anchor
kept by heading/bullet identity. One `theme.rs` holds `ui.*`/`markdown.*`
from the PDF and `code.*` from `tokyo_custom`. Vim state machine sits above
the scroll/selection layer; yank goes through GPUI clipboard.

## Work Plan

1. Scaffold binary + GPUI Wayland window: `Cargo.toml` (pinned GPUI +
   `gpui_platform/wayland`, `comrak`, `syntect`, `notify`,
   `notify-debouncer-mini`), `src/main.rs`, `src/theme.rs`,
   `src/markdown.rs`. Depends on: none.
2. Markdown pipeline: frontmatter table, headings, lists, task lists,
   tables (incl. multi-row headers), blockquotes, links, mark/kbd, pills.
   Depends on: 1.
3. Theme pass against PDF pages 1-4: backgrounds, borders, H2 rules,
   pills, code-block chrome, table striping. Depends on: 2.
4. Watch + reload: debounced watcher, atomic-save handling, missing-file
   banner, scroll retention. Depends on: 1.
5. Vim + copy: normal motions, VISUAL char/line, `/` search, `y` yank via
   GPUI clipboard on Wayland. Depends on: 2, 4.
6. Code highlight: `syntect` SyntaxSet load, language by fence tag
   (json/csv/toml/ini/rust/bash/python/etc.), custom `tokyo_custom`
   theme, line rendering in code chrome. Depends on: 2, 3.
7. Native Mermaid: flowchart node/edge + sequence actor/message layout
   with dark fills/strokes; fallback block otherwise. Depends on: 2, 3.

## Validation Plan

- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- Unit: comrak AST mapping (tables, task lists, fallback), mermaid subset
  parser (flowchart + sequence golden scenes), theme token mapping from
  `tokyo_custom` hex values.
- Visual: `md-view example.md` against `example-rendered.pdf` pages 1-4
  (frontmatter table, tag pills, headings, code, math, mermaid, tasks,
  tables, formatting). The PDF is paginated; the viewer should scroll one
  document, not paginate.
- Manual on Wayland: open `example.md` covering all 4 PDF pages; edit-save
  shows update preserving position; delete/replace shows banner then
  recovers; `j/gg/G///y` in normal/VISUAL verified by pasting into another
  app; unsupported mermaid shows code fallback.
- Highest risk: GPUI git-rev build + Wayland window launch; validate in
  unit 1 before deeper work.

## Risks / Rollback

- GPUI pre-1.0 breakage pins progress: pin rev in `Cargo.lock`, keep
  UI layer thin, record rev in README.
- Native Mermaid subset will mis-render exotic syntax: strict fallback to
  code block, log unhandled kind.
- Large-file layout cost: cap diagram/code work per reload, keep reload
  off the UI thread.
- Rollback: each work unit is additive; theme/mermaid/highlight can ship
  disabled behind flags.

## Open Questions

None.

## Implementation Notes And Knowledge Gaps

These details were not verified against pinned APIs during planning and must
be closed at implementation time against the pinned rev/version docs and
examples before writing the corresponding code:

- GPUI pinned-rev API: app entry, `Render`/layout/text-run usage, key and
  action bindings, clipboard access, and scroll/selection handling, plus the
  required stable Rust. Close at work unit 1: pin the git rev in `Cargo.lock`,
  then read that rev's `crates/gpui` README and examples. Partial evidence so
  far only covers the framework choice and `wayland` feature.
- `comrak` pinned-version details: GFM option flags and AST node shapes for
  frontmatter, tables (including multi-row headers), task lists, and inline
  spans. Close at work unit 2 before building the layout model.
- `syntect` pinned-version details: building a custom theme from the
  `tokyo_custom` hex values (instead of a bundled theme), loading the
  syntax set, and mapping fence tags to syntaxes. Close at work unit 6 with
  a small highlight probe plus golden tests.
- `notify`/`notify-debouncer-mini` pinned-version details: debouncer
  constructor, debounce interval behavior, and event kinds for atomic-save
  replace/rename plus delete. Close at work unit 4 with save, replace, and
  delete manual checks.
- Toolchain floor: GPUI wants latest stable while `comrak` carries its own
  MSRV. Lock the floor when the manifest is written and record the GPUI rev
  and Rust version in the README.

## Sources

- https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md
- https://github.com/kivikakk/comrak
- https://github.com/trishume/syntect
- https://github.com/notify-rs/notify
