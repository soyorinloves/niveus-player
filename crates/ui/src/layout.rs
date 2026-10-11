//! Responsive layout: which regions exist, and where.
//!
//! A pure function of the terminal rectangle, so the app can ask for the same
//! answer when it needs to size a scrolling list without duplicating the rules.

use niveus_config::CoverSize;
use ratatui::layout::{Constraint, Layout, Rect};

/// How much room the terminal has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutTier {
    /// Not enough room to draw anything useful.
    TooSmall,
    /// Navigation, one list, and playback controls in the status bar. The
    /// now-playing pane is suppressed.
    Minimal,
    /// Both panes side by side.
    Standard,
    /// Both panes, with room for more of everything.
    Wide,
}

impl LayoutTier {
    /// Below this, nothing is drawn but the size warning.
    pub const MIN_WIDTH: u16 = 80;
    pub const MIN_HEIGHT: u16 = 24;
    /// At or above this width the now-playing pane joins the two the main
    /// interface already has. The figure rose from 120 when the interface went
    /// from two columns to three: at 120 the secondary column, the main area
    /// and the pane all want space and all three end up cramped (SPEC §22).
    pub const STANDARD_WIDTH: u16 = 130;
    /// At or above this width the layout is considered roomy.
    pub const WIDE_WIDTH: u16 = 160;
    /// Below this height the two-pane layout is refused regardless of width.
    pub const FULL_HEIGHT: u16 = 32;

    /// Width and height together decide the tier.
    ///
    /// Height is checked first: a wide but short terminal is still cramped.
    pub fn for_size(width: u16, height: u16) -> Self {
        if width < Self::MIN_WIDTH || height < Self::MIN_HEIGHT {
            return Self::TooSmall;
        }
        if height < Self::FULL_HEIGHT || width < Self::STANDARD_WIDTH {
            return Self::Minimal;
        }
        if width < Self::WIDE_WIDTH {
            Self::Standard
        } else {
            Self::Wide
        }
    }

    /// Whether the now-playing pane is drawn at all.
    pub fn shows_now_playing(self) -> bool {
        matches!(self, LayoutTier::Standard | LayoutTier::Wide)
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutTier::TooSmall => "TooSmall",
            LayoutTier::Minimal => "Minimal",
            LayoutTier::Standard => "Standard",
            LayoutTier::Wide => "Wide",
        }
    }
}

/// The width of the secondary column.
///
/// Fixed rather than proportioned: it holds one short word per item plus the
/// focus marker, and a column that grew with the terminal would trade rows of
/// track titles for blank space beside a category name (SPEC §10.2).
pub const SECONDARY_WIDTH: u16 = 14;

/// Where each part of the interface goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    pub tier: LayoutTier,
    /// The whole terminal.
    pub area: Rect,
    /// First-level navigation row.
    pub nav: Rect,
    /// Rule beneath the navigation row.
    pub nav_rule: Rect,
    /// Everything below the navigation: the columns and the status bar.
    pub body: Rect,
    /// The secondary column: one item per category.
    pub secondary: Rect,
    /// The `│` between the secondary column and the main area.
    pub secondary_rule: Rect,
    /// The main area: browsing, and editing within SETTINGS.
    pub main: Rect,
    /// Column of `│` between the main area and the now-playing pane.
    pub pane_rule: Option<Rect>,
    /// The now-playing pane, absent at narrow sizes.
    pub now_playing: Option<Rect>,
    /// Where album art goes inside that pane, when there is room for any.
    pub artwork: Option<Rect>,
    /// The lyrics / metadata panel inside that pane.
    ///
    /// The app needs its height to size the lyric window; that is why it is
    /// here rather than left to the renderer.
    pub panel: Option<Rect>,
    /// Rule above the status bar.
    pub status_rule: Rect,
    pub status: Rect,
}

impl Regions {
    /// Rows available for list content in the main area.
    ///
    /// Its own title and the rule beneath it take two rows.
    pub fn main_viewport(&self) -> usize {
        self.main.height.saturating_sub(2) as usize
    }
}

