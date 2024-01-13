use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use anyhow::anyhow;
use rayon::prelude::*;
use tempdir::TempDir;
use tracing::instrument;

use crate::core::*;
use crate::parsers::markdown;
use crate::{assets, cache, diskio, Config, Renderer};

// TODO this is unseemly, surely I can rewrite this in a nicer manner.
// struct MarkdownPageInfo {
//     items: Vec<Markdown>,
//     dir_path: PathBuf,
//     page_index: PageIndex,
// }
//
// enum RenderChannelItem {
//     // TODO try to not clone so much
//     Html((PageFile, Arc<RenderRules>)),
//     Liquid((PageFile, Arc<RenderRules>)),
//     MarkdownSet((MarkdownPageInfo, Arc<RenderRules>)),
// }

/// TODO maybe this still should be in diskio territory, not sure.
// fn forward_collected_entries(mut node: SiteNode, render_sink: Sender<RenderChannelItem>) {
//     // TODO should this also be a sink? need to drop when we're done with this...
//     let mut markdown_queue: Vec<PageFile> = vec![];
//
//     for page_file in node.page_files.drain(0..) {
//         match page_file.get_page_type() {
//             PageType::Markdown => {
//                 markdown_queue.push(page_file);
//             }
//             PageType::Liquid => {
//                 render_sink
//                     .send(RenderChannelItem::Liquid((
//                         page_file,
//                         node.render_rules.clone(),
//                     )))
//                     .unwrap();
//             }
//             PageType::Html => {
//                 render_sink
//                     .send(RenderChannelItem::Html((
//                         page_file,
//                         node.render_rules.clone(),
//                     )))
//                     .unwrap();
//             }
//         }
//     }
//
//     process_markdowns(&node, markdown_queue, render_sink);
// }

// fn process_markdowns(
//     node: &SiteNode,
//     mut markdown_queue: Vec<PageFile>,
//     render_sink: Sender<RenderChannelItem>,
// ) {
//     if markdown_queue.is_empty() {
//         return;
//     }
//
//     let mut markdown_queue: Vec<(FrontMatter, PageFile, usize)> = markdown_queue
//         .drain(0..)
//         .map(|page_file| {
//             let contents = diskio::read_file_contents(&page_file.abs_path);
//             // TODO if anything, probably want buffered reading, and not Mmap -> `[..]` slice.
//             let (frontmatter, offset) = parse_frontmatter(&contents[..]).unwrap();
//             (frontmatter, page_file, offset)
//         })
//         .collect();
//
//     // TODO(low priority): support other order bys
//     markdown_queue.sort_by_key(|item| -item.0.timestamp.timestamp());
//
//     let page_size = if let Some(listing_rule_set) = node.render_rules.listing.as_ref() {
//         listing_rule_set.page_size.unwrap_or(10)
//     } else {
//         1
//     };
//
//     // TODO don't really need this if not paginating...
//     let md_count = markdown_queue.len();
//
//     // Round down OK because page numbers are 0-indexed.
//     // TODO check the math on 1 full page.
//     // TODO ensure not md_count == 0... would be bad config but whatever
//     let page_count = (md_count - 1) / page_size;
//
//     markdown_queue
//         .chunks(page_size)
//         .enumerate()
//         .for_each(|(page_index, chunk)| {
//             let pages: Vec<Markdown> = chunk
//                 .iter()
//                 .map(|(frontmatter, page_file, offset)| {
//                     // TODO try to rework to avoid the `.clone()`s here.
//                     let contents = diskio::read_file_contents(&page_file.abs_path);
//                     Markdown {
//                         frontmatter: frontmatter.clone(),
//                         blocks: parse_blocks(&contents[(*offset)..]),
//                     }
//                 })
//                 .collect();
//             // TODO ought to be a cleaner way to get this?
//             let render_info = MarkdownPageInfo {
//                 items: pages,
//                 dir_path: node.dir.clone(),
//                 page_index: (page_index, page_count),
//             };
//             render_sink
//                 .send(RenderChannelItem::MarkdownSet((
//                     render_info,
//                     node.render_rules.clone(),
//                 )))
//                 .unwrap();
//         });
// }

