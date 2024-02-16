use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

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

#[derive(Debug, Clone)]
pub enum PageType {
    Markdown,
    Liquid,
    Html,
}

/// Representation of the filesystem path of a page to render.
#[derive(Debug, Clone)]
pub struct SiteEntry {
    // TODO not sure I really want the rel_path, abs_path thing...
    // Absolute path of the file.
    pub abs_path: PathBuf,
    // Path relative to the pages directory.
    pub rel_path: PathBuf,
    // Desired output path, relative to the output directory.
    pub out_path: PathBuf,
    // Relative url path.
    pub url_path: String,
    // Cached file contents.
    // TODO this might not be so good...
    // we might be keeping the contents in memory for a long time.
    pub contents: OnceLock<String>,
    // Cached content hash.
    pub hash: OnceLock<String>,
}

// TODO ought to call this something else now... since it's not just for Pages, per se.
impl SiteEntry {
    pub fn try_new<P: AsRef<Path>>(pages_dir: P, path: P) -> anyhow::Result<Self> {
        let abs_path = path.as_ref().to_path_buf();
        if matches!(
            abs_path.extension().and_then(|ext| ext.to_str()),
            Some("md") | Some("liquid") | Some("html")
        ) {
            let rel_path = abs_path.strip_prefix(pages_dir)?.to_owned();
            let mut out_path = rel_path
                .with_extension("")
                .components()
                .map(|c| slugify(c.as_os_str().to_str().unwrap()))
                .collect::<Vec<_>>()
                .join("/")
                .parse::<PathBuf>()?;

            match out_path.file_stem().unwrap().to_str().unwrap() {
                "index" => {
                    out_path.set_extension("html");
                }
                _ => {
                    out_path = out_path.join("index.html");
                }
            };

            let url_path = format!("{}/", out_path.parent().unwrap().to_str().unwrap());

            Ok(Self {
                abs_path,
                rel_path,
                out_path,
                url_path,
                contents: OnceLock::new(),
                hash: OnceLock::new(),
            })
        } else {
            anyhow::bail!("Invalid file type")
        }
    }

    /// Returns the type of the file.
    pub fn get_page_type(&self) -> PageType {
        match self.rel_path.extension().and_then(|ext| ext.to_str()) {
            Some("md") => PageType::Markdown,
            Some("liquid") => PageType::Liquid,
            Some("html") => PageType::Html,
            _ => panic!("Unknown file type"),
        }
    }

    /// Returns the absolute path of the directory containing the file.
    pub fn abs_dir(&self) -> PathBuf {
        self.abs_path.parent().unwrap().to_owned()
    }

    /// Returns the relative path of the directory containing the file.
    pub fn rel_dir(&self) -> PathBuf {
        self.rel_path.parent().unwrap().to_owned()
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

    pub fn get_contents(&self) -> anyhow::Result<&str> {
        self.contents
            .get_or_try_init(|| {
                std::fs::read(&self.abs_path)
                    .map(|c| std::str::from_utf8(&c).unwrap().to_string())
                    .map_err(|e| anyhow::anyhow!("{}", e))
            })
            .map(|c| c.as_str())
    }

    pub fn get_hash(&self) -> anyhow::Result<&str> {
        self.hash
            .get_or_try_init(|| {
                let contents = self.get_contents()?;
                Ok(utils::hash(contents.as_bytes()))
            })
            .map(|h| h.as_str())
    }
}

#[derive(Debug)]
pub struct Page {
    pub file: SiteEntry,
    pub data: PageData,
}

#[derive(Debug)]
pub enum PageData {
    Markdown(Markdown),
    Liquid,
    Html,
}

trait GenericPage {
    fn get_url(&self) -> String;
}

impl Page {
    pub fn new_markdown_page(file: SiteEntry, markdown: Markdown) -> Self {
        Self {
            file,
            data: PageData::Markdown(markdown),
        }
    }

    pub fn new_liquid_page(file: SiteEntry) -> Self {
        Self {
            file,
            data: PageData::Liquid,
        }
    }

    pub fn new_html_page(file: SiteEntry) -> Self {
        Self {
            file,
            data: PageData::Html,
        }
    }

    // TODO verify that data matches file.file_type?
    pub fn get_url(&self) -> String {
        match &self.data {
            PageData::Markdown(md) => {
                format!("{:?}/{}", self.file.rel_dir(), md.frontmatter.slug)
            }
            PageData::Liquid => unimplemented!(),
            PageData::Html => self.file.rel_path.to_string_lossy().to_string(),
        }
    }

    pub fn get_link(&self) -> String {
        format!(
            "{}/",
            self.file.out_path.parent().unwrap().to_str().unwrap()
        )
    }
}

/// Contains one level of the site hierarchy.
/// By design, each entry in the level shares the same `RenderRuleSet`.
/// We may want to process entries of a given layer in sequence, since we may need to generate
/// listings pages as well, but otherwise, after the initial collection, `SiteNode`s can
/// be processed in parallel.
#[derive(Debug)]
pub struct SiteNode {
    // TODO still needed? Feels like one could get away with just the info on `FileEntry`.
    pub dir: PathBuf,
    pub render_rules: Arc<RenderRules>,
    pub site_entries: Vec<SiteEntry>,
}

#[derive(Debug, Serialize)]
pub struct Markdown {
    pub frontmatter: FrontMatter,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderRules {
    pub layouts: Vec<String>,
    #[serde(rename = "blocks")]
    pub block_rules: Option<HashMap<String, String>>,
    pub listing: Option<ListingRuleSet>,
}

impl RenderRules {
    pub fn should_render_listing(&self) -> bool {
        self.listing.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingRuleSet {
    pub layouts: Vec<String>,
    pub page_size: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Block {
    pub kind: String, // TODO want to avoid alloc here!
    pub tokens: Vec<Token>,
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
pub type PageIndex = (usize, usize);
