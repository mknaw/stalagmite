use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::{fs, thread};

use memmap2::Mmap;
use rayon::prelude::*;
use rusqlite::Connection;
use tempdir::TempDir;

use crate::core::{Block, FileType, FrontMatter, GenerationNode, PageIndex, RenderRuleSet};
use crate::markdown::{parse_blocks, parse_frontmatter};
use crate::{diskio, styles, Config, MarkdownPage, Renderer};

type MarkdownQueueItem<'a> = (FrontMatter, &'a Path, &'a str, usize);

// TODO this is somewhat unseemly, surely I can rewrite this in a nicer manner.
struct MarkdownRenderInfo {
    pages: Vec<MarkdownPage>,
    render_rules: Arc<RenderRuleSet>,
    dir_path: String,
    page_index: PageIndex,
}

enum RenderChannelItem {
    // TODO try to not clone so much
    Html((PathBuf, PathBuf)),
    Markdown(MarkdownRenderInfo),
}

#[allow(dead_code)] // TODO will use this eventually
fn open_db_connection() -> rusqlite::Result<Connection> {
    // TODO obviously don't want in-memory!
    let conn = Connection::open_in_memory()?;

    // TODO this will be in a project initialization path
    conn.execute(
        "CREATE TABLE markdown (
            id       INTEGER PRIMARY KEY,
            name     TEXT NOT NULL,
            hash     TEST NOT NULL,
            contents BLOB NOT NULL
        )",
        (), // empty list of parameters.
    )?;
    Ok(conn)
}

#[allow(dead_code)] // TODO will use this eventually
fn update_markdown_in_db(conn: &Connection, page: &MarkdownPage) -> rusqlite::Result<()> {
    let name = page.dir_path.to_path_buf().join(&page.frontmatter.slug);
    conn.execute("INSERT INTO markdown (name) VALUES (?1)", (&name.to_str(),))?;
    Ok(())
}

#[allow(dead_code)] // TODO will use this eventually
fn read_markdowns_from_db(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT name, contents FROM markdown")?;
    let _results: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .flatten()
        .collect();
    Ok(())
}

/// TODO maybe this still should be in diskio territory, not sure.
fn process_generation_node(node: GenerationNode, render_sink: Sender<RenderChannelItem>) {
    // TODO probably will want an actual type around this.
    let mut markdown_queue: Vec<MarkdownQueueItem> = vec![];

    for file_entry in node.entries.iter() {
        match file_entry.ftype {
            FileType::Markdown => {
                let contents = read_file_contents(&file_entry.abs_path);
                let (frontmatter, offset) = parse_frontmatter(&contents[..]).unwrap();
                markdown_queue.push((frontmatter, &file_entry.abs_path, &node.dir_path, offset));
            }
            FileType::Liquid => unimplemented!(), // TODO
            FileType::Html => {
                render_sink
                    .send(RenderChannelItem::Html((
                        file_entry.abs_path.clone(),
                        file_entry.rel_path.clone(),
                    )))
                    .unwrap();
            }
        }
    }
    if !markdown_queue.is_empty() {
        process_markdown_queue(&node, &mut markdown_queue, render_sink);
    }
}

fn process_markdown_queue(
    node: &GenerationNode,
    markdown_queue: &mut [MarkdownQueueItem],
    render_sink: Sender<RenderChannelItem>,
) {
    // TODO(low priority): support other order bys
    markdown_queue.sort_by_key(|(frontmatter, _, _, _)| -frontmatter.timestamp.timestamp());

    let page_size = if let Some(listing_rule_set) = node.rules.listing.as_ref() {
        listing_rule_set.page_size.unwrap_or(10)
    } else {
        1
    };

    // TODO don't really need this if not paginating...
    let md_count = node
        .entries
        .iter()
        .filter(|file_entry| matches!(file_entry.ftype, FileType::Markdown))
        .count();
    // Round down OK because page numbers are 0-indexed.
    // TODO check the math on 1 full page.
    // TODO ensure not md_count == 0... would be bad config but whatever
    let page_count = (md_count - 1) / page_size;

    markdown_queue
        .chunks(page_size)
        .enumerate()
        .for_each(|(page_index, chunk)| {
            let pages: Vec<MarkdownPage> = chunk
                .iter()
                .map(|(frontmatter, path, dir_path, offset)| {
                    let blocks = read_blocks(path, *offset);
                    MarkdownPage {
                        dir_path: dir_path.into(),
                        // TODO certainly not crazy about the clone here!
                        frontmatter: frontmatter.clone(),
                        blocks,
                    }
                })
                .collect();
            let dir_path = chunk[0].2; // TODO this is extremely hack, and just a quick solution
            let render_info = MarkdownRenderInfo {
                pages,
                render_rules: node.rules.clone(),
                dir_path: dir_path.into(),
                page_index: (page_index, page_count),
            };
            render_sink
                .send(RenderChannelItem::Markdown(render_info))
                .unwrap();
        });
}

