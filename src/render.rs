use std::collections::HashMap;
use std::path::Path;
use std::{env, fs};

use liquid::partials::{EagerCompiler, InMemorySource};
use liquid::{ObjectView, Parser, ParserBuilder, Template, ValueView};
use serde::Serialize;
use thiserror::Error;

use crate::pages::{Block, Token};
use crate::{diskio, Config, Page};

// TODO not sure I necessarily want this specific impl...
type Partials = EagerCompiler<InMemorySource>;

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("templating engine failure")]
    LiquidError(#[from] liquid::Error),
}

type RenderResult<T> = Result<T, RenderError>;

pub struct Renderer {
    // parser: Parser,
    // TODO probably don't want to hold it all in memory!
    layouts: HashMap<String, Template>,
    blocks: HashMap<String, Template>,
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

impl From<&Page> for MetaContext {
    fn from(markdown: &Page) -> Self {
        Self {
            title: markdown.frontmatter.title.clone(),
            timestamp: markdown.frontmatter.timestamp.to_rfc3339(),
        }
    }
}

impl Renderer {
    pub fn new(config: &Config) -> Self {
        let parser = ParserBuilder::with_stdlib()
            // TODO don't think this is needed as is... but maybe interesting soon.
            .partials(collect_partials(&env::current_dir().unwrap()))
            .build()
            .unwrap();
        let layouts = collect_template_map(&parser, &config.layouts_dir());
        let blocks = collect_template_map(&parser, &config.blocks_dir());
        Self {
            // parser,
            layouts,
            blocks,
        }
    }

    pub fn render(&self, page: &Page) -> RenderResult<String> {
        let render_rules = &page.render_rules;
        self.render_content(page, &render_rules.layouts)
    }

    // Recursively render liquid templates, allowing specification of nested layouts.
    // TODO nicer to pass an iterator perhaps, instead of a slice
    fn render_content(&self, page: &Page, layout_stack: &[String]) -> RenderResult<String> {
        assert!(!layout_stack.is_empty());
        let template = self
            .layouts
            .get(&layout_stack[0])
            .unwrap_or_else(|| panic!("could not locate layout: {}", layout_stack[0]));
        let content = if layout_stack.len() == 1 {
            self.render_blocks(page)
        } else {
            self.render_content(page, &layout_stack[1..])?
        };
        // TODO this feels like improper use of the liquid concept...
        // maybe there's a better way with defining a custom tag or whatever.
        // works for now though. probably not good to allocate a new object just to pass around
        // the same `meta` down the hierarchy.
        let globals = liquid::object!({
            "meta": MetaContext::from(page),
            "content": content,
        });
        // TODO better not to discard the info from here
        template.render(&globals).map_err(|e| e.into())
    }

    // TODO should be a `Result`.
    fn render_blocks(&self, page: &Page) -> String {
        page.blocks
            .iter()
            .map(|block| self.render_block(page, block))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_block(&self, page: &Page, block: &Block) -> String {
        let template_name = page
            .render_rules
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
                    Token::Block(nested) => self.render_block(page, nested),
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
