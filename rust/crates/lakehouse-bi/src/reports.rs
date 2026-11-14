//! A report: a written page of text blocks with live charts between them
//! (`BI-18` part D; feature page `docs/core/features/reports-and-templates.md`,
//! decisions R1 to R4; plan
//! `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md`, T1).
//!
//! This module is the pure part: the block shapes, the limits and the fixed
//! sentence for each refusal. The store (`lakehouse_store::dashboard_reports`)
//! keeps the blocks and the routes (`lakehouse-api`) decide who may read and
//! write them.
//!
//! # Text is text (R1)
//!
//! A text block holds plain structured text: a heading, a paragraph, the items
//! of a list, a link's words and address. Nothing here parses markup, and no
//! block carries HTML: the console renders every string as text, so `<script>`
//! typed into a paragraph is shown as those eight characters. A link's address
//! must be `https://`, `http://`, `mailto:` or a path on this console
//! (`/...`); anything else (`javascript:`, `data:`) is refused, so a link
//! cannot run code when someone clicks it.
//!
//! # A chart is a reference (R1)
//!
//! A chart block names a chart that already exists on a dashboard, never its
//! data or its definition. Every reader reads the chart as themselves (masks,
//! row filters, the dashboard's access), so the report stays live and shows
//! each person only what they may see. `board` is filled by the server from
//! the chart's own row when the report is saved; what a client sends there is
//! ignored.
//!
//! # Limits
//!
//! [`MAX_CHARTS`] is decision R4. The other limits bound what one save may
//! store and what one read may draw; they are not product decisions and say
//! so in their sentences.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most chart blocks a report may hold (decision R4).
pub const MAX_CHARTS: usize = 30;

/// The most blocks of any kind a report may hold.
pub const MAX_BLOCKS: usize = 300;

/// The longest report title, in characters.
pub const MAX_TITLE_CHARS: usize = 200;

/// The longest text of one heading, paragraph, list item or link, in
/// characters.
pub const MAX_TEXT_CHARS: usize = 10_000;

/// The most items one list may hold.
pub const MAX_LIST_ITEMS: usize = 100;

/// The longest link address, in characters.
pub const MAX_URL_CHARS: usize = 2_000;

/// The longest chart id a chart block may name (chart ids are `c_<hex>`).
const MAX_CHART_ID_CHARS: usize = 64;

/// One block of a report, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Block {
    /// A section heading. `level` is 1, 2 or 3.
    Heading {
        /// The heading's words.
        text: String,
        /// 1, 2 or 3; 2 when absent.
        #[serde(default = "default_level")]
        level: u8,
    },
    /// A paragraph of plain text.
    Paragraph {
        /// The paragraph's words.
        text: String,
    },
    /// A bulleted or numbered list.
    List {
        /// The items, in order.
        items: Vec<String>,
        /// Numbered when true.
        #[serde(default)]
        ordered: bool,
    },
    /// A link: the words shown and the address opened.
    Link {
        /// The words shown; the address when empty.
        #[serde(default)]
        text: String,
        /// `https://`, `http://`, `mailto:` or a console path.
        url: String,
    },
    /// A chart from an existing dashboard, drawn live for each reader.
    Chart {
        /// The chart's id.
        chart: String,
        /// The dashboard the chart is on. Set by the server on save.
        #[serde(default)]
        board: String,
    },
}

const fn default_level() -> u8 {
    2
}

impl Block {
    /// The chart this block names, for a chart block.
    #[must_use]
    pub fn chart_id(&self) -> Option<&str> {
        match self {
            Self::Chart { chart, .. } => Some(chart.as_str()),
            _ => None,
        }
    }
}

/// Why a report was refused. Each has one fixed sentence ([`Self::message`]);
/// the parser's own text, which could quote the request, never reaches a
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportRule {
    /// The title is blank.
    TitleRequired,
    /// The title is over [`MAX_TITLE_CHARS`].
    TitleTooLong,
    /// `blocks` is not a list of the five block kinds.
    NotBlocks,
    /// More than [`MAX_BLOCKS`] blocks.
    TooManyBlocks,
    /// More than [`MAX_CHARTS`] chart blocks (decision R4).
    TooManyCharts,
    /// A text over [`MAX_TEXT_CHARS`].
    TextTooLong,
    /// A heading level other than 1, 2 or 3.
    BadHeadingLevel,
    /// A list over [`MAX_LIST_ITEMS`].
    TooManyItems,
    /// A link whose address is not one a reader may safely open.
    BadLink,
    /// A chart block with no chart id, or one too long to be an id.
    ChartRequired,
}