// TODO now this _definitely_ should be in diskio
fn read_file_contents<P: AsRef<Path>>(path: P) -> Mmap {
    let file = fs::File::open(path).unwrap();
    unsafe { Mmap::map(&file).unwrap() }
}

fn read_blocks<P: AsRef<Path>>(path: P, offset: usize) -> Vec<Block> {
    let contents = read_file_contents(path);
    parse_blocks(&contents[offset..])
}

// TODO we already do this walking once in `collect_template_map`, so don't do it again here!
// TODO layouts alone will not suffice - also need all the .html and .liquid in pages.
fn get_all_liquids(dir: &Path) -> Vec<PathBuf> {
    // TODO stuff like this should be parallelizable..
    diskio::walk(dir, "liquid").collect()
}

fn render_markdown<P: AsRef<Path>>(renderer: &Renderer, out_dir: P, info: MarkdownRenderInfo) {
    let MarkdownRenderInfo {
        pages,
        render_rules,
        dir_path,
        page_index,
    } = info;
    let mut html_paths = vec![];
    if render_rules.should_render_listing() {
        let rendered = renderer
            .render_listing_page(&pages, &render_rules, page_index)
            .unwrap();
        let mut path: PathBuf = dir_path.into();
        path = path.join(format!("{}", page_index.0));
        html_paths.push(diskio::write_html(&out_dir, &path, &rendered));
    }
    pages.into_iter().for_each(|page| {
        let rendered = renderer
            .render(&page, &render_rules)
            .expect("rendering failed");
        // TODO probably also want to minify + compress
        html_paths.push(diskio::write_html(
            &out_dir,
            page.dir_path.join(&page.frontmatter.slug),
            &rendered,
        ));
    });
}

pub fn generate() {
    // let conn = open_db_connection().expect("failed to open db connection");
    let config = Arc::new(Config::init().map_or_else(|e| panic!("{}", e), |c| c));

    let (collection_sender, collection_sink) = channel::<GenerationNode>();

    // TODO `rayon`ize for better parallelism
    let config_clone = config.clone();
    thread::spawn(move || {
        diskio::collect_generation_nodes(config_clone, collection_sender);
    });

    // TODO ought to have some type aliases here.
    let (render_sender, render_sink) = channel::<RenderChannelItem>();

    thread::spawn(move || {
        for node in collection_sink {
            process_generation_node(node, render_sender.clone());
        }
    });

    let out_dir = TempDir::new("stalagmite_out").unwrap();
    let liquids = get_all_liquids(&config.current_dir);
    // TODO css generation can be done in parallel to template rendering.
    let tailwind_filename = styles::generate_css(&liquids, true, &out_dir).unwrap();

    // TODO wonder if `Renderer` should just take the `out_dir` as well.
    let renderer = Renderer::new(&config, tailwind_filename);

    // TODO name is stupid because it's not even only rendering.
    // Also it seems like in fact only the rendering bits should be hit with `rayon`,
    // as they are the must CPU-intensive.
    render_sink
        .into_iter()
        .par_bridge()
        .for_each(|item| match item {
            RenderChannelItem::Markdown(info) => {
                render_markdown(&renderer, &out_dir, info);
            }
            RenderChannelItem::Html((abs_path, rel_path)) => {
                let out_path = out_dir.as_ref().join(rel_path);
                std::fs::copy(abs_path, out_path).unwrap();
            }
        });

    // Replace the old output directory with the new one.
    std::fs::remove_dir_all(config.out_dir()).unwrap();
    std::fs::rename(out_dir, config.out_dir()).unwrap();

    tracing::info!("static site generated!");
}
