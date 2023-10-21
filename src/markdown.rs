use std::collections::HashMap;
use std::path::PathBuf;

use chrono::prelude::*;
use nom::bytes::complete::tag;
use nom::character::complete::{alpha1, newline, not_line_ending};
use nom::multi::{many0, separated_list1};
use nom::sequence::delimited;
use nom::IResult;
use thiserror::Error;

use crate::utils::slugify;

type Paragraph = Vec<Token>;
type MarkdownResult<T> = Result<T, MarkdownError>;

#[derive(Error, Debug)]
pub enum MarkdownError {
    #[error("parsing error")]
    ParseError,
}

#[derive(Debug, PartialEq)]
pub enum Token {
    Literal(String),
}

#[derive(Debug, PartialEq)]
pub struct FrontMatter {
    pub title: String,
    pub timestamp: DateTime<Utc>,
    pub slug: String,
    // TODO want to be able to just pass a String tag rather than a filepath
    pub layout: Option<PathBuf>,
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
            layout: None,
        })
    }
}

#[derive(Debug)]
pub struct Markdown {
    pub frontmatter: FrontMatter,
    pub paragraphs: Vec<Paragraph>,
    // layout: PathBuf,
}

/// Parse a `.md` file containing some post
pub fn parse_markdown(input: &str) -> MarkdownResult<Markdown> {
    fn parse(input: &str) -> IResult<&str, Markdown> {
        let input = input.trim();
        let (input, frontmatter) = delimited(tag("---"), parse_frontmatter, tag("---"))(input)?;
        let paragraphs = parse_body(input);
        // TODO don't really need `layout` in frontmatter after this...
        // or maybe just should leave in frontmatter and not extract
        // let layout = if let Some(layout) = frontmatter.layout.as_ref().cloned() {
        //     layout
        // } else {
        //     PathBuf::from("default.liquid")
        // };
        let markdown = Markdown {
            frontmatter,
            paragraphs,
            // layout,
        };
        // TODO this fn doesn't really have to return an IResult ... we don't care about rest of str
        Ok((input, markdown))
    }
    parse(input).map_or_else(|_| Err(MarkdownError::ParseError), |(_, md)| Ok(md))
}

/// Parse out some arbitrary "foo: bar" + newline
fn parse_metadata_kv(input: &str) -> IResult<&str, (&str, &str)> {
    let (input, key) = alpha1(input)?;
    let (input, _) = tag(": ")(input)?;
    let (input, value) = not_line_ending(input)?;
    Ok((input, (key, value)))
}

/// Parse out the `frontmatter` metadata
fn parse_frontmatter(input: &str) -> IResult<&str, FrontMatter> {
    // TODO would have expected `separated_list1` to do this, so maybe I'm doing it wrong.
    let (input, _) = many0(newline)(input)?;
    let (input, frontmatter) =
        nom::combinator::map_res(separated_list1(newline, parse_metadata_kv), |kvs| {
            kvs.try_into()
        })(input)?;
    let (input, _) = many0(newline)(input)?;
    Ok((input, frontmatter))
}

/// Parse out the `body` of the post
fn parse_body(input: &str) -> Vec<Paragraph> {
    // TODO probably should be doing this with `nom` too but I can't be buggered right now.
    input
        .trim()
        .split("\n\n")
        .map(|p| vec![Token::Literal(p.to_string())])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_metadata_kv_test() {
        let input = "foo: bar";
        let (input, (key, val)) = parse_metadata_kv(input).unwrap();
        assert_eq!(input, "");
        assert_eq!(key, "foo");
        assert_eq!(val, "bar");
    }

    #[test]
    fn parse_markdown_test() {
        let input = r#"
---
title: Excellent Blog Post
timestamp: 2023-10-21T10:00:00-05:00
---
First, I'd like to start with this paragraph.

Then I'd like to add another paragraph.
        "#;
        let mut markdown = parse_markdown(input).unwrap();
        assert_eq!(&markdown.frontmatter.title, "Excellent Blog Post",);
        assert_eq!(
            markdown.frontmatter.timestamp,
            Utc.with_ymd_and_hms(2023, 10, 21, 15, 0, 0).unwrap(),
        );
        assert_eq!(&markdown.frontmatter.slug, "excellent-blog-post");

        let second_paragraph = markdown.paragraphs.pop().unwrap();
        let first_paragraph = markdown.paragraphs.pop().unwrap();
        assert!(markdown.paragraphs.is_empty());
        assert_eq!(
            first_paragraph,
            vec![Token::Literal(
                "First, I'd like to start with this paragraph.".to_string()
            )],
        );
        assert_eq!(
            second_paragraph,
            vec![Token::Literal(
                "Then I'd like to add another paragraph.".to_string()
            )],
        );
    }
}
