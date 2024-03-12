use std::collections::HashMap;
use std::fs;
use std::ops::Deref;

use camino::Utf8Path;
use chrono::prelude::*;
use futures::StreamExt;
use liquid::partials::{EagerCompiler, InMemorySource};
use liquid::{ObjectView, Parser, ParserBuilder, Template, ValueView};
use serde::Serialize;
use thiserror::Error;

use crate::common::{Block, BlockRules, ContentFile, PageData, RenderRules};
use crate::{diskio, Config, Markdown};

pub const BLOCK_RULES_TEMPLATE_VAR: &str = "__block_rules";
pub const STATIC_ASSET_MAP_TEMPLATE_VAR: &str = "__static_asset_map";
pub const TAILWIND_FILENAME_TEMPLATE_VAR: &str = "__tailwind_filename";

// TODO not sure I necessarily want this specific impl...
type Partials = EagerCompiler<InMemorySource>;

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("templating engine failure")]
    LiquidError(#[from] liquid::Error),
}

type RenderResult<T> = Result<T, RenderError>;

// TODO this is pretty silly, just want the rel path, don't need a fancy fn for it.
fn make_partial_key(partial_path: &Utf8Path, current_dir: &Utf8Path) -> String {
    partial_path
        .strip_prefix(current_dir)
        .unwrap()
        .as_str()
        .to_owned()
}

/// Helper fn for collecting layouts from a directory.
async fn collect_template_map(parser: &Parser, dir: &Utf8Path) -> HashMap<String, Template> {
    // TODO stuff like this should be parallelizable..
    diskio::walk(dir, &Some("liquid"))
        .map(|path| {
            let key = make_template_key(path.strip_prefix(dir).unwrap());
            let raw = fs::read_to_string(&path).unwrap();
            let layout = parser.parse(raw.trim()).unwrap();
            (key, layout)
        })
        .collect::<HashMap<_, _>>()
        .await
}

fn make_template_key(path: &Utf8Path) -> String {
    // TODO should check for non `layout` duplicates rather than permitting
    match path.file_stem().unwrap() {
        "layout" => path.to_string(),
        name => name.to_string(),
    }
}

fn get_inner_content(
    renderer: &Renderer,
    render_rules: &RenderRules,
    page_data: &PageData,
) -> String {
    match page_data {
        PageData::Markdown(md) => renderer.render_blocks(&md.blocks, &render_rules.block_rules),
        _ => "".to_string(),
    }
}

fn get_meta_context(page_data: &PageData) -> liquid::Object {
    match page_data {
        PageData::Markdown(md) => {
            liquid::object!({
                "title": md.frontmatter.title.clone(),
                "timestamp": md.frontmatter.timestamp.to_rfc3339(),
            })
        }
        PageData::Listing(group_path, group, page_index) => {
            let entries: Vec<ListingEntry> = group.iter().map(|markdown| markdown.into()).collect();

            let prev_page_link = if page_index.0 == 0 {
                None
            } else {
                Some(format!("/{}/{}/", group_path, page_index.0 - 1))
            };
            let next_page_link = if (page_index.0 + 1) == page_index.1 {
                None
            } else {
                Some(format!("/{}/{}/", group_path, page_index.0 + 1))
            };

            liquid::object!({
                "title": "",
                "timestamp": "",
                "entries": entries,
                "prev_page_link": prev_page_link, // TODO
                "next_page_link": next_page_link, // TODO
            })
        }
        _ => liquid::object!({
            "title": "",
            "timestamp": "",
        }),
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

impl From<&(Markdown, String)> for ListingEntry {
    fn from(tup: &(Markdown, String)) -> Self {
        let (markdown, url) = tup;
        Self {
            title: markdown.frontmatter.title.clone(),
            timestamp: markdown.frontmatter.timestamp,
            slug: markdown.frontmatter.slug.clone(),
            link: format!("/{}", url),
            blocks: markdown.blocks.clone(),
        }
    }
}

pub struct Renderer {
    // TODO do we really want to have all layouts in memory at generation time?
    layouts: HashMap<String, Template>,
    block_content_template: Template,
    static_asset_map: HashMap<String, String>,
    // TODO nowadays can probably just get this from the static_asset_map.
    tailwind_filename: String,
}

impl Renderer {
    pub async fn new(
        config: &Config,
        static_asset_map: HashMap<String, String>,
        // TODO why was this needed anyway?
        css_file_name: String,
        partials: Vec<ContentFile>,
    ) -> Self {
        let partials =
            partials
                .into_iter()
                .fold(Partials::empty(), |mut partials, content_file| {
                    partials.add(
                        make_partial_key(&content_file.abs_path, &config.project_dir),
                        content_file.contents,
                    );
                    partials
                });

        let parser = ParserBuilder::with_stdlib()
            // TODO don't think this is needed as is... but maybe interesting soon.
            .partials(partials)
            .tag(crate::liquid::tags::RenderBlockTag)
            .tag(crate::liquid::tags::StaticAssetTag)
            .tag(crate::liquid::tags::TailwindTag)
            .filter(crate::liquid::filters::FirstBlockOfKind)
            .build()
            .unwrap();

        // TODO this is really stupid, since we already have this available in `partials`,
        // and we've even done all the reading of those files etc.
        // Or maybe partials should really be partials and these things are kept as templates.
        let layouts = collect_template_map(&parser, &config.layouts_dir()).await;

        // TODO this whole maneuver still seems kind of hacky, but it's better than prior art.
        let block_content_template = parser
            .parse("{% for block in blocks %}{% render_block block %}{% endfor %}")
            .unwrap();

        Self {
            layouts,
            block_content_template,
            static_asset_map,
            tailwind_filename: css_file_name,
        }
    }

    fn get_template(&self, template_name: &str) -> &Template {
        self.layouts
            .get(template_name)
            // TODO `Result`ize me.
            .unwrap_or_else(|| panic!("could not locate layout: {}", template_name))
    }

    // Recursively render liquid templates, allowing specification of nested layouts.
    pub fn render<R: Deref<Target = RenderRules>>(
        &self,
        page_data: &PageData,
        render_rules: &R,
        layouts: &[String],
    ) -> RenderResult<String> {
        let mut content = get_inner_content(self, render_rules, page_data);

        let meta_context = get_meta_context(page_data);

        for layout in layouts.iter().rev() {
            println!("{:?}", layout);
            println!("{:?}", content);
            let template = self.get_template(layout);
            let globals = liquid::object!({
                // Kind of stupid to be cloning this stuff, but whatever.
                "meta": meta_context.clone(),
                "content": content,
                BLOCK_RULES_TEMPLATE_VAR: render_rules.block_rules,
                STATIC_ASSET_MAP_TEMPLATE_VAR: self.static_asset_map,
                TAILWIND_FILENAME_TEMPLATE_VAR: self.tailwind_filename,
            });
            // TODO better not to discard the info from here
            content = template.render(&globals)?;
        }
        Ok(content)
    }

    // TODO should be a `Result`.
    fn render_blocks(&self, blocks: &[Block], block_rules: &Option<BlockRules>) -> String {
        let ctx = liquid::object!({
            "blocks": blocks,
            BLOCK_RULES_TEMPLATE_VAR: block_rules,
        });
        self.block_content_template.render(&ctx).unwrap()
    }
}
