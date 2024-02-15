use std::collections::HashMap;
use std::fs;
use std::ops::Deref;
use std::path::Path;

use chrono::prelude::*;
use liquid::partials::{EagerCompiler, InMemorySource};
use liquid::{ObjectView, Parser, ParserBuilder, Template, ValueView};
use serde::Serialize;
use thiserror::Error;

use crate::core::{Block, Page, PageData, PageFile, PageIndex, RenderRules, Token};
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

fn make_partial_key(partial_path: &Path, current_dir: &Path) -> String {
    partial_path
        .strip_prefix(current_dir)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
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
    fn render<R: Deref<Target = RenderRules>>(
        &self,
        renderer: &Renderer,
        render_rules: &R,
    ) -> String {
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

impl From<&Page> for ListingEntry {
    fn from(page: &Page) -> Self {
        match &page.data {
            PageData::Markdown(md) => Self {
                title: md.frontmatter.title.clone(),
                timestamp: md.frontmatter.timestamp,
                slug: md.frontmatter.slug.clone(),
                link: page.get_link(),
                blocks: md.blocks.clone(),
            },
            _ => unimplemented!(),
        }
    }
}

pub struct Renderer {
    // TODO do we really want to have all layouts in memory at generation time?
    layouts: HashMap<String, Template>,
    blocks: HashMap<String, Template>,
    tailwind_filename: String,
}

impl Renderer {
    pub fn new(config: &Config, css_file_name: String, partials: &[PageFile]) -> Self {
        let partials = partials
            .iter()
            .fold(Partials::empty(), |mut partials, page_file| {
                partials.add(
                    make_partial_key(&page_file.abs_dir(), &config.project_dir),
                    page_file.get_contents().unwrap(),
                );
                partials
            });

        let parser = ParserBuilder::with_stdlib()
            // TODO don't think this is needed as is... but maybe interesting soon.
            .partials(partials)
            .tag(crate::liquid::tags::RenderBlockTag)
            .tag(crate::liquid::tags::TailwindTag)
            .filter(crate::liquid::filters::FirstBlockOfKind)
            .build()
            .unwrap();

        // TODO this is really stupid, since we already have this available in `partials`,
        // and we've even done all the reading of those files etc.
        // Or maybe partials should really be partials and these things are kept as templates.
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

    pub fn render_page<R: Deref<Target = RenderRules>>(
        &self,
        page: &Page,
        render_rules: &R,
    ) -> RenderResult<String> {
        match &page.data {
            PageData::Markdown(md) => self.render_markdown(&page.file, md, render_rules),
            PageData::Liquid => unimplemented!(),
            PageData::Html => self.render_html(page.file.get_contents().unwrap(), render_rules),
        }
    }

    pub fn render_listing_page<R: Deref<Target = RenderRules>>(
        &self,
        pages: &[Page],
        render_rules: &R,
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
    fn render_markdown<R: Deref<Target = RenderRules>>(
        &self,
        page_file: &PageFile,
        markdown: &Markdown,
        render_rules: &R,
    ) -> RenderResult<String> {
        tracing::debug!(
            "rendering {:?}/{}",
            page_file.rel_dir(),
            markdown.frontmatter.slug
        );
        let layout_stack = &render_rules.layouts[..];
        self.render_content(&Renderable::Markdown(markdown), render_rules, layout_stack)
    }

    fn render_html<R: Deref<Target = RenderRules>>(
        &self,
        html: &str,
        render_rules: &R,
    ) -> RenderResult<String> {
        let layout_stack = &render_rules.layouts[..];
        self.render_content(&Renderable::Html(html), render_rules, layout_stack)
    }

    // Recursively render liquid templates, allowing specification of nested layouts.
    // TODO nicer to pass an iterator over layouts perhaps, instead of a slice
    fn render_content<R: Deref<Target = RenderRules>>(
        &self,
        renderable: &Renderable,
        render_rules: &R,
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
    fn render_blocks<R: Deref<Target = RenderRules>>(
        &self,
        page: &Markdown,
        render_rules: &R,
    ) -> String {
        page.blocks
            .iter()
            .map(|block| self.render_block(block, render_rules))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // TODO probably really should be a template tag?
    // it would certainly make it easier to render some block in a listing page...
    fn render_block<R: Deref<Target = RenderRules>>(
        &self,
        block: &Block,
        render_rules: &R,
    ) -> String {
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
                    Token::Block(nested) => self.render_block(nested, render_rules),
                }
            }).collect::<Vec<_>>().join(""),
        });
        template.render(&globals).unwrap()
    }
}