// fn delegate_rendering(renderer: &Renderer, sink: Receiver<RenderChannelItem>, out_dir: &Path) {
//     sink.into_iter().par_bridge().for_each(|item| match item {
//         RenderChannelItem::MarkdownSet((info, render_rules)) => {
//             // This one's the most complicated because it may be necessary to render a listing page.
//             let MarkdownPageInfo {
//                 items: pages,
//                 dir_path,
//                 page_index,
//             } = info;
//
//             if render_rules.should_render_list() {
//                 let rendered = renderer
//                     .render_listing_page(&pages, &render_rules, page_index)
//                     .unwrap();
//                 let path = dir_path.join(format!("{}", page_index.0));
//                 diskio::write_html(out_dir, path, &rendered);
//             }
//
//             pages.into_iter().for_each(|page| {
//                 let rendered = renderer
//                     .render_markdown(&page, &render_rules)
//                     .expect("rendering failed");
//                 // TODO probably also want to minify + compress
//                 diskio::write_html(out_dir, dir_path.join(&page.frontmatter.slug), &rendered);
//             });
//         }
//         // TODO should use these `render_rules`
//         RenderChannelItem::Liquid((page_file, _render_rules)) => {
//             let rendered = renderer
//                 .render(&page_file.abs_path)
//                 .expect("rendering failed");
//             diskio::write_html(out_dir, &page_file.rel_path, &rendered);
//         }
//         RenderChannelItem::Html((file_entry, render_rules)) => {
//             let html = fs::read_to_string(&file_entry.abs_path).unwrap();
//             let rendered = renderer
//                 .render_html(&html, &render_rules)
//                 .unwrap_or_else(|_| panic!("rendering failed: {:?}", &file_entry.abs_path));
//             // TODO only `rel_dir()` for `index.html`?
//             diskio::write_html(out_dir, file_entry.rel_dir(), &rendered);
//         }
//     });
// }

