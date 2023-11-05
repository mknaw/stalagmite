use std::collections::HashMap;
use std::path::Path;
use std::{env, fs};

use ignore::Walk;
use liquid::partials::{EagerCompiler, InMemorySource};
use liquid::{ObjectView, Parser, ParserBuilder, ValueView};
use serde::Serialize;
use thiserror::Error;

use crate::diskio::collect_layouts;
use crate::markdown::{Markdown, Token};
use crate::Config;

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
    // TODO probably don't want to hold it all in memory!
    layouts: HashMap<String, String>,
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
    for entry in Walk::new(dir).flatten() {
        if entry.path().is_file() && entry.path().extension() == Some("liquid".as_ref()) {
            let layout = fs::read_to_string(entry.path()).unwrap();
            // TODO do we really want the paths for the layouts?
            partials.add(make_partial_key(entry.path(), dir), layout);
        }
    }
    partials
}

#[derive(ObjectView, ValueView, Clone, Debug, Serialize)]
struct MetaContext {
    title: String,
    timestamp: String, // TODO can we make it so DateTime can derive a ValueView?
}

impl From<&Markdown> for MetaContext {
    fn from(markdown: &Markdown) -> Self {
        Self {
            title: markdown.frontmatter.title.clone(),
            timestamp: markdown.frontmatter.timestamp.to_rfc3339(),
        }
    }
}

impl Renderer {
    pub fn new(config: &Config) -> Self {
        let layouts = collect_layouts(&config.layout_dir());
        Self {
            parser: ParserBuilder::with_stdlib()
                .partials(collect_partials(&env::current_dir().unwrap()))
                .build()
                .unwrap(),
            layouts,
        }
    }

    pub fn render(&self, markdown: &Markdown) -> RenderResult<String> {
        let layout = self.layouts.get("primary").expect("missing primary layout");
        let template = self.parser.parse(layout).unwrap();
        let globals = liquid::object!({
            "meta": MetaContext::from(markdown),
            "content": self.render_content(markdown)?,
        });
        // TODO better not to discard the info from here
        template.render(&globals).map_err(|e| e.into())
    }

    fn render_content(&self, markdown: &Markdown) -> RenderResult<String> {
        // TODO `{% render 'filename' for array as item %}` this seems like a useful construct
        let template = self
            .parser
            .parse(
                r#"
{% for block in blocks %}
    <h1>{{ block }}</h1>
{% endfor %}
        "#,
            )
            .unwrap();
        let globals = liquid::object!({
            "blocks": markdown.blocks.iter().map(|tokens| join_tokens(tokens)).collect::<Vec<_>>(),
        });
        // TODO better not to discard the info from here
        template.render(&globals).map_err(|e| e.into())
    }
}

// TODO temporary debug code
fn join_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|token| match token {
            Token::Literal(s) => s.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("")
}
