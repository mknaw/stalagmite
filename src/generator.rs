use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::{fs, thread};

use rayon::prelude::*;
// use rusqlite::Connection;
use tempdir::TempDir;

use crate::core::{FileType, FrontMatter, PageFile, PageIndex, RenderRuleSet, SiteNode};
use crate::markdown::{parse_blocks, parse_frontmatter};
use crate::{assets, diskio, Config, Markdown, Renderer};

// TODO this is unseemly, surely I can rewrite this in a nicer manner.
struct MarkdownPageInfo {
    items: Vec<Markdown>,
    dir_path: PathBuf,
    page_index: PageIndex,
}

// TODO is this a bit redundant, given that we've identified the `ftype` in `FileEntry`?
enum RenderChannelItem {
    // TODO try to not clone so much
    Html((PageFile, Arc<RenderRuleSet>)),
    Liquid((PageFile, Arc<RenderRuleSet>)),
    MarkdownSet((MarkdownPageInfo, Arc<RenderRuleSet>)),
}

// #[allow(dead_code)] // TODO will use this eventually
// fn open_db_connection() -> rusqlite::Result<Connection> {
//     // TODO obviously don't want in-memory!
//     let conn = Connection::open_in_memory()?;
//
//     // TODO this will be in a project initialization path
//     conn.execute(
//         "CREATE TABLE markdown (
//             id       INTEGER PRIMARY KEY,
//             name     TEXT NOT NULL,
//             hash     TEST NOT NULL,
//             contents BLOB NOT NULL
//         )",
//         (), // empty list of parameters.
//     )?;
//     Ok(conn)
// }

// #[allow(dead_code)] // TODO will use this eventually
// fn update_markdown_in_db(conn: &Connection, page: &Markdown) -> rusqlite::Result<()> {
//     let name = page.dir_path.to_path_buf().join(&page.frontmatter.slug);
//     conn.execute("INSERT INTO markdown (name) VALUES (?1)", (&name.to_str(),))?;
//     Ok(())
// }

// #[allow(dead_code)] // TODO will use this eventually
// fn read_markdowns_from_db(conn: &Connection) -> rusqlite::Result<()> {
//     let mut stmt = conn.prepare("SELECT name, contents FROM markdown")?;
//     let _results: Vec<(String, String)> = stmt
//         .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
//         .flatten()
//         .collect();
//     Ok(())
// }

/// TODO maybe this still should be in diskio territory, not sure.
fn forward_collected_entries(mut node: SiteNode, render_sink: Sender<RenderChannelItem>) {
    // TODO should this also be a sink? need to drop when we're done with this...
    let mut markdown_queue: Vec<PageFile> = vec![];

    for page_file in node.entries.drain(0..) {
        match page_file.ftype {
            FileType::Markdown => {
                markdown_queue.push(page_file);
            }
            FileType::Liquid => {
                render_sink
                    .send(RenderChannelItem::Liquid((page_file, node.rules.clone())))
                    .unwrap();
            }
            FileType::Html => {
                render_sink
                    .send(RenderChannelItem::Html((page_file, node.rules.clone())))
                    .unwrap();
            }
        }
    }

    process_markdowns(&node, markdown_queue, render_sink);
}

fn process_markdowns(
    node: &SiteNode,
    mut markdown_queue: Vec<PageFile>,
    render_sink: Sender<RenderChannelItem>,
) {
    if markdown_queue.is_empty() {
        return;
    }

    let mut markdown_queue: Vec<(FrontMatter, PageFile, usize)> = markdown_queue
        .drain(0..)
        .map(|page_file| {
            let contents = diskio::read_file_contents(&page_file.abs_path);
            // TODO if anything, probably want buffered reading, and not Mmap -> `[..]` slice.
            let (frontmatter, offset) = parse_frontmatter(&contents[..]).unwrap();
            (frontmatter, page_file, offset)
        })
        .collect();

    // TODO(low priority): support other order bys
    markdown_queue.sort_by_key(|item| -item.0.timestamp.timestamp());

    let page_size = if let Some(listing_rule_set) = node.rules.listing.as_ref() {
        listing_rule_set.page_size.unwrap_or(10)
    } else {
        1
    };

    // TODO don't really need this if not paginating...
    let md_count = markdown_queue.len();

    // Round down OK because page numbers are 0-indexed.
    // TODO check the math on 1 full page.
    // TODO ensure not md_count == 0... would be bad config but whatever
    let page_count = (md_count - 1) / page_size;

    markdown_queue
        .chunks(page_size)
        .enumerate()
        .for_each(|(page_index, chunk)| {
            let pages: Vec<Markdown> = chunk
                .iter()
                .map(|(frontmatter, page_file, offset)| {
                    // TODO try to rework to avoid the `.clone()`s here.
                    let contents = diskio::read_file_contents(&page_file.abs_path);
                    Markdown {
                        dir_path: node.dir.clone(),
                        frontmatter: frontmatter.clone(),
                        blocks: parse_blocks(&contents[(*offset)..]),
                    }
                })
                .collect();
            // TODO ought to be a cleaner way to get this?
            let render_info = MarkdownPageInfo {
                items: pages,
                dir_path: node.dir.clone(),
                page_index: (page_index, page_count),
            };
            render_sink
                .send(RenderChannelItem::MarkdownSet((
                    render_info,
                    node.rules.clone(),
                )))
                .unwrap();
        });
}

