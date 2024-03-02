use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::ops::Deref;
use std::path::Path;

use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use railwind::warning::Position;
use railwind::ParsedClass;
use regex::Regex;
use thiserror::Error;

use crate::{cache, diskio, utils, Config};

// TODO probably should be using seemingly better `tailwind-rs` instead of `railwind`.
// that comes with its own problems, like wanting HTML files to parse instead of
// just regexing the classes from the liquids. but it does other things better.

lazy_static! {
    // Silly copy-pasta we must do to work around overly restrictive public API of `railwind`.
    // TODO maybe parsing the HTML like tailwind-rs does is OK at this point?
    static ref HTML_CLASS_REGEX: Regex =
        Regex::new(r#"(?:class|className)=(?:["]\W+\s*(?:\w+)\()?["]([^"]+)["]"#).unwrap();
}

pub const TAILWIND_FILENAME: &str = "tw.css";

pub type AssetMap = HashMap<String, String>;

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
fn parse_classes<'a>(sorted_classes: &'a [&'a String]) -> Vec<ParsedClass<'a>> {
    let position = Position::new("", 0, 0);
    sorted_classes
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
fn make_cache_busted_name(path: &Path, hash: &str) -> OsString {
    let stem = path.file_stem().unwrap();
    let ext = path.extension().unwrap();

    OsString::from(format!(
        "{}.{}.{}",
        stem.to_str().unwrap(),
        hash,
        ext.to_str().unwrap()
    ))
}

/// Render the CSS and write it to the output directory.
pub fn render_css<P: AsRef<Path>>(
    base_name: &str,
    class_collector: ClassCollector,
    minify: bool,
    out_dir: P,
) -> Result<String, StyleError> {
    // With our cache-busting technique, it's important to produce deterministic results,
    // so we sort the classes before hashing them.
    let mut sorted_classes = class_collector.0.iter().collect::<Vec<_>>();
    sorted_classes.sort();
    let parsed_classes = parse_classes(&sorted_classes);
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

    // TODO ultimately everything from here on out should be pretty similar to `collect` below.

    let css = css.as_bytes();

    // TODO wonder if something else should be used as the cache for this particular file...
    // this requires having iterated through all the files, so you can't do the CSS generation in
    // parallel. meanwhile we'd like to know the tailwind.css file name before we start rendering
    // templates, so we can programmatically list the correct name.
    let hash = utils::hash(css);
    let filename = make_cache_busted_name(Path::new(base_name), &hash);

    let static_dir = out_dir.as_ref().join("static");
    std::fs::create_dir_all(&static_dir).unwrap();
    let out_path = static_dir.join(&filename);
    let mut css_file = File::create(out_path).unwrap();
    css_file.write_all(css).unwrap();

    Ok(filename.to_str().unwrap().to_string())
}

pub fn collect<C: Deref<Target = Config>, P: AsRef<Path>>(
    config: &C,
    staging_dir: P,
    conn: &rusqlite::Connection,
) -> anyhow::Result<(AssetMap, bool)> {
    let mut static_asset_map = HashMap::new();
    let assets_dir = config.assets_dir();
    let mut changed = false;
    for path_buf in diskio::walk(&assets_dir, &None) {
        let contents = diskio::read_file_contents(&path_buf);
        let hash = utils::hash(&contents);
        let name = make_cache_busted_name(path_buf.as_path().as_std_path(), &hash);
        let alias = path_buf.strip_prefix(&assets_dir)?.to_string();
        let out = staging_dir
            .as_ref()
            .join("static")
            .join(&alias)
            .with_file_name(&name);
        fs::create_dir_all(out.parent().unwrap())?;
        fs::copy(&path_buf, out)?;
        changed |= cache::check_asset_changed(conn, &alias, &hash)?;
        static_asset_map.insert(alias, name.to_string_lossy().to_string());
    }
    Ok((static_asset_map, changed))
}
