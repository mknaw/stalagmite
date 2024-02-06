use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use railwind::warning::Position;
use railwind::ParsedClass;
use regex::Regex;
use thiserror::Error;

// TODO probably should be using seemingly better `tailwind-rs` instead of `railwind`.
// that comes with its own problems, like wanting HTML files to parse instead of
// just regexing the classes from the liquids. but it does other things better.

lazy_static! {
    // Silly copy-pasta we must do to work around overly restrictive public API of `railwind`.
    // TODO maybe parsing the HTML like tailwind-rs does is OK at this point?
    static ref HTML_CLASS_REGEX: Regex =
        Regex::new(r#"(?:class|className)=(?:["]\W+\s*(?:\w+)\()?["]([^"]+)["]"#).unwrap();
}

#[derive(Error, Debug)]
pub enum StyleError {}

#[derive(Debug)]
pub struct ClassCollector(pub HashSet<String>);

impl ClassCollector {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    pub fn insert(&mut self, class: String) {
        self.0.insert(class);
    }
}

pub fn collect_classes(html: &str, class_collector: &mut ClassCollector) {
    for captures in HTML_CLASS_REGEX.captures_iter(html) {
        let Some(group) = captures.get(1) else {
            continue;
        };
        for cap in group.as_str().split([' ', '\n']) {
            if !cap.is_empty() && (cap != "group") && (cap != "peer") {
                class_collector.insert(cap.to_string());
            }
        }
    }
}

// Copy pasta from `railwind` library. Unfortunately, the library isn't very flexible.
fn parse_classes(class_collector: &ClassCollector) -> Vec<ParsedClass> {
    let position = Position::new("", 0, 0);
    class_collector
        .0
        .iter()
        .filter_map(
            |raw_str| match ParsedClass::new_from_raw_class(raw_str, position.clone()) {
                Ok(c) => Some(c),
                Err(_) => None,
            },
        )
        .collect()
}

fn generate_strings(parsed_classes: Vec<ParsedClass>) -> Vec<String> {
    let mut out = Vec::new();

    for class in parsed_classes {
        if let Ok(class) = class.try_to_string() {
            out.push(class);
        }
    }

    out
}

/// Generate a name that includes a hash of the contents.
fn make_cache_busted_name(path: &Path, _contents: &[u8]) -> OsString {
    let stem = path.file_stem().unwrap();
    let ext = path.extension().unwrap();

    // TODO temporarily have to not cache-bust for development purposes until I figure out how to
    // do a "second pass" over liquid templates...
    // OsString::from(format!(
    //     "{}.{}.{}",
    //     stem.to_str().unwrap(),
    //     utils::hash(contents),
    //     ext.to_str().unwrap()
    // ))
    OsString::from(format!(
        "{}.{}",
        stem.to_str().unwrap(),
        ext.to_str().unwrap()
    ))
}

// TODO incremental CSS generation.
// with railwind, it will look something like this
// with tailwind-rs, it will also probably be similar - in both cases I think we will need
// to do the regexing ourselves - in railwind because it's not part of public API, and in
// tailwind-rs because there they don't regex but rather parse html, which we don't want,
// since we want to be able to hit liquid files with the generation before they are rendered.
// other problems with tailwind-rs are that it doesn't work at all (have to bump lightningcss)
// and have to verify that it can handle `hover:whatever`. but seems better in that it supports
// the theme directive from config, which is nice.
// fn incremental_generation() {
//     let class_strings: Vec<String> = vec![]; // need to be unique.
//     for class_string in class_strings.iter() {
//         let pc = ParsedClass::new_from_raw_class(class_string, Position::new("", 0, 0)).unwrap();
//         generated_classes append pc.try_to_string())
//     }
//     take those generated classes and make css file
// }

pub fn render_css<P: AsRef<Path>>(
    class_collector: ClassCollector,
    minify: bool,
    out_dir: P,
) -> Result<String, StyleError> {
    let parsed_classes = parse_classes(&class_collector);
    let styles = generate_strings(parsed_classes);
    let raw = styles.join("\n");
    let css = if minify {
        let stylesheet = StyleSheet::parse(&raw, ParserOptions::default()).unwrap();
        let printer_options = PrinterOptions {
            minify: true,
            ..Default::default()
        };
        stylesheet.to_css(printer_options).unwrap().code
    } else {
        raw
    };
    let css = css.as_bytes();

    // TODO wonder if something else should be used as the cache for this particular file...
    // this requires having iterated through all the files, so you can't do the CSS generation in
    // parallel. meanwhile we'd like to know the tailwind.css file name before we start rendering
    // templates, so we can programmatically list the correct name.
    let filename = make_cache_busted_name(Path::new("tw.css"), css);

    let static_dir = out_dir.as_ref().join("static");
    std::fs::create_dir_all(&static_dir).unwrap();
    let out_path = static_dir.join(&filename);
    let mut css_file = File::create(out_path).unwrap();
    css_file.write_all(css).unwrap();

    Ok(filename.to_str().unwrap().to_string())
}
