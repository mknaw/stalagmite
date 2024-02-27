use std::collections::HashMap;

use nom::branch::alt;
use nom::bytes::complete::{tag, take, take_until, take_while};
use nom::character::complete::{char, newline, not_line_ending};
use nom::combinator::{map, opt};
use nom::multi::many0;
use nom::sequence::delimited;
use nom::IResult;
use thiserror::Error;

use crate::common::{Block, FrontMatter, Token};
use crate::Markdown;

type MarkdownResult<T> = Result<T, MarkdownError>;

#[derive(Error, Debug)]
pub enum MarkdownError {
    #[error("io error")]
    IoError(#[from] std::io::Error),
    #[error("parsing error")]
    ParseError,
}

/// Parse a `Markdown` struct from a `&str` of .md contents.
pub fn parse(contents: &str) -> MarkdownResult<Markdown> {
    let (frontmatter, offset) = parse_frontmatter(contents)?;
    let blocks = parse_blocks(&contents[offset..]);
    Ok(Markdown {
        frontmatter,
        blocks,
    })
}

/// Parse the `FrontMatter` section of the contents.
pub fn parse_frontmatter(contents: &str) -> MarkdownResult<(FrontMatter, usize)> {
    fn parse(input: &str) -> IResult<&str, FrontMatter> {
        let (input, _) = many0(newline)(input)?;
        let (input, frontmatter_raw) = delimited(tag("---"), take_until("---"), tag("---"))(input)?;
        let (input, _) = many0(newline)(input)?;
        // TODO lightly gross to first allocate the `HashMap` just to try to supply some programmatic
        // defaults to the `FrontMatter` struct. Maybe there's a better way
        let frontmatter: HashMap<&str, &str> = serde_yaml::from_str(frontmatter_raw).unwrap();
        Ok((input, frontmatter.try_into().unwrap()))
    }
    let total_len = contents.len();
    let (remaining, frontmatter) = parse(contents).map_err(|_| MarkdownError::ParseError)?;
    let offset = total_len - remaining.len();
    Ok((frontmatter, offset))
}

/// Parse out the `body` of the post, which is composed of `Block`s.
pub fn parse_blocks(contents: &str) -> Vec<Block> {
    contents.trim().split("\n\n").map(parse_block).collect()
}

/// Parse a single chunk of text into a `Block`.
fn parse_block(content: &str) -> Block {
    // TODO nested block support, like links etc.
    let (content, kind) = opt(parse_kind)(content).unwrap();
    Block {
        kind: kind.map_or("p".to_string(), |k| k.to_string()),
        tokens: parse_inner(content),
        meta: None,
    }
}

/// Try parsing non-default `Block` `kind`s, which determine the rendering of the block.
fn parse_kind(input: &str) -> IResult<&str, &str> {
    let (input, kind) = alt((
        map(tag("# "), |_| "h1"),
        map(tag("## "), |_| "h2"),
        map(tag("### "), |_| "h3"),
        map(tag("#### "), |_| "h4"),
        map(tag("##### "), |_| "h5"),
        map(tag("###### "), |_| "h6"),
        |input| {
            // Custom block kinds are declared like `~kind`.
            let (input, _) = tag("~:")(input)?;
            // TODO this shouldn't have any spaces in it, either.
            let (input, kind) = not_line_ending(input)?;
            let (input, _) = newline(input)?;
            Ok((input, kind))
        },
    ))(input)?;
    Ok((input, kind))
}

fn parse_nested_special_token<'a>(
    delimiter: char,
    kind: &'a str,
) -> impl nom::Parser<&'a str, Token, nom::error::Error<&'a str>> {
    move |input: &'a str| {
        let (input, _) = char(delimiter)(input)?;
        let (input, content) = take_while(|c| c != delimiter)(input)?;
        let tokens = parse_inner(content);
        let (input, _) = char(delimiter)(input)?;
        Ok((
            input,
            Token::Block(Block {
                kind: kind.to_string(),
                tokens,
                meta: None,
            }),
        ))
    }
}