impl ReportRule {
    /// The sentence the caller is told.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::TitleRequired => "A report needs a title.",
            Self::TitleTooLong => "A report title is at most 200 characters.",
            Self::NotBlocks => {
                "blocks must be a list of heading, paragraph, list, link and chart blocks."
            }
            Self::TooManyBlocks => "A report holds at most 300 blocks.",
            Self::TooManyCharts => "A report holds at most 30 charts.",
            Self::TextTooLong => "A block's text is at most 10,000 characters.",
            Self::BadHeadingLevel => "A heading level is 1, 2 or 3.",
            Self::TooManyItems => "A list holds at most 100 items.",
            Self::BadLink => {
                "A link must start with https://, http://, mailto: or / (a page of this console)."
            }
            Self::ChartRequired => "A chart block needs the id of a chart.",
        }
    }
}

/// Read a posted `blocks` value.
///
/// # Errors
///
/// [`ReportRule::NotBlocks`] when it is not a list of the five block kinds
/// (an unknown kind or field included), [`ReportRule::TooManyBlocks`] before
/// it is parsed when the list is too long.
pub fn parse_blocks(raw: Value) -> Result<Vec<Block>, ReportRule> {
    if raw.as_array().is_some_and(|b| b.len() > MAX_BLOCKS) {
        return Err(ReportRule::TooManyBlocks);
    }
    serde_json::from_value(raw).map_err(|_| ReportRule::NotBlocks)
}

/// The title trimmed, or why it cannot be one.
///
/// # Errors
///
/// [`ReportRule::TitleRequired`] or [`ReportRule::TitleTooLong`].
pub fn check_title(title: &str) -> Result<String, ReportRule> {
    let title = title.trim();
    if title.is_empty() {
        return Err(ReportRule::TitleRequired);
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(ReportRule::TitleTooLong);
    }
    Ok(title.to_owned())
}

/// Whether a link address is one a reader may open: the web, an email, or a
/// path on this console (not `//host`, which leaves it).
fn safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (url.starts_with('/') && !url.starts_with("//"))
}

fn long(text: &str) -> bool {
    text.chars().count() > MAX_TEXT_CHARS
}

/// Check the blocks against every limit and tidy them: a link address is
/// trimmed, a chart id is trimmed.
///
/// # Errors
///
/// The first [`ReportRule`] a block breaks, in reading order, except that
/// the chart count is checked first so a report of 31 charts is told the
/// limit and not a later problem.
pub fn check_blocks(blocks: Vec<Block>) -> Result<Vec<Block>, ReportRule> {
    if blocks.len() > MAX_BLOCKS {
        return Err(ReportRule::TooManyBlocks);
    }
    if blocks.iter().filter(|b| b.chart_id().is_some()).count() > MAX_CHARTS {
        return Err(ReportRule::TooManyCharts);
    }
    let mut out = Vec::with_capacity(blocks.len());
    for block in blocks {
        let block = match block {
            Block::Heading { text, level } => {
                if !(1..=3).contains(&level) {
                    return Err(ReportRule::BadHeadingLevel);
                }
                if long(&text) {
                    return Err(ReportRule::TextTooLong);
                }
                Block::Heading { text, level }
            }
            Block::Paragraph { text } => {
                if long(&text) {
                    return Err(ReportRule::TextTooLong);
                }
                Block::Paragraph { text }
            }
            Block::List { items, ordered } => {
                if items.len() > MAX_LIST_ITEMS {
                    return Err(ReportRule::TooManyItems);
                }
                if items.iter().any(|i| long(i)) {
                    return Err(ReportRule::TextTooLong);
                }
                Block::List { items, ordered }
            }
            Block::Link { text, url } => {
                let url = url.trim().to_owned();
                if url.chars().count() > MAX_URL_CHARS || !safe_url(&url) {
                    return Err(ReportRule::BadLink);
                }
                if long(&text) {
                    return Err(ReportRule::TextTooLong);
                }
                Block::Link { text, url }
            }
            Block::Chart { chart, board } => {
                let chart = chart.trim().to_owned();
                if chart.is_empty() || chart.chars().count() > MAX_CHART_ID_CHARS {
                    return Err(ReportRule::ChartRequired);
                }
                Block::Chart { chart, board }
            }
        };
        out.push(block);
    }
    Ok(out)
}