fn get_latest_modified(paths: &[PathBuf]) -> u64 {
    paths
        .iter()
        .map(|p| p.metadata().unwrap().modified().unwrap())
        .max()
        .unwrap() // Assume there must be _some_ templates
        // TODO could probably assert that earlier though
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn check_latest_modified_liquid(conn: &rusqlite::Connection, paths: &[PathBuf]) -> bool {
    let files_ts = get_latest_modified(paths);
    if let Some(cache_ts) = cache::get_latest_template_modified(conn).unwrap() {
        // TODO really have to roll this back if we blow up later in the generation...
        tracing::debug!("files_ts: {}, cache_ts: {}", files_ts, cache_ts);
        if cache_ts < files_ts {
            cache::set_latest_template_modified(conn, files_ts).unwrap();
            true
        } else {
            false
        }
    } else {
        cache::set_latest_template_modified(conn, files_ts).unwrap();
        true
    }
}

fn get_all_liquids(config: &Config) -> Vec<PathBuf> {
    diskio::walk(&config.layouts_dir(), "liquid")
        .chain(diskio::walk(&config.blocks_dir(), "liquid"))
        .collect()
}

fn parse_page(page_file: PageFile) -> anyhow::Result<Page> {
    match page_file.get_page_type() {
        PageType::Markdown => {
            let contents = page_file.get_contents()?;
            let markdown = markdown::parse(contents.as_bytes())?;
            Ok(Page::Markdown(MarkdownPage {
                file: page_file,
                markdown,
            }))
        }
        PageType::Liquid => Ok(Page::Liquid(page_file)),
        PageType::Html => Ok(Page::Html(page_file)),
    }
}

fn copy_previously_generated<C: Deref<Target = Config>, P: AsRef<Path>>(
    config: &C,
    page_file: &PageFile,
    staging_dir: P,
) -> anyhow::Result<()> {
    let previous_path = config.out_dir().join(&page_file.out_path);
    if previous_path.metadata()?.is_file() {
        let current_path = staging_dir.as_ref().join(&page_file.out_path);
        fs::create_dir_all(current_path.parent().unwrap())?;
        fs::copy(&previous_path, current_path)?;
        Ok(())
    } else {
        Err(anyhow!(
            "no previously generated file found for {:?}",
            page_file.abs_path
        ))
    }
}

#[instrument(skip(config, pool, renderer, staging_dir, templates_were_modified))]
fn handle_site_node<C: Deref<Target = Config>, P: AsRef<Path>>(
    config: &C,
    node: &SiteNode,
    pool: &cache::Pool,
    renderer: &Renderer,
    staging_dir: P,
    templates_were_modified: bool,
) {
    let conn = pool.get().unwrap();
    // TODO have to do a chunked iteration when we're rendering listing
    let pages: Vec<Page> = node
        .page_files
        .iter()
        .map(|page_file| {
            // TODO currently this hardcodes in the cache-busted CSS ... would be nice not to do
            // that. so we don't have to rerender the HTMLs for CSS changes.
            tracing::debug!("processing page");
            if !templates_were_modified {
                if let Some((page, _rendered)) = cache::restore_cached(&conn, page_file.clone())? {
                    // TODO try copying, but if that fails, then write the `rendered`
                    match copy_previously_generated(config, page_file, &staging_dir) {
                        Ok(_) => {
                            tracing::debug!(
                                "copied previously generated file for {:?}",
                                &page_file.out_path
                            );
                            return Ok(page);
                        }
                        Err(e) => {
                            tracing::warn!("error copying previously generated file: {:?}", e);
                        }
                    }
                }
            }

            tracing::debug!("rendering page: {:?}", page_file.rel_path);
            let page = parse_page(page_file.clone()).unwrap();
            let rendered = renderer.render_page(&page, &node.render_rules)?;
            // TODO really maybe should do the rest of this stuff with tokio async,
            // so just send whatever is needed through a channel to handle that.
            cache::cache(&conn, &page, &rendered).unwrap();
            diskio::write_html(staging_dir.as_ref().join(&page_file.out_path), &rendered)?;
            Ok(page)
        })
        .collect::<anyhow::Result<Vec<Page>>>()
        .unwrap();

    // TODO listing pages
    // if node.render_rules.should_render_listing() {
    //     let markdowns: Vec<Markdown> = pages.iter().filter_map(|page| match page {
    //         Page::Markdown(m) => Some(m),
    //         _ => None,
    //     });
    //     let rendered = renderer
    //         .render_listing_page(&pages, &node.render_rules, 0)
    //         .unwrap();
    //     let path = dir_path.join(format!("{}", page_index.0));
    //     diskio::write_html(out_dir, path, &rendered);
    // }
}

/// Generate the site.
pub fn generate() -> anyhow::Result<()> {
    let pool = cache::new_pool();
    let conn = pool.get()?;
    // TODO probably should be one big tx so idk about the pool...
    cache::init_cache(&conn).unwrap();
    let config = Arc::new(Config::init().map_or_else(|e| panic!("{}", e), |c| c));

    let config_clone = config.clone();
    let site_nodes = diskio::collect_site_nodes(config_clone);

    let liquids = get_all_liquids(&config);
    let templates_were_modified = {
        let conn = pool.get().unwrap();
        check_latest_modified_liquid(&conn, &liquids)
    };

    let staging_dir = TempDir::new("stalagmite_staging").unwrap();

    // TODO incremental css parsing, queued after template rendering.
    let tailwind_filename = assets::generate_css(&liquids, true, &staging_dir).unwrap();
    let renderer = Renderer::new(&config, tailwind_filename, &liquids);

    tracing::debug!("collected {} site nodes", site_nodes.len());

    site_nodes.iter().par_bridge().for_each(|site_node| {
        handle_site_node(
            &config,
            site_node,
            &pool,
            &renderer,
            &staging_dir,
            templates_were_modified,
        )
    });
    // Replace the old output directory with the new one.
    std::fs::remove_dir_all(config.out_dir()).unwrap();
    std::fs::rename(staging_dir, config.out_dir()).unwrap();

    tracing::info!("static site generated!");
    Ok(())
}