/// How tall the album art region is inside a now-playing pane of this height.
///
/// Shared rather than decided in the renderer alone, because the app has to ask
/// for a cover at the right size before there is anything to draw: asking for
/// one size and drawing at another would either waste the download or leave the
/// picture resampled twice.
///
/// There is no absolute cap on the height — how tall a cover may be is decided
/// by how tall the pane is. The only limit is the one the pane's own furniture
/// and the lyrics panel impose.
pub fn artwork_rows(pane_height: u16, size: CoverSize) -> u16 {
    // What the pane can spare once its own furniture and three lines of the
    // lyrics panel are paid for. In any pane the application actually draws
    // this is roomy enough that the named proportions below are what decides;
    // it is here so that a pane squeezed to nothing does not quietly take the
    // panel's lines away instead.
    let spare = pane_height.saturating_sub(ARTWORK_FIXED_ROWS + PANEL_ROWS);

    let wanted = match size {
        CoverSize::Small => pane_height / 4,
        CoverSize::Normal => pane_height / 3,
        // Everything left over.
        CoverSize::Large => spare,
    };
    wanted.min(spare)
}

/// The rows the pane spends on everything that is not the cover or the panel:
/// the heading, its rule, the three lines of track identity, the rule under
/// them, the section labels and the rule and progress bar below.
const ARTWORK_FIXED_ROWS: u16 = 9;

/// How many rows the metadata block takes when it is showing.
///
/// One per field, and the design asks for five of them (SPEC §14).
pub const METADATA_ROWS: u16 = 5;

/// The lines the panel is always left.
///
/// A cover that ate into these would leave the panel unreadable, and the space
/// is shared: the metadata block fills the bottom when it is on (SPEC §12.1),
/// and the lyric window takes what is left. Reserving the block's own height
/// means the cover does not change size when the switch is flipped.
const PANEL_ROWS: u16 = METADATA_ROWS;

/// The rows the pane's own furniture takes above the artwork: the heading and
/// the rule under it.
const PANE_HEADER_ROWS: u16 = 2;

/// The rows the now-playing pane is made of, top to bottom.
///
/// One function answers this for both the renderer and the app. The app has to
/// know how tall the panel is before it can ask for a lyric window of the right
/// size, and the renderer has to draw in exactly those rows — working them out
/// twice is how a renderer and a scroll window drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneRows {
    pub title: Rect,
    pub title_rule: Rect,
    pub artwork: Option<Rect>,
    pub identity: Rect,
    pub identity_rule: Rect,
    pub section: Rect,
    pub panel: Rect,
    pub panel_rule: Rect,
    pub progress: Rect,
}

/// Divides a now-playing pane into its rows.
pub fn pane_rows(pane: Rect, cover_size: CoverSize) -> PaneRows {
    let art_rows = artwork_rows(pane.height, cover_size);
    let [title, title_rule, _artwork, identity, identity_rule, section, panel, panel_rule, progress] =
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(art_rows),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(pane);

    PaneRows {
        title,
        title_rule,
        // A band with no room in it is not a place to draw a cover.
        artwork: (art_rows > 0 && pane.height >= PANE_HEADER_ROWS + art_rows)
            .then(|| Rect::new(pane.x, pane.y + PANE_HEADER_ROWS, pane.width, art_rows)),
        identity,
        identity_rule,
        section,
        panel,
        panel_rule,
        progress,
    }
}

/// How the panel is divided: the lyrics, the rule under them, and the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelBlocks {
    /// The room the lyric window has.
    pub lyrics: Rect,
    /// The rule between the lyrics and the block, when both are showing.
    pub rule: Option<Rect>,
    /// The metadata block, when it is showing.
    pub metadata: Option<Rect>,
}

/// Splits the panel between the lyric window and the metadata block.
///
/// The block is pinned to the bottom when it is showing and the lyrics take
/// what is above it, with a rule between the two — the same rule as everywhere
/// else in the interface, since it is doing the same job (SPEC §10, §12.1).
///
/// The rule is part of the block's cost, not decoration around it: the lyrics
/// are sized from what is left after both. And a rule separates two things, so
/// with no room above the block for anything to separate it is not drawn — that
/// is the largest cover, where the block is the whole panel.
pub fn panel_blocks(panel: Rect, metadata: bool) -> PanelBlocks {
    if !metadata {
        return PanelBlocks {
            lyrics: panel,
            rule: None,
            metadata: None,
        };
    }

    let block_rows = METADATA_ROWS.min(panel.height);
    let block_top = panel.y + panel.height - block_rows;
    let above = block_top - panel.y;

    let rule = (above >= 2).then(|| Rect {
        y: block_top - 1,
        height: 1,
        ..panel
    });
    let lyrics = Rect {
        height: above - u16::from(rule.is_some()),
        ..panel
    };

    PanelBlocks {
        lyrics,
        rule,
        metadata: Some(Rect {
            y: block_top,
            height: block_rows,
            ..panel
        }),
    }
}