/// The distinct charts the blocks name, in reading order.
#[must_use]
pub fn chart_ids(blocks: &[Block]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    blocks
        .iter()
        .filter_map(Block::chart_id)
        .filter(|id| seen.insert(*id))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn chart(id: &str) -> Value {
        json!({ "type": "chart", "chart": id })
    }

    #[test]
    fn every_block_kind_reads_back_in_order() {
        let blocks = parse_blocks(json!([
            { "type": "heading", "text": "Visitors", "level": 1 },
            { "type": "paragraph", "text": "Up this quarter." },
            { "type": "list", "items": ["a", "b"], "ordered": true },
            { "type": "link", "text": "Source", "url": " https://example.com/x " },
            chart("c_1"),
        ]))
        .unwrap();
        let blocks = check_blocks(blocks).unwrap();
        assert_eq!(blocks.len(), 5);
        assert_eq!(
            blocks[3],
            Block::Link {
                text: "Source".to_owned(),
                url: "https://example.com/x".to_owned()
            }
        );
        assert_eq!(chart_ids(&blocks), vec!["c_1".to_owned()]);
        let back = serde_json::to_value(&blocks).unwrap();
        assert_eq!(
            back[0],
            json!({ "type": "heading", "text": "Visitors", "level": 1 })
        );
        assert_eq!(
            back[4],
            json!({ "type": "chart", "chart": "c_1", "board": "" })
        );
    }

    #[test]
    fn a_heading_without_a_level_is_a_level_two_heading() {
        let blocks = parse_blocks(json!([{ "type": "heading", "text": "A" }])).unwrap();
        assert_eq!(
            blocks[0],
            Block::Heading {
                text: "A".to_owned(),
                level: 2
            }
        );
        let blocks = parse_blocks(json!([{ "type": "heading", "text": "A", "level": 4 }])).unwrap();
        assert_eq!(check_blocks(blocks), Err(ReportRule::BadHeadingLevel));
    }

    /// Decision R4: the 31st chart is refused with a sentence that names the limit.
    #[test]
    fn the_thirty_first_chart_is_refused_with_the_limit_in_the_sentence() {
        let thirty: Vec<Value> = (0..30).map(|i| chart(&format!("c_{i}"))).collect();
        assert!(check_blocks(parse_blocks(json!(thirty)).unwrap()).is_ok());
        let mut more = thirty;
        more.push(chart("c_31"));
        let err = check_blocks(parse_blocks(json!(more)).unwrap()).unwrap_err();
        assert_eq!(err, ReportRule::TooManyCharts);
        assert!(err.message().contains("30 charts"), "{}", err.message());
    }

    #[test]
    fn an_unknown_kind_or_field_is_not_a_block() {
        for raw in [
            json!([{ "type": "html", "text": "<b>x</b>" }]),
            json!([{ "type": "paragraph", "text": "x", "html": "<b>x</b>" }]),
            json!({ "type": "paragraph" }),
            json!([{ "type": "paragraph" }]),
        ] {
            assert_eq!(
                parse_blocks(raw.clone()),
                Err(ReportRule::NotBlocks),
                "{raw}"
            );
        }
    }

    /// Text is stored as typed; nothing here turns it into markup.
    #[test]
    fn markup_in_a_paragraph_is_kept_as_plain_text() {
        let blocks = check_blocks(
            parse_blocks(json!([{ "type": "paragraph", "text": "<script>alert(1)</script>" }]))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            blocks[0],
            Block::Paragraph {
                text: "<script>alert(1)</script>".to_owned()
            }
        );
    }

    #[test]
    fn a_link_that_could_run_code_is_refused() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,x",
            "//elsewhere.example/x",
            "",
        ] {
            let blocks = parse_blocks(json!([{ "type": "link", "url": url }])).unwrap();
            assert_eq!(check_blocks(blocks), Err(ReportRule::BadLink), "{url}");
        }
        for url in [
            "https://a.example",
            "http://a.example",
            "mailto:a@b.example",
            "/dashboards/b_1",
        ] {
            let blocks = parse_blocks(json!([{ "type": "link", "url": url }])).unwrap();
            assert!(check_blocks(blocks).is_ok(), "{url}");
        }
    }

    #[test]
    fn the_text_limits_are_enforced() {
        let long_text = "x".repeat(MAX_TEXT_CHARS + 1);
        let blocks = parse_blocks(json!([{ "type": "paragraph", "text": long_text }])).unwrap();
        assert_eq!(check_blocks(blocks), Err(ReportRule::TextTooLong));
        let items: Vec<String> = (0..=MAX_LIST_ITEMS).map(|i| i.to_string()).collect();
        let blocks = parse_blocks(json!([{ "type": "list", "items": items }])).unwrap();
        assert_eq!(check_blocks(blocks), Err(ReportRule::TooManyItems));
        let many: Vec<Value> = (0..=MAX_BLOCKS)
            .map(|_| json!({ "type": "paragraph", "text": "" }))
            .collect();
        assert_eq!(parse_blocks(json!(many)), Err(ReportRule::TooManyBlocks));
        let blocks = parse_blocks(json!([{ "type": "chart", "chart": "  " }])).unwrap();
        assert_eq!(check_blocks(blocks), Err(ReportRule::ChartRequired));
    }

    #[test]
    fn a_title_is_trimmed_and_bounded() {
        assert_eq!(check_title("  Q3  ").unwrap(), "Q3");
        assert_eq!(check_title("   "), Err(ReportRule::TitleRequired));
        assert_eq!(
            check_title(&"x".repeat(MAX_TITLE_CHARS + 1)),
            Err(ReportRule::TitleTooLong)
        );
    }

    #[test]
    fn the_same_chart_twice_is_one_chart_to_read() {
        let blocks = parse_blocks(json!([chart("c_1"), chart("c_2"), chart("c_1")])).unwrap();
        assert_eq!(chart_ids(&blocks), vec!["c_1".to_owned(), "c_2".to_owned()]);
    }
}
