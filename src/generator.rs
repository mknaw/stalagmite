use crate::{diskio, parse_markdown_file, styles, Config, Renderer};

pub fn generate() {
    let config = Config::init().map_or_else(|e| panic!("{}", e), |c| c);
    let renderer = Renderer::new(&config);
    let html_paths = diskio::walk_markdowns(&config.current_dir)
        .map(|path| {
            println!("{:?}", path);
            // TODO want to check no slug collisions too.
            let markdown =
                parse_markdown_file(path.strip_prefix(&config.current_dir).unwrap()).unwrap();
            let rendered = renderer.render(&markdown).expect("rendering failed");
            // TODO would be good to generate in tempdir so failure could be atomic?
            diskio::write_html(&markdown.path, &markdown.frontmatter.slug, &rendered)
        })
        .collect::<Vec<_>>();
    styles::generate_css(&html_paths);
    println!("static site generated!");
}
