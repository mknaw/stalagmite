use std::collections::HashMap;
use std::path::Path;

use nom::branch::alt;
use nom::bytes::complete::{tag, take, take_until, take_while};
use nom::character::complete::{char, newline, not_line_ending};
use nom::combinator::{map, opt};
use nom::sequence::delimited;
use nom::IResult;
use thiserror::Error;

use crate::pages::{Block, FrontMatter, Token};

type MarkdownResult<T> = Result<T, MarkdownError>;

#[derive(Error, Debug)]
pub enum MarkdownError {
    #[error("io error")]
    IoError(#[from] std::io::Error),
    #[error("parsing error")]
    ParseError,
}

/// Extract the relevant bits from the `contents` of a `.md` file
fn parse_markdown(contents: &str) -> IResult<&str, (FrontMatter, Vec<Block>)> {
    let input = contents.trim();
    let (input, frontmatter_raw) = delimited(tag("---"), take_until("---"), tag("---"))(input)?;
    // TODO lightly gross to first allocate the `HashMap` just to try to supply some programmatic
    // defaults to the `FrontMatter` struct. Maybe there's a better way
    let frontmatter: HashMap<&str, &str> = serde_yaml::from_str(frontmatter_raw).unwrap();
    let blocks = parse_body(input);
    // TODO this fn doesn't really have to return an IResult ... we don't care about rest of str
    Ok((input, (frontmatter.try_into().unwrap(), blocks)))
}

/// Parse a `.md` file containing some post
pub fn parse_markdown_file(path: &Path) -> MarkdownResult<(FrontMatter, Vec<Block>)> {
    let contents = std::fs::read_to_string(path)?;
    let (_, (frontmatter, blocks)) =
        parse_markdown(&contents).map_err(|_| MarkdownError::ParseError)?;
    Ok((frontmatter, blocks))
}

/// Parse out the `body` of the post, which is composed of `Block`s.
fn parse_body(input: &str) -> Vec<Block> {
    // TODO probably should be doing this with `nom` too but I can't be buggered right now.
    input.trim().split("\n\n").map(parse_block).collect()
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

/// Parse a single chunk of text into a `Block`.
fn parse_block(content: &str) -> Block {
    // TODO nested block support, like links etc.
    let (content, kind) = opt(parse_kind)(content).unwrap();
    Block {
        kind: kind.map_or("p".to_string(), |k| k.to_string()),
        tokens: parse_inner(content),
    }
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
            }),
        ))
    }
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
    fn parse_markdown_test() {
        let input = r#"
---
title: Excellent Blog Post
timestamp: 2023-10-21T10:00:00-05:00
---
First, I'd like to start with this paragraph.

Then I'd like to add another paragraph.
        "#;
        let (_, (frontmatter, mut blocks)) = parse_markdown(input).unwrap();
        assert_eq!(&frontmatter.title, "Excellent Blog Post",);
        assert_eq!(
            frontmatter.timestamp,
            Utc.with_ymd_and_hms(2023, 10, 21, 15, 0, 0).unwrap(),
        );
        assert_eq!(&frontmatter.slug, "excellent-blog-post");

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
    fn parse_parse_nested_special_token() {
        let input = "`code is here`";
        let parser = parse_nested_special_token('`', "code");
        let (_, token) = opt(parser)(input).unwrap();
        assert_eq!(
            token,
            Some(Token::Block(Block {
                kind: "code".to_string(),
                tokens: vec![Token::Literal("code is here".to_string())],
            })),
        );
    }
}
