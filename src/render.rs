use std::fs;

use liquid::{Parser, ParserBuilder};
use thiserror::Error;

use crate::markdown::{Markdown, Token};
use crate::Config;

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("invalid layout file")]
    LiquidError,
}

type RenderResult<T> = Result<T, RenderError>;

pub struct Renderer {
    parser: Parser,
    base_template: String,
}

impl Renderer {
    pub fn new(config: &Config) -> Self {
        let layout = fs::read_to_string(&config.layout).unwrap();
        Self {
            parser: ParserBuilder::with_stdlib().build().unwrap(),
            base_template: layout,
        }
    }

    pub fn render(&self, markdown: &Markdown) -> RenderResult<String> {
        let template = self.parser.parse(&self.base_template).unwrap();
        let globals = liquid::object!({
            "title": markdown.frontmatter.title,
            "timestamp": markdown.frontmatter.timestamp,
            "content": self.render_content(markdown)?,
        });
        // TODO better not to discard the info from here
        template
            .render(&globals)
            .map_err(|_| RenderError::LiquidError)
    }

    fn render_content(&self, markdown: &Markdown) -> RenderResult<String> {
        // TODO
        let template = self
            .parser
            .parse(
                r#"
{% for paragraph in paragraphs %}
    <p>{{ paragraph }}</p>
{% endfor %}
        "#,
            )
            .unwrap();
        let globals = liquid::object!({
            "paragraphs": markdown.paragraphs.iter().map(|tokens| join_tokens(tokens)).collect::<Vec<_>>(),
        });
        // TODO better not to discard the info from here
        template
            .render(&globals)
            .map_err(|_| RenderError::LiquidError)
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
