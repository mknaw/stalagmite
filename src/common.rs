use std::collections::HashMap;
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::utils;
use crate::utils::slugify;

lazy_static! {
    pub static ref DEFAULT_RENDER_RULE_SET: Arc<RenderRules> = Arc::new(RenderRules {
        layouts: vec!["primary".to_string()],
        block_rules: None,
        listing: None,
    });
}

pub const DEFAULT_LISTING_PAGE_SIZE: u8 = 100;

#[derive(Debug, Clone)]
pub enum PageType {
    Markdown,
    Liquid,
    Html,
}

#[derive(Debug)]
pub struct ContentFile {
    // Absolute path of the file.
    pub abs_path: Utf8PathBuf,
    // Path relative to the pages directory.
    pub rel_path: Utf8PathBuf,
    // File contents.
    pub contents: String,
    // Cached content hash.
    pub hash: u64,
}

impl ContentFile {
    pub async fn new(pages_dir: &Utf8Path, abs_path: Utf8PathBuf) -> anyhow::Result<Self> {
        let rel_path = abs_path.strip_prefix(pages_dir).unwrap().to_owned();
        let contents = tokio::fs::read(&abs_path).await;
        let hash = utils::hash(contents.as_ref().unwrap());
        Ok(Self {
            abs_path,
            rel_path,
            contents: contents
                .map(|c| std::str::from_utf8(&c).unwrap().to_string())
                .map_err(|e| anyhow::anyhow!("{}", e))?,
            hash,
        })
    }

    /// Returns the absolute path of the directory containing the file.
    pub fn abs_dir(&self) -> Utf8PathBuf {
        self.abs_path.parent().unwrap().to_owned()
    }

    /// Returns the relative path of the directory containing the file.
    pub fn rel_dir(&self) -> Utf8PathBuf {
        self.rel_path.parent().unwrap().to_owned()
    }
}

/// Representation of the filesystem path of a page to render.
#[derive(Debug)]
pub struct SiteEntry {
    // The actual underlying file info.
    pub file: ContentFile,
    // Desired output path, relative to the output directory.
    pub out_path: Utf8PathBuf,
    // Relative url path.
    pub url_path: String,
}

impl SiteEntry {
    pub async fn try_new(pages_dir: &Utf8Path, abs_path: Utf8PathBuf) -> anyhow::Result<Self> {
        if matches!(
            abs_path.extension(),
            Some("md") | Some("liquid") | Some("html")
        ) {
            let file = ContentFile::new(pages_dir, abs_path).await?;
            // let rel_path = abs_path.strip_prefix(pages_dir)?.to_owned();
            let mut out_path = file
                .rel_path
                .with_extension("")
                .components()
                .map(|c| slugify(c.as_os_str().to_str().unwrap()))
                .collect::<Vec<_>>()
                .join("/")
                .parse::<Utf8PathBuf>()?;

            match out_path.file_stem().unwrap() {
                "index" => {
                    out_path.set_extension("html");
                }
                _ => {
                    out_path = out_path.join("index.html");
                }
            };

            let url_path = format!("{}/", out_path.parent().unwrap());

            Ok(Self {
                file,
                out_path,
                url_path,
            })
        } else {
            anyhow::bail!("Invalid file type")
        }
    }

    /// Returns the type of the file.
    pub fn get_page_type(&self) -> PageType {
        match self.file.rel_path.extension() {
            Some("md") => PageType::Markdown,
            Some("liquid") => PageType::Liquid,
            Some("html") => PageType::Html,
            _ => panic!("Unknown file type"),
        }
    }

    pub fn parent_url(&self) -> String {
        // TODO might be better off just doing this with the rest of my ungodly initialization.
        let segments: Vec<&str> = self
            .url_path
            .split('/')
            .filter(|&s| !s.is_empty())
            .collect();
        if segments.is_empty() {
            return "".to_string();
        }
        if segments.len() == 1 && self.url_path.starts_with('/') {
            return "/".to_string();
        }
        segments[..segments.len() - 1].join("/")
    }
}

/// Contains one level of the site hierarchy.
/// By design, each entry in the level shares the same `RenderRules`.
/// We may want to process entries of a given layer in sequence, since we may need to generate
/// listings pages as well, but otherwise, after the initial collection, `SiteNode`s can
/// be processed in parallel.
#[derive(Debug)]
pub struct SiteNode {
    // TODO still needed? Feels like one could get away with just the info on `ContentFile`.
    pub dir: Utf8PathBuf,
    pub render_rules: Arc<RenderRules>,
    pub site_entries: Vec<SiteEntry>,
}

#[derive(Debug)]
pub enum PageData {
    Markdown(Markdown),
    Liquid,
    Html,
}

#[derive(Debug, Serialize)]
pub struct Markdown {
    pub frontmatter: FrontMatter,
    pub blocks: Vec<Block>,
}

pub type BlockRules = HashMap<String, String>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderRules {
    pub layouts: Vec<String>,
    #[serde(rename = "blocks")]
    pub block_rules: Option<BlockRules>,
    pub listing: Option<ListingRules>,
}

impl RenderRules {
    pub fn should_render_listing(&self) -> bool {
        self.listing.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingRules {
    pub layouts: Vec<String>,
    pub page_size: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Block {
    pub kind: String, // TODO want to avoid alloc here!
    pub tokens: Vec<Token>,
    pub meta: Option<HashMap<String, String>>,
    // TODO will need to include something extra to represent arbitrary metadata
    // in particular, the URLs of links.
}

// TODO "token" isn't really a great name for these.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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
pub type PageIndex = (u8, u8);