fn delegate_rendering(renderer: &Renderer, sink: Receiver<RenderChannelItem>, out_dir: &Path) {
    sink.into_iter().par_bridge().for_each(|item| match item {
        RenderChannelItem::MarkdownSet((info, render_rules)) => {
            // This one's the most complicated because it may be necessary to render a listing page.
            let MarkdownPageInfo {
                items: pages,
                dir_path,
                page_index,
            } = info;

            if render_rules.should_render_list() {
                let rendered = renderer
                    .render_listing_page(&pages, &render_rules, page_index)
                    .unwrap();
                let path = dir_path.join(format!("{}", page_index.0));
                diskio::write_html(out_dir, path, &rendered);
            }

            pages.into_iter().for_each(|page| {
                let rendered = renderer
                    .render_markdown(&page, &render_rules)
                    .expect("rendering failed");
                // TODO probably also want to minify + compress
                diskio::write_html(out_dir, dir_path.join(&page.frontmatter.slug), &rendered);
            });
        }
        // TODO should use these `render_rules`
        RenderChannelItem::Liquid((page_file, _render_rules)) => {
            let rendered = renderer
                .render(&page_file.abs_path)
                .expect("rendering failed");
            diskio::write_html(out_dir, &page_file.rel_path, &rendered);
        }
        RenderChannelItem::Html((file_entry, render_rules)) => {
            let html = fs::read_to_string(&file_entry.abs_path).unwrap();
            let rendered = renderer
                .render_html(&html, &render_rules)
                .unwrap_or_else(|_| panic!("rendering failed: {:?}", &file_entry.abs_path));
            // TODO only `rel_dir()` for `index.html`?
            diskio::write_html(out_dir, file_entry.rel_dir(), &rendered);
        }
    });
}

// TODO we already do this walking once in `collect_template_map`, so don't do it again here!
// TODO layouts alone will not suffice - also need all the .html and .liquid in pages.
fn get_all_liquids(dir: &Path) -> Vec<PathBuf> {
    // TODO stuff like this should be parallelizable..
    diskio::walk(dir, "liquid").collect()
}

pub fn generate() {
    // let conn = open_db_connection().expect("failed to open db connection");
    let config = Arc::new(Config::init().map_or_else(|e| panic!("{}", e), |c| c));

    let (collection_sender, collection_sink) = channel::<SiteNode>();

    // TODO `rayon`ize for better parallelism
    let config_clone = config.clone();
    thread::spawn(move || {
        diskio::collect_generation_nodes(config_clone, collection_sender);
    });

    // TODO ought to have some type aliases here.
    let (render_sender, render_sink) = channel::<RenderChannelItem>();

    thread::spawn(move || {
        for node in collection_sink {
            forward_collected_entries(node, render_sender.clone());
        }
    });

    let out_dir = TempDir::new("stalagmite_out").unwrap();
    // TODO css generation can be done in parallel to template rendering.
    let tailwind_filename =
        assets::generate_css(&get_all_liquids(&config.current_dir), true, &out_dir).unwrap();

    // TODO wonder if `Renderer` should just take the `out_dir` as well.
    let renderer = Renderer::new(&config, tailwind_filename);
    delegate_rendering(&renderer, render_sink, out_dir.path());

    // Replace the old output directory with the new one.
    std::fs::remove_dir_all(config.out_dir()).unwrap();
    std::fs::rename(out_dir, config.out_dir()).unwrap();

    tracing::info!("static site generated!");
}
