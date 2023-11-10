use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

/// Use `railwind` to generate our tailwind CSS file.
pub fn generate_css(input: &[PathBuf]) {
    let mut warnings = vec![];

    let source_options = input
        .iter()
        .map(|i| railwind::SourceOptions {
            input: i,
            option: railwind::CollectionOptions::Html,
        })
        .collect();

    let css = railwind::parse_to_string(
        railwind::Source::Files(source_options),
        // TODO should get this from a config option, perhaps?
        true, // Whether to include tailwind preflight
        &mut warnings,
    );

    // TODO cache-bust with name
    let mut css_file = File::create("./public/styles.css").unwrap();
    css_file.write_all(css.as_bytes()).unwrap();

    // TODO want to do something more useful with these warnings?
    for warning in warnings {
        println!("{}", warning)
    }
}
