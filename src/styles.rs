use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StyleError {
    #[error("railwind error")]
    RailwindError,
}

/// Use `railwind` to generate our tailwind CSS file.
pub fn generate_css<P: AsRef<Path>>(
    input: &[PathBuf],
    minify: bool,
    out_dir: P,
) -> Result<String, StyleError> {
    let mut warnings = vec![];

    let source_options = input
        .iter()
        .map(|i| railwind::SourceOptions {
            input: i,
            option: railwind::CollectionOptions::Html,
        })
        .collect();

    let raw = railwind::parse_to_string(
        railwind::Source::Files(source_options),
        // TODO should get this from a config option, perhaps?
        true, // Whether to include tailwind preflight
        &mut warnings,
    );
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

    // Hash contents for cache-busting.
    // TODO probably will want something more general for cache-busting...
    // will also want to cache-bust imgs, js, etc.
    let filename = format!("tw.{:016x}.css", seahash::hash(css));

    let static_dir = out_dir.as_ref().join("static");
    std::fs::create_dir_all(&static_dir).unwrap();
    let out_path = static_dir.join(&filename);
    let mut css_file = File::create(out_path).unwrap();
    css_file.write_all(css).unwrap();

    // TODO want to do something more useful with these warnings?
    for warning in warnings {
        println!("{}", warning);
    }

    Ok(filename)
}
