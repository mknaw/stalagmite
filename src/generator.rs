use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use std::{fs, thread};

use anyhow::anyhow;
use rayon::prelude::*;
use tempdir::TempDir;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::core::*;
use crate::parsers::markdown;
use crate::{assets, cache, diskio, Config, Renderer};

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
            Ok(Page::new_markdown_page(page_file, markdown))
        }
        PageType::Liquid => Ok(Page::new_liquid_page(page_file)),
        PageType::Html => Ok(Page::new_html_page(page_file)),
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

/// Generate the site.
pub async fn generate() -> anyhow::Result<()> {
    let pool = Arc::new(cache::new_pool());
    let conn = pool.get()?;
    // TODO probably should be one big tx so idk about the pool...
    // Or maybe copy the whole DB?
    cache::init_cache(&conn).unwrap();
    let config = Arc::new(Config::init().map_or_else(|e| panic!("{}", e), |c| c));

    let config_clone = config.clone();
    let site_nodes = diskio::collect_site_nodes(config_clone);

    let liquids = get_all_liquids(&config);
    let liquids_were_modified = {
        let conn = pool.get().unwrap();
        check_latest_modified_liquid(&conn, &liquids)
    };

    let staging_dir = TempDir::new("stalagmite_staging").unwrap();

    // TODO incremental css parsing, queued after template rendering.
    let renderer = Renderer::new(&config, "tw.css".to_string(), &liquids);

    tracing::debug!("collected {} site nodes", site_nodes.len());

    let (render_tx, render_rx) = channel::<(PageFile, Arc<RenderRules>)>();
    let (post_render_tx, post_render_rx) = channel::<(Page, String)>();

    let staging_path = Arc::new(staging_dir.path().to_path_buf());
    {
        let config = config.clone();
        let post_render_tx = post_render_tx.clone();
        let pool = pool.clone();
        let staging_path = staging_path.clone();

        thread::spawn(move || {
            let conn = &pool.get().unwrap();
            site_nodes
                .iter()
                .try_for_each(|node| -> Result<(), anyhow::Error> {
                    node.page_files
                        .iter()
                        .try_for_each(|page_file| -> Result<(), anyhow::Error> {
                            // If `liquids_were_modified`, we know we have to rerender anyway.
                            if !liquids_were_modified {
                                if let Some((page, rendered)) =
                                    cache::restore_cached(conn, page_file.clone())?
                                {
                                    // TODO probably should use tokio `copy` and do this with some
                                    // concurrency?
                                    match copy_previously_generated(
                                        &config,
                                        page_file,
                                        staging_path.as_path(),
                                    ) {
                                        Ok(_) => {
                                            tracing::debug!(
                                                "copied previously generated file for {:?}",
                                                &page_file.out_path
                                            );
                                            post_render_tx.send((page, rendered))?;
                                            return Ok(());
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "error copying previously generated file: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                            render_tx
                                .send((page_file.clone(), node.render_rules.clone()))
                                .map_err(|e| anyhow!(e))
                        })
                })
                .unwrap();
        });
    }

    thread::spawn(move || {
        render_rx
            .into_iter()
            .par_bridge()
            .try_for_each(|(page_file, render_rules)| -> Result<(), anyhow::Error> {
                tracing::debug!("rendering page: {:?}", page_file.rel_path);
                let page = parse_page(page_file.clone()).unwrap();
                let rendered = renderer.render_page(&page, &render_rules)?;
                post_render_tx.send((page, rendered)).unwrap();
                Ok(())
            })
            .unwrap();
    });

    // TODO in fact I think there can be other race conditions, for the other thread.
    // so maybe need something better than this.
    let mut join_set = JoinSet::new();

    let class_collector = Arc::new(Mutex::new(assets::ClassCollector::new()));

    for (page, rendered) in post_render_rx.iter() {
        let staging_path = staging_path.clone();
        let pool = pool.clone();
        let class_collector = class_collector.clone();
        // TODO probably not even worth doing async... probably better to just thread it...
        join_set.spawn(async move {
            tracing::debug!("writing rendered page to disk");
            // TODO in fact these could all have intra-task concurrency...
            {
                // TODO ends up being quite a bit of duplicate work here, since the overwhelming
                // majority of templates in a real context will repeat the same classes ...
                // Would be much better to just regex for all the tailwind terms in htmls + liquids...
                // Even the actual JS tailwind doesn't really tolerate conditional, concatenated classes
                let mut class_collector = class_collector.lock().await;
                assets::collect_classes(&rendered, &mut class_collector);
            }
            // TODO really should use async rusqlite for this...
            let conn = &pool.get().unwrap();
            cache::cache(conn, &page, &rendered).unwrap();
            diskio::write_html(staging_path.join(&page.file.out_path), &rendered).await?;
            Ok::<(), anyhow::Error>(())
        });
    }

    while (join_set.join_next().await).is_some() {}

    let class_collector = Arc::try_unwrap(class_collector).unwrap().into_inner();
    assets::render_css(class_collector, true, &staging_dir).unwrap();

    // Replace the old output directory with the new one.
    std::fs::remove_dir_all(config.out_dir()).unwrap();
    std::fs::rename(staging_dir, config.out_dir()).unwrap();
    tracing::info!("static site generated!");

    Ok(())
}
