use std::fs;

use crate::{diskio, parse_markdown, styles, Config, Renderer};

pub fn generate() {
    let config = Config::init().map_or_else(|e| panic!("{}", e), |c| c);
    let renderer = Renderer::new(&config);
    // let layout_map = diskio::collect_layouts(&config.layout);
    let html_paths = diskio::walk_markdowns()
        .map(|p| {
            println!("{:?}", p);
            let raw = fs::read_to_string(p).unwrap();
            let markdown = parse_markdown(&raw).unwrap();
            // TODO have to check no slug collisions at this point too.
            let rendered = renderer.render(&markdown).unwrap();
            // TODO would be good to generate in tempdir so failure could be atomic?
            diskio::write_html(&markdown.frontmatter.slug, &rendered)
        })
        .collect::<Vec<_>>();
    styles::generate_css(&html_paths);
    println!("static site generated!");
}