fn parse_link_tag(input: &str) -> IResult<&str, Token> {
    let (input, _) = char('[')(input)?;
    let (input, content) = take_while(|c| c != ']')(input)?;
    let (input, _) = char(']')(input)?;
    let (input, _) = char('(')(input)?;
    let (input, url) = take_while(|c| c != ')')(input)?;
    let (input, _) = char(')')(input)?;
    Ok((
        input,
        Token::Block(Block {
            kind: "a".to_string(),
            tokens: vec![Token::Literal(content.to_string())],
            meta: Some(HashMap::from_iter(vec![(
                "href".to_string(),
                url.to_string(),
            )])),
        }),
    ))
}

// TODO this looks terrible, but I gues it works for now.
fn parse_inner(content: &str) -> Vec<Token> {
    let mut content = content;
    let mut result = vec![];
    let mut literal = String::new();
    while !content.is_empty() {
        let (rest, nested) = opt(alt((
            parse_nested_special_token('`', "code"),
            parse_nested_special_token('*', "b"),
            parse_nested_special_token('_', "i"),
            parse_link_tag,
        )))(content)
        .unwrap();
        content = if let Some(nested) = nested {
            if !literal.is_empty() {
                result.push(Token::Literal(literal));
                literal = String::new();
            }
            result.push(nested);
            rest
        } else {
            let (rest, next) = take::<usize, &str, nom::error::Error<&str>>(1usize)(rest).unwrap();
            literal.push_str(next);
            rest
        };
    }
    if !literal.is_empty() {
        result.push(Token::Literal(literal));
    }
    result
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    #[test]
    fn test_parse_markdown() {
        let input = r#"
---
title: Excellent Blog Post
timestamp: 2023-10-21T10:00:00-05:00
---
First, I'd like to start with this paragraph.

Then I'd like to add another paragraph.
        "#;
        let (frontmatter, offset) = parse_frontmatter(input).unwrap();
        assert_eq!(&frontmatter.title, "Excellent Blog Post",);
        assert_eq!(
            frontmatter.timestamp,
            Utc.with_ymd_and_hms(2023, 10, 21, 15, 0, 0).unwrap(),
        );
        assert_eq!(&frontmatter.slug, "excellent-blog-post");

        let mut blocks = parse_blocks(&input[offset..]);

        let second_paragraph = blocks.pop().unwrap();
        let first_paragraph = blocks.pop().unwrap();
        assert!(blocks.is_empty());
        assert_eq!(
            first_paragraph.tokens,
            vec![Token::Literal(
                "First, I'd like to start with this paragraph.".to_string()
            )],
        );
        assert_eq!(
            second_paragraph.tokens,
            vec![Token::Literal(
                "Then I'd like to add another paragraph.".to_string()
            )],
        );
    }

    #[test]
    fn test_parse_nested_special_token() {
        let input = "`code is here`";
        let parser = parse_nested_special_token('`', "code");
        let (_, token) = opt(parser)(input).unwrap();
        assert_eq!(
            token,
            Some(Token::Block(Block {
                kind: "code".to_string(),
                tokens: vec![Token::Literal("code is here".to_string())],
                meta: None,
            })),
        );
    }

    #[test]
    fn test_parse_link_tag() {
        let input = "This is a [link](https://example.com).";
        let tokens = parse_inner(input);
        assert_eq!(
            tokens,
            vec![
                Token::Literal("This is a ".to_string()),
                Token::Block(Block {
                    kind: "a".to_string(),
                    tokens: vec![Token::Literal("link".to_string())],
                    meta: Some(HashMap::from_iter(vec![(
                        "href".to_string(),
                        "https://example.com".to_string()
                    )])),
                }),
                Token::Literal(".".to_string())
            ],
        );
    }
}