/// Divides the terminal into regions for `tier`.
///
/// A fixed secondary column on the left, then the main area; at the roomier
/// tiers the now-playing pane takes the right 45% of what remains, with a
/// single column of rule between each pair.
pub fn compute(area: Rect, tier: LayoutTier, cover_size: CoverSize) -> Regions {
    if tier == LayoutTier::TooSmall {
        // Nothing is drawn but the warning, so every region collapses.
        let empty = Rect::new(area.x, area.y, area.width, 0);
        return Regions {
            tier,
            area,
            nav: empty,
            nav_rule: empty,
            body: area,
            secondary: empty,
            secondary_rule: empty,
            main: area,
            pane_rule: None,
            now_playing: None,
            artwork: None,
            panel: None,
            status_rule: empty,
            status: empty,
        };
    }

    let [nav, nav_rule, body, status_rule, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // The secondary column is the same width at every tier: it is the spine of
    // the navigation, and narrowing it would only clip a category name (§22).
    let [secondary, secondary_rule, rest] = Layout::horizontal([
        Constraint::Length(SECONDARY_WIDTH),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(body);

    let (main, pane_rule, now_playing) = if tier.shows_now_playing() {
        let [main, pane_rule, right] = Layout::horizontal([
            Constraint::Fill(55),
            Constraint::Length(1),
            Constraint::Fill(45),
        ])
        .areas(rest);
        (main, Some(pane_rule), Some(right))
    } else {
        (rest, None, None)
    };

    // Both come from the one description of what a pane is made of, so the app
    // asking for a cover or a lyric window of a size gets the size that will
    // actually be drawn.
    let (artwork, panel) = match now_playing {
        Some(pane) => {
            let rows = pane_rows(pane, cover_size);
            (rows.artwork, Some(rows.panel))
        }
        None => (None, None),
    };

    Regions {
        tier,
        area,
        nav,
        nav_rule,
        body,
        secondary,
        secondary_rule,
        main,
        pane_rule,
        now_playing,
        artwork,
        panel,
        status_rule,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(w: u16, h: u16) -> LayoutTier {
        LayoutTier::for_size(w, h)
    }

    #[test]
    fn small_terminals_are_refused() {
        assert_eq!(tier(79, 40), LayoutTier::TooSmall);
        assert_eq!(tier(120, 23), LayoutTier::TooSmall);
        assert_eq!(tier(79, 23), LayoutTier::TooSmall);
    }

    #[test]
    fn height_beats_width_when_downgrading() {
        // Wide enough for two panes, but too short to want them.
        assert_eq!(tier(200, 24), LayoutTier::Minimal);
        assert_eq!(tier(200, 31), LayoutTier::Minimal);
        assert_eq!(tier(200, 32), LayoutTier::Wide);
    }

    #[test]
    fn the_documented_boundaries_hold() {
        assert_eq!(tier(80, 32), LayoutTier::Minimal);
        assert_eq!(tier(129, 32), LayoutTier::Minimal);
        assert_eq!(tier(130, 32), LayoutTier::Standard);
        assert_eq!(tier(159, 32), LayoutTier::Standard);
        assert_eq!(tier(160, 32), LayoutTier::Wide);
    }

    #[test]
    fn only_the_roomy_tiers_show_the_third_column() {
        assert!(!tier(80, 24).shows_now_playing());
        assert!(!tier(129, 32).shows_now_playing());
        assert!(tier(130, 32).shows_now_playing());
        assert!(tier(200, 50).shows_now_playing());
    }

    #[test]
    fn a_normal_terminal_splits_into_the_expected_rows() {
        let regions = compute(
            Rect::new(0, 0, 120, 40),
            LayoutTier::Standard,
            CoverSize::Normal,
        );

        assert_eq!(regions.nav.height, 1);
        assert_eq!(regions.nav_rule.height, 1);
        assert_eq!(regions.status.height, 1);
        assert_eq!(regions.status_rule.height, 1);
        // nav + rule + body + rule + status
        assert_eq!(regions.body.height, 40 - 4);

        let right = regions.now_playing.expect("standard shows the pane");
        assert_eq!(regions.pane_rule.expect("a rule splits the panes").width, 1);
        // The secondary column and its rule come off first, then 55:45 of what
        // is left.
        assert_eq!(regions.secondary.width, SECONDARY_WIDTH);
        assert_eq!(regions.secondary_rule.width, 1);
        assert!(regions.main.width > right.width);
        assert_eq!(
            regions.main.width + 1 + right.width,
            120 - SECONDARY_WIDTH - 1
        );
    }

    #[test]
    fn the_minimal_tier_keeps_the_secondary_column_and_the_main_area() {
        let regions = compute(
            Rect::new(0, 0, 100, 30),
            LayoutTier::Minimal,
            CoverSize::Normal,
        );
        assert!(regions.now_playing.is_none());
        assert!(regions.pane_rule.is_none());
        // The column is kept at every tier, only the third one goes (§22).
        assert_eq!(regions.secondary.width, SECONDARY_WIDTH);
        assert_eq!(regions.main.width, 100 - SECONDARY_WIDTH - 1);
    }

    #[test]
    fn the_main_viewport_excludes_its_title_and_rule() {
        let regions = compute(
            Rect::new(0, 0, 120, 40),
            LayoutTier::Standard,
            CoverSize::Normal,
        );
        assert_eq!(regions.main_viewport(), regions.main.height as usize - 2);
    }

    #[test]
    fn a_short_pane_does_not_give_the_largest_cover_less_than_the_middle_one() {
        // A pane too short for the largest setting must not end up with a
        // smaller cover than the middle setting would have given it.
        for pane in 0..60u16 {
            let large = artwork_rows(pane, CoverSize::Large);
            let normal = artwork_rows(pane, CoverSize::Normal);
            assert!(large >= normal, "{large} < {normal} in a {pane}-row pane");
        }
    }

    #[test]
    fn the_named_proportions_hold_in_a_pane_the_application_draws() {
        // The pane only exists in the two roomy tiers: a 32-row terminal gives
        // it 28 rows.
        for pane in 24..80u16 {
            assert_eq!(
                artwork_rows(pane, CoverSize::Small),
                pane / 4,
                "pane {pane}"
            );
            assert_eq!(
                artwork_rows(pane, CoverSize::Normal),
                pane / 3,
                "pane {pane}"
            );
            assert_eq!(
                artwork_rows(pane, CoverSize::Large),
                pane - ARTWORK_FIXED_ROWS - PANEL_ROWS,
                "pane {pane}"
            );
        }
    }

    #[test]
    fn the_cover_size_decides_how_much_of_the_pane_is_art() {
        let pane = 30;
        let small = artwork_rows(pane, CoverSize::Small);
        let normal = artwork_rows(pane, CoverSize::Normal);
        let large = artwork_rows(pane, CoverSize::Large);

        assert!(small < normal, "{small} is not smaller than {normal}");
        assert!(normal < large, "{normal} is not smaller than {large}");
        assert_eq!(small, pane / 4);
        assert_eq!(normal, pane / 3);
        // Everything the pane can spare once its own furniture and three lines
        // of the panel are paid for.
        assert_eq!(large, pane - ARTWORK_FIXED_ROWS - PANEL_ROWS);
    }

    #[test]
    fn no_setting_ever_takes_the_panels_lines_from_it() {
        // There is deliberately no fixed cap on the cover; what this pins down
        // is the other end — that a bigger cover is never paid for out of the
        // lyrics panel, whatever the pane's height.
        for pane in 0..80u16 {
            for size in CoverSize::ALL {
                let rows = artwork_rows(pane, size);
                assert!(
                    rows + ARTWORK_FIXED_ROWS + PANEL_ROWS <= pane
                        || rows <= pane.saturating_sub(ARTWORK_FIXED_ROWS),
                    "{size:?} asked for {rows} rows of a {pane}-row pane"
                );
                assert!(
                    rows <= pane.saturating_sub(ARTWORK_FIXED_ROWS + PANEL_ROWS),
                    "{size:?} left the panel short in a {pane}-row pane"
                );
            }
        }
    }

    #[test]
    fn the_largest_cover_still_leaves_the_track_its_own_lines() {
        // The drawing itself has to agree: the piece left for the panel is at
        // least the three lines the design asks the lyrics to show.
        for pane_height in [24u16, 30, 40, 60] {
            let rows = artwork_rows(pane_height, CoverSize::Large);
            let left = pane_height - ARTWORK_FIXED_ROWS - rows;
            assert!(
                left >= PANEL_ROWS,
                "{left} rows left in a {pane_height}-row pane"
            );
        }
    }

    // ---- the panel --------------------------------------------------------

    fn panel_rect(height: u16) -> Rect {
        Rect::new(0, 10, 50, height)
    }

    #[test]
    fn the_metadata_block_is_pinned_to_the_bottom_of_the_panel() {
        let blocks = panel_blocks(panel_rect(15), true);
        let block = blocks.metadata.expect("the block is showing");
        assert_eq!(block.height, METADATA_ROWS);
        assert_eq!(
            block.y + block.height,
            10 + 15,
            "the block is not at the bottom"
        );
    }

    #[test]
    fn the_rule_above_the_block_comes_out_of_the_lyrics_room() {
        // The block costs six rows when it is showing: itself and the rule.
        let blocks = panel_blocks(panel_rect(15), true);
        let rule = blocks.rule.expect("both are showing, so there is a rule");
        let block = blocks.metadata.expect("the block is showing");

        assert_eq!(blocks.lyrics.height, 15 - METADATA_ROWS - 1);
        assert_eq!(
            rule.y + 1,
            block.y,
            "the rule is not directly above the block"
        );
        assert_eq!(blocks.lyrics.y + blocks.lyrics.height, rule.y);
    }

    #[test]
    fn no_rule_is_drawn_when_there_is_nothing_above_it_to_separate() {
        // The largest cover leaves a panel exactly the height of the block: it
        // fills the panel and the lyrics are gone (SPEC §12.1).
        let blocks = panel_blocks(panel_rect(METADATA_ROWS), true);
        assert!(
            blocks.rule.is_none(),
            "a rule was drawn with nothing above it"
        );
        assert_eq!(blocks.lyrics.height, 0);
        assert_eq!(blocks.metadata.expect("showing").height, METADATA_ROWS);

        // And one row short of that: a rule would take the only row a lyric
        // could have used, so the rule gives way instead.
        let blocks = panel_blocks(panel_rect(METADATA_ROWS + 1), true);
        assert!(blocks.rule.is_none());
        assert_eq!(blocks.lyrics.height, 1);
        assert_eq!(blocks.lyrics.y + 1, blocks.metadata.expect("showing").y);
    }

    #[test]
    fn nothing_is_split_off_while_the_block_is_off() {
        let blocks = panel_blocks(panel_rect(15), false);
        assert_eq!(
            blocks.lyrics,
            panel_rect(15),
            "the lyrics lost room to nothing"
        );
        assert!(blocks.rule.is_none());
        assert!(blocks.metadata.is_none());
    }

    #[test]
    fn a_panel_squeezed_shorter_than_the_block_still_divides_sanely() {
        // Defensive: the pane is never drawn this small, but the arithmetic
        // must not underflow if it ever is.
        for height in 0..=METADATA_ROWS {
            let blocks = panel_blocks(panel_rect(height), true);
            let block = blocks.metadata.expect("showing");
            assert!(block.y + block.height <= 10 + height);
            assert_eq!(
                blocks.lyrics.height + u16::from(blocks.rule.is_some()) + block.height,
                height
            );
        }
    }

    #[test]
    fn a_too_small_terminal_has_no_usable_regions() {
        let regions = compute(
            Rect::new(0, 0, 40, 10),
            LayoutTier::TooSmall,
            CoverSize::Normal,
        );
        assert_eq!(regions.nav.height, 0);
        assert_eq!(regions.status.height, 0);
        assert!(regions.now_playing.is_none());
    }
}
