use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use thiserror::Error;

// TODO probably should be using seemingly better `tailwind-rs` instead of `railwind`.
// that comes with its own problems, like wanting HTML files to parse instead of
// just regexing the classes from the liquids. but it does other things better.

#[derive(Error, Debug)]
pub enum StyleError {}

/// Generate a name that includes a hash of the contents.
fn make_cache_busted_name(path: &Path, contents: &[u8]) -> OsString {
    let stem = path.file_stem().unwrap();
    let hash = seahash::hash(contents);
    let ext = path.extension().unwrap();

    OsString::from(format!(
        "{}.{:016x}.{}",
        stem.to_str().unwrap(),
        hash,
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

    // TODO all the fancy shit I do with parallelization and limiting disk io is
    // for naught with this blocking call that iterates and reads all the files anew.
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

    // TODO want to do something more useful with these warnings?
    for warning in warnings {
        tracing::warn!("{}", warning);
    }

    Ok(filename.to_str().unwrap().to_string())
}
