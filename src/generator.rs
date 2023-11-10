use crate::{diskio, styles, Config, Renderer};

pub fn generate() {
    let config = Config::init().map_or_else(|e| panic!("{}", e), |c| c);
    let renderer = Renderer::new(&config);
    let pages = diskio::collect_pages(&config);
    let html_paths: Vec<_> = pages
        .iter()
        .map(|page| {
            let rendered = renderer.render(page).expect("rendering failed");
            // TODO would be good to generate in tempdir so failure could be atomic?
            diskio::write_html(&page.path, &page.frontmatter.slug, &rendered)
        })
        .collect();
    styles::generate_css(&html_paths);
    println!("static site generated!");
}
