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
use crate::{diskio, Config, Markdown};

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
    parser: Parser,
    // TODO do we really want to have all layouts in memory at generation time?
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

// TODO pointless to walk & search for '.liquid' 3x - 2x here and then also in `generator.rs`
// for supplying an arg to `railwind`.
fn collect_partials(dir: &Path) -> Partials {
    let mut partials = Partials::empty();
    diskio::walk(dir, "liquid").for_each(|path| {
        let layout = fs::read_to_string(&path).unwrap();
        partials.add(make_partial_key(&path, dir), layout);
    });
    partials
}

/// Helper fn for collecting layouts from a directory.
fn collect_template_map(parser: &Parser, dir: &Path) -> HashMap<String, Template> {
    // TODO stuff like this should be parallelizable..
    HashMap::from_iter(diskio::walk(dir, "liquid").map(|path| {
        let key = make_template_key(path.strip_prefix(dir).unwrap());
        let raw = fs::read_to_string(&path).unwrap();
        let layout = parser.parse(raw.trim()).unwrap();
        (key, layout)
    }))
}

fn make_template_key(path: &Path) -> String {
    // TODO should check for non `layout` duplicates rather than permitting
    match path.file_stem().unwrap().to_str().unwrap() {
        "layout" => path.to_string_lossy().to_string(),
        name => name.to_string(),
    }
}

#[derive(Debug)]
enum Renderable<'a> {
    Markdown(&'a Markdown),
    Html(&'a str),
}

impl Renderable<'_> {
    fn render(&self, renderer: &Renderer, render_rules: &Arc<RenderRuleSet>) -> String {
        match self {
            Self::Markdown(markdown) => renderer.render_blocks(markdown, render_rules),
            Self::Html(html) => html.to_string(),
        }
    }

    fn get_context(&self) -> liquid::Object {
        match self {
            Self::Markdown(markdown) => liquid::object!({
                "title": markdown.frontmatter.title.clone(),
                "timestamp": markdown.frontmatter.timestamp.to_rfc3339(),
            }),
            // TODO this is kind of hack but alas, works for now.
            _ => liquid::object!({
                "title": "",
                "timestamp": "",
            }),
        }
    }
}

#[derive(ObjectView, ValueView, Clone, Debug, Serialize)]
struct MarkdownContext {
    title: String,
    timestamp: String, // TODO can we make it so DateTime can derive a ValueView?
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

impl From<&Markdown> for ListingEntry {
    fn from(markdown: &Markdown) -> Self {
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
            parser,
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

    pub fn render<P: AsRef<Path>>(&self, path: P) -> RenderResult<String> {
        let raw = fs::read_to_string(&path).unwrap();
        let template = self.parser.parse(raw.trim()).unwrap();
        let globals = liquid::object!({
            // TODO should be constants for these, since it's used in the tag.
            TAILWIND_FILENAME_TEMPLATE_VAR: self.tailwind_filename,
        });
        template.render(&globals).map_err(|e| e.into())
    }

    pub fn render_listing_page(
        &self,
        pages: &[Markdown],
        render_rules: &Arc<RenderRuleSet>,
        page_index: PageIndex,
    ) -> RenderResult<String> {
        // TODO feels like the parsing of blocks within the markdown should be lazy,
        // but that could be tricky.

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
    pub fn render_markdown(
        &self,
        markdown: &Markdown,
        render_rules: &Arc<RenderRuleSet>,
    ) -> RenderResult<String> {
        tracing::debug!(
            "rendering {}/{}",
            markdown.dir_path.to_str().unwrap(),
            markdown.frontmatter.slug
        );
        let layout_stack = &render_rules.clone().layouts[..];
        self.render_content(&Renderable::Markdown(markdown), render_rules, layout_stack)
    }

    pub fn render_html(
        &self,
        html: &str,
        render_rules: &Arc<RenderRuleSet>,
    ) -> RenderResult<String> {
        let layout_stack = &render_rules.clone().layouts[..];
        self.render_content(&Renderable::Html(html), render_rules, layout_stack)
    }

    // Recursively render liquid templates, allowing specification of nested layouts.
    // TODO nicer to pass an iterator over layouts perhaps, instead of a slice
    fn render_content(
        &self,
        renderable: &Renderable,
        render_rules: &Arc<RenderRuleSet>,
        layout_stack: &[String],
    ) -> RenderResult<String> {
        assert!(!layout_stack.is_empty());

        let template = self.get_template(&layout_stack[0]);
        let content = if layout_stack.len() == 1 {
            renderable.render(self, render_rules)
        } else {
            self.render_content(renderable, render_rules, &layout_stack[1..])?
        };
        // TODO this feels like improper use of the liquid concept...
        // maybe there's a better way with defining a custom tag or whatever.
        // works for now though. probably not good to allocate a new object just to pass around
        // the same `meta` down the hierarchy.
        let globals = liquid::object!({
            "meta": renderable.get_context(),
            "content": content,
            TAILWIND_FILENAME_TEMPLATE_VAR: self.tailwind_filename,
        });
        // TODO better not to discard the info from here
        template.render(&globals).map_err(|e| e.into())
    }

    // TODO should be a `Result`.
    fn render_blocks(&self, page: &Markdown, render_rules: &Arc<RenderRuleSet>) -> String {
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