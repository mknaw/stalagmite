// TODO maybe this should be named `core`?

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::utils::slugify;

lazy_static! {
    pub static ref DEFAULT_RENDER_RULE_SET: Arc<RenderRuleSet> = Arc::new(RenderRuleSet {
        layouts: vec!["primary".to_string()],
        block_rules: None,
        listing: None,
    });
}

#[derive(Debug)]
pub enum FileType {
    Markdown,
    Liquid,
    Html,
}

#[derive(Debug)]
pub struct FileEntry {
    pub abs_path: PathBuf,
    pub rel_path: PathBuf,
    pub ftype: FileType,
}

/// Contains one level of the site hierarchy.
/// By design, each entry in the level shares the same `RenderRuleSet`.
/// We may want to process entries of a given layer in sequence, since we may need to generate
/// listings pages as well, but otherwise, after the initial collection, `GenerationNode`s can
/// be processed in parallel.
#[derive(Debug)]
pub struct GenerationNode {
    // TODO still needed? Feels like one could get away with just the info on `FileEntry`.
    pub dir_path: String,
    pub rules: Arc<RenderRuleSet>,
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Serialize)]
pub struct MarkdownPage {
    pub dir_path: PathBuf, // TODO must one really be `String` and the other `PathBuf`?
    pub frontmatter: FrontMatter,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderRuleSet {
    pub layouts: Vec<String>,
    #[serde(rename = "blocks")]
    pub block_rules: Option<HashMap<String, String>>,
    pub listing: Option<ListingRuleSet>,
}

impl RenderRuleSet {
    pub fn should_render_listing(&self) -> bool {
        self.listing.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingRuleSet {
    pub layouts: Vec<String>,
    pub page_size: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Block {
    pub kind: String, // TODO want to avoid alloc here!
    pub tokens: Vec<Token>,
    // TODO will need to include something extra to represent arbitrary metadata
    // in particular, the URLs of links.
}

// TODO "token" isn't really a great name for these.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Token {
    Literal(String),
    Block(Block),
}

// TODO really might be interested in calling this `PageMetadata` or something
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrontMatter {
    pub title: String,
    pub timestamp: DateTime<Utc>,
    pub slug: String,
    // TODO any other arbitrary KVs?
}

impl TryFrom<Vec<(&str, &str)>> for FrontMatter {
    type Error = &'static str;

    fn try_from(kvs: Vec<(&str, &str)>) -> Result<Self, Self::Error> {
        let kvs = HashMap::from_iter(kvs);
        kvs.try_into()
    }
}

impl TryFrom<HashMap<&str, &str>> for FrontMatter {
    type Error = &'static str;

    fn try_from(kv: HashMap<&str, &str>) -> Result<Self, Self::Error> {
        let title = kv.get("title").ok_or("missing title")?;
        let timestamp = kv
            .get("timestamp")
            .map(|s| DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)))
            .unwrap()
            .unwrap();
        let slug = kv
            .get("slug")
            .map_or_else(|| slugify(title), |s| s.to_string());
        Ok(FrontMatter {
            title: title.to_string(),
            timestamp,
            slug,
        })
    }
}

/// Represents "page `i` of `n`".
pub type PageIndex = (usize, usize);
