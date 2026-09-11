//! Dark theme values for `md-view`.
//!
//! `ui` / document chrome follows `example-rendered.pdf` (dark GitHub-style
//! reference of `example.md`); code token colors follow the Neovim
//! `tokyo_custom` palette in `~/.config/nvim/lua/colors/tokyo_custom.lua`.

/// Window background (`tokyo_custom` `bg`).
pub const BG: u32 = 0x1a1b26;
/// Primary text (`tokyo_custom` `fg`).
pub const FG: u32 = 0xc0caf5;
/// Muted text and gutters (`tokyo_custom` `fg_gutter`).
pub const FG_GUTTER: u32 = 0x3b4261;
/// Comments (`tokyo_custom` `comment`).
pub const COMMENT: u32 = 0x565f89;
/// Strings (`tokyo_custom` light green).
pub const STRING: u32 = 0x9ece6a;
/// Constants and numbers (`tokyo_custom` orange).
pub const CONSTANT: u32 = 0xff9e64;
/// Functions (`tokyo_custom` love).
pub const FUNCTION: u32 = 0xeb6f92;
/// Keywords (`tokyo_custom` purple).
pub const KEYWORD: u32 = 0x9d7cd8;
/// Types (`tokyo_custom` aqua).
pub const TYPE: u32 = 0x2ac3de;
/// Operators and punctuation (`tokyo_custom` turquoise).
pub const OPERATOR: u32 = 0x89ddff;
/// Links and headings accent (`tokyo_custom` blue).
pub const BLUE: u32 = 0x7aa2f7;
/// Visual selection background (`tokyo_custom` `bg_selection`).
pub const BG_SELECTION: u32 = 0x283457;

// ---------------------------------------------------------------------------
// Document chrome sampled from `example-rendered.pdf` (150 DPI render).
// These govern page, block, table, link, highlight, and diagram colors;
// code token colors above stay on the Neovim palette by design.
// ---------------------------------------------------------------------------

/// Document canvas: page background (`#0D1117` sampled).
pub const PAGE_BG: u32 = 0x0D1117;
/// Code blocks, table cells, tag pills (`#161B22` sampled).
pub const BLOCK_BG: u32 = 0x161B22;
/// Alternate table row (`#1C2128` sampled zebra stripe).
pub const ROW_ALT_BG: u32 = 0x1C2128;
/// Borders and rules (`#30363D` sampled).
pub const BORDER: u32 = 0x30363D;
/// Body and heading text (`#C9D1D9` sampled).
pub const BODY: u32 = 0xC9D1D9;
/// Links and tag pills (`#58A6FF` sampled).
pub const LINK: u32 = 0x58A6FF;
/// Tags and section headers (`tokyo_custom` magenta).
pub const MAGENTA: u32 = 0xbb9af7;
/// Member and property access (`tokyo_custom` green).
pub const GREEN: u32 = 0x73daca;
/// `==mark==` background (`#FFF8C5` sampled) and its dark text (`#24292F`).
pub const MARK_BG: u32 = 0xFFF8C5;
pub const MARK_FG: u32 = 0x24292F;
/// Native diagram node fill (`#1F2020`) and edges (`#CCCCCC` sampled).
pub const DIAGRAM_FILL: u32 = 0x1F2020;
pub const DIAGRAM_LINE: u32 = 0xCCCCCC;

#[cfg(test)]
mod tests {
    use super::*;

    /// Approximate relative luminance used to pin the chrome contrast order.
    /// Only ever consumed by these tests.
    fn luminance(rgb: u32) -> f32 {
        let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    #[test]
    fn chrome_stays_readable() {
        assert_ne!(BG, FG);
        assert_ne!(BG, BG_SELECTION);
        assert_ne!(FG, FG_GUTTER);
    }

    #[test]
    fn chrome_contrast_ordering() {
        assert!(luminance(BODY) > luminance(BLOCK_BG));
        assert!(luminance(BLOCK_BG) > luminance(PAGE_BG));
        assert!(luminance(ROW_ALT_BG) > luminance(PAGE_BG));
        assert!(luminance(LINK) > luminance(PAGE_BG));
        assert!(luminance(MARK_BG) > luminance(MARK_FG));
        assert!(luminance(DIAGRAM_LINE) > luminance(DIAGRAM_FILL));
    }

    #[test]
    fn chrome_entries_stay_distinct() {
        let entries = [
            PAGE_BG,
            BLOCK_BG,
            ROW_ALT_BG,
            BORDER,
            BODY,
            LINK,
            MARK_BG,
            MARK_FG,
            DIAGRAM_FILL,
            DIAGRAM_LINE,
        ];
        for (i, a) in entries.iter().enumerate() {
            for b in &entries[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn palette_entries_stay_distinct() {
        // Touch every token color so two roles can never silently merge.
        let entries = [
            BG,
            FG,
            FG_GUTTER,
            COMMENT,
            STRING,
            CONSTANT,
            FUNCTION,
            KEYWORD,
            TYPE,
            OPERATOR,
            BLUE,
            MAGENTA,
            GREEN,
            BG_SELECTION,
        ];
        for (i, a) in entries.iter().enumerate() {
            for b in &entries[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
