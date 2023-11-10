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
    });
}

// TODO should this even be defined here at this point?
#[derive(Debug)]
pub struct Page {
    pub path: PathBuf,
    pub frontmatter: FrontMatter,
    pub blocks: Vec<Block>,
    pub render_rules: Arc<RenderRuleSet>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderRuleSet {
    pub layouts: Vec<String>,
    #[serde(rename = "blocks")]
    pub block_rules: Option<HashMap<String, String>>,
}

#[derive(Debug, PartialEq)]
pub struct Block {
    pub kind: String, // TODO want to avoid alloc here!
    pub tokens: Vec<Token>,
    // TODO will need to include something extra to represent arbitrary metadata
    // in particular, the URLs of links.
}

// TODO "token" isn't really a great name for these.
#[derive(Debug, PartialEq)]
pub enum Token {
    Literal(String),
    Block(Block),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
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
