use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::{env, fs};

use chrono::prelude::*;
use liquid::partials::{EagerCompiler, InMemorySource};
use liquid::{ObjectView, Parser, ParserBuilder, Template, ValueView};
use serde::Serialize;
use thiserror::Error;

use crate::core::{Block, PageIndex, RenderRuleSet, Token};
use crate::{diskio, Config, MarkdownPage};

pub const BLOCK_RULES_TEMPLATE_VAR: &str = "__block_rules";
pub const TAILWIND_FILENAME_TEMPLATE_VAR: &str = "__tailwind_filename";

// TODO not sure I necessarily want this specific impl...
type Partials = EagerCompiler<InMemorySource>;

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("templating engine failure")]
    LiquidError(#[from] liquid::Error),
}

type RenderResult<T> = Result<T, RenderError>;

pub struct Renderer {
    // TODO probably don't want to hold it all in memory!
    layouts: HashMap<String, Template>,
    blocks: HashMap<String, Template>,
    tailwind_filename: String,
}

fn make_partial_key(partial_path: &Path, current_dir: &Path) -> String {
    partial_path
        .strip_prefix(current_dir)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

fn collect_partials(dir: &Path) -> Partials {
    let mut partials = Partials::empty();
    diskio::walk(dir, "liquid").for_each(|path| {
        let layout = fs::read_to_string(&path).unwrap();
        partials.add(make_partial_key(&path, dir), layout);
    });
    partials
}

#[derive(ObjectView, ValueView, Clone, Debug, Serialize)]
struct MetaContext {
    title: String,
    timestamp: String, // TODO can we make it so DateTime can derive a ValueView?
}

impl From<&MarkdownPage> for MetaContext {
    fn from(markdown: &MarkdownPage) -> Self {
        Self {
            title: markdown.frontmatter.title.clone(),
            timestamp: markdown.frontmatter.timestamp.to_rfc3339(),
        }
    }
}

// TODO maybe should just use this for the "page" type in the detail version too?
#[derive(Serialize)]
struct ListingEntry {
    title: String,
    pub timestamp: DateTime<Utc>,
    pub slug: String,
    pub link: String, // TODO this should be "on-demand" and probably like a tag or something
    pub blocks: Vec<Block>,
}

impl From<&MarkdownPage> for ListingEntry {
    fn from(markdown: &MarkdownPage) -> Self {
        // TODO not really liking the clone here!
        Self {
            title: markdown.frontmatter.title.clone(),
            timestamp: markdown.frontmatter.timestamp,
            slug: markdown.frontmatter.slug.clone(),
            link: format!(
                "/{}/{}/",
                markdown.dir_path.to_str().unwrap(),
                markdown.frontmatter.slug
            ),
            blocks: markdown.blocks.clone(),
        }
    }
}

impl Renderer {
    pub fn new(config: &Config, css_file_name: String) -> Self {
        let parser = ParserBuilder::with_stdlib()
            // TODO don't think this is needed as is... but maybe interesting soon.
            .filter(crate::liquid::filters::FirstBlockOfKind)
            .tag(crate::liquid::tags::RenderBlockTag)
            .tag(crate::liquid::tags::TailwindTag)
            .partials(collect_partials(&env::current_dir().unwrap()))
            .build()
            .unwrap();
        let layouts = collect_template_map(&parser, &config.layouts_dir());
        let blocks = collect_template_map(&parser, &config.blocks_dir());
        Self {
            layouts,
            blocks,
            tailwind_filename: css_file_name,
        }
    }

    fn get_template(&self, template_name: &str) -> &Template {
        self.layouts
            .get(template_name)
            // TODO `Result`ize me.
            .unwrap_or_else(|| panic!("could not locate layout: {}", template_name))
    }

    pub fn render_listing_page(
        &self,
        pages: &[MarkdownPage],
        render_rules: &Arc<RenderRuleSet>,
        page_index: PageIndex,
    ) -> RenderResult<String> {
        // TODO ought to genericize this `layout_stack` recursion pattern..
        // or just learn the proper `liquid` idioms?
        // let layout_stack = &render_rules.clone().layouts[..];
        // TODO for now just use the first, will return to this later.
        // TODO should settle on either calling `layout` or `template`? Maybe?
        let layouts = &render_rules.listing.as_ref().unwrap().layouts;
        let template = self.get_template(&layouts[0]);
        let entries: Vec<ListingEntry> = pages.iter().map(|page| page.into()).collect();
        let prev_page_link = if page_index.0 == 0 {
            None
        } else {
            // TODO have to get from some `dir_path` instead of hardcoding.
            Some(format!("/blog/{}/", page_index.0 - 1))
        };
        let next_page_link = if page_index.0 == page_index.1 {
            None
        } else {
            Some(format!("/blog/{}/", page_index.0 + 1))
        };
        let globals = liquid::object!({
            "entries": entries,
            "prev_page_link": prev_page_link, // TODO
            "next_page_link": next_page_link, // TODO
            // TODO should be constants for these, since it's used in the tag.
            BLOCK_RULES_TEMPLATE_VAR: render_rules.block_rules,
            TAILWIND_FILENAME_TEMPLATE_VAR: self.tailwind_filename,
        });
        // TODO better not to discard the info from here
        template.render(&globals).map_err(|e| e.into())
    }

    // TODO not sure `render` has to know that it is `Arc`,
    // probably here is where one uses a `Deref to RenderRuleSet` kind of pattern.
    pub fn render(
        &self,
        page: &MarkdownPage,
        render_rules: &Arc<RenderRuleSet>,
    ) -> RenderResult<String> {
        tracing::debug!(
            "rendering {}/{}",
            page.dir_path.to_str().unwrap(),
            page.frontmatter.slug
        );
        let layout_stack = &render_rules.clone().layouts[..];
        self.render_content(page, render_rules, layout_stack)
    }

    // Recursively render liquid templates, allowing specification of nested layouts.
    // TODO nicer to pass an iterator over layouts perhaps, instead of a slice
    fn render_content(
        &self,
        page: &MarkdownPage,
        render_rules: &Arc<RenderRuleSet>,
        layout_stack: &[String],
    ) -> RenderResult<String> {
        assert!(!layout_stack.is_empty());

        let template = self.get_template(&layout_stack[0]);
        let content = if layout_stack.len() == 1 {
            self.render_blocks(page, render_rules)
        } else {
            self.render_content(page, render_rules, &layout_stack[1..])?
        };
        // TODO this feels like improper use of the liquid concept...
        // maybe there's a better way with defining a custom tag or whatever.
        // works for now though. probably not good to allocate a new object just to pass around
        // the same `meta` down the hierarchy.
        let globals = liquid::object!({
            "meta": MetaContext::from(page),
            "content": content,
            TAILWIND_FILENAME_TEMPLATE_VAR: self.tailwind_filename,
        });
        // TODO better not to discard the info from here
        template.render(&globals).map_err(|e| e.into())
    }

    // TODO should be a `Result`.
    fn render_blocks(&self, page: &MarkdownPage, render_rules: &Arc<RenderRuleSet>) -> String {
        page.blocks
            .iter()
            .map(|block| self.render_block(block, render_rules.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // TODO probably really should be a template tag?
    // it would certainly make it easier to render some block in a listing page...
    fn render_block(&self, block: &Block, render_rules: Arc<RenderRuleSet>) -> String {
        let template_name = render_rules
            .block_rules
            .as_ref()
            .and_then(|rules| rules.get(&block.kind).cloned())
            .unwrap_or_else(|| block.kind.clone());
        let template = self
            .blocks
            .get(&template_name)
            // TODO really ought to error handle!
            .unwrap_or_else(|| panic!("could not locate block template: {}", template_name));
        let globals = liquid::object!({
            "content": block.tokens.iter().map(|token| {
                match token {
                    Token::Literal(text) => text.clone(),
                    Token::Block(nested) => self.render_block(nested, render_rules.clone()),
                }
            }).collect::<Vec<_>>().join(""),
        });
        template.render(&globals).unwrap()
    }
}

/// Helper fn for collecting layouts from a directory.
fn collect_template_map(parser: &Parser, dir: &Path) -> HashMap<String, Template> {
    // TODO stuff like this should be parallelizable..
    HashMap::from_iter(diskio::walk(dir, "liquid").map(|path| {
        let key = make_layout_key(path.strip_prefix(dir).unwrap());
        let raw = fs::read_to_string(&path).unwrap();
        let layout = parser.parse(raw.trim()).unwrap();
        (key, layout)
    }))
}

fn make_layout_key(path: &Path) -> String {
    // TODO should check for non `layout` duplicates rather than permitting
    match path.file_stem().unwrap().to_str().unwrap() {
        "layout" => path.to_string_lossy().to_string(),
        name => name.to_string(),
    }
}
