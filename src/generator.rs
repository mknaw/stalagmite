use std::ops::Deref;
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use std::{fs, thread};

use anyhow::anyhow;
use rayon::prelude::*;
use tempdir::TempDir;
use tokio::task::JoinSet;

use crate::common::*;
use crate::parsers::markdown;
use crate::{assets, cache, diskio, Config, Renderer};

fn get_latest_modified(site_entries: &[SiteEntry]) -> u64 {
    site_entries
        .iter()
        .map(|p| p.abs_path.metadata().unwrap().modified().unwrap())
        .max()
        .unwrap() // Assume there must be _some_ templates
        // TODO could probably assert that earlier though
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn check_latest_modified_liquid(conn: &rusqlite::Connection, liquids: &[SiteEntry]) -> bool {
    let files_ts = get_latest_modified(liquids);
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

// TODO these don't actually need to be SiteEntrys... I just wanted to reuse `get_contents`.
// Really could have a `StaticAsset` type.
fn collect_liquids(config: &Config) -> Vec<SiteEntry> {
    let layouts = diskio::walk(&config.layouts_dir(), "liquid")
        .map(|path| SiteEntry::try_new(&config.layouts_dir(), path).unwrap());
    let blocks = diskio::walk(&config.blocks_dir(), "liquid")
        .map(|path| SiteEntry::try_new(&config.blocks_dir(), path).unwrap());
    layouts.chain(blocks).collect()
}

fn parse_page(site_entry: SiteEntry) -> anyhow::Result<Page> {
    match site_entry.get_page_type() {
        PageType::Markdown => {
            let contents = site_entry.get_contents()?;
            let markdown = markdown::parse(contents.as_bytes())?;
            Ok(Page::new_markdown_page(site_entry, markdown))
        }
        PageType::Liquid => Ok(Page::new_liquid_page(site_entry)),
        PageType::Html => Ok(Page::new_html_page(site_entry)),
    }
}

fn copy_previously_generated<C: Deref<Target = Config>, P: AsRef<Path>>(
    config: &C,
    site_entry: &SiteEntry,
    staging_dir: P,
) -> anyhow::Result<()> {
    // TODO even better than copying from the previous output to the staging would be to mark as
    // "keep" and then merge the staging with the previous dir. I will probably never do that tho.
    let previous_path = config.out_dir().join(&site_entry.out_path);
    if previous_path.metadata()?.is_file() {
        let current_path = staging_dir.as_ref().join(&site_entry.out_path);
        fs::create_dir_all(current_path.parent().unwrap())?;
        fs::copy(&previous_path, current_path)?;
        Ok(())
    } else {
        Err(anyhow!(
            "no previously generated file found for {:?}",
            site_entry.rel_path
        ))
    }
}

fn generate_listing<P: AsRef<Path>, R: Deref<Target = RenderRules>>(
    conn: &rusqlite::Connection,
    renderer: &Renderer,
    render_rules: &R,
    staging_dir: P,
    group_path: &str,
) -> anyhow::Result<()> {
    // TODO need to get the right pagination count.
    // TODO also should be able to restore cached renders from the db!
    let page_info_iterator = cache::get_markdown_info_listing_iterator(conn, group_path, 100);
    for (index, group) in page_info_iterator.enumerate() {
        match group {
            Ok(markdowns) => {
                // TODO need to get the page count (sqlite also).
                let rendered =
                    renderer.render_listing_page(&markdowns, render_rules, (index, 100))?;
                let out_path = staging_dir
                    .as_ref()
                    .join(group_path)
                    .join(format!("{}/index.html", index));
                fs::create_dir_all(out_path.parent().unwrap())?;
                diskio::write_html_sync(out_path, &rendered)?;
            }
            Err(e) => {
                return Err(anyhow!(e));
            }
        }
    }
    Ok(())
}

/// Generate the site.
pub async fn generate(config: Arc<Config>) -> anyhow::Result<()> {
    let pool = Arc::new(cache::new_pool());
    let conn = pool.get()?;
    // TODO probably should be one big tx so idk about the pool...
    // Or maybe copy the whole DB?
    cache::init_cache(&conn).unwrap();

    let site_nodes = diskio::collect_site_nodes(config.clone());

    let staging_dir = TempDir::new("stalagmite_staging").unwrap();

    let liquids = collect_liquids(&config);

    {
        let mut class_collector = assets::ClassCollector::new();
        liquids.iter().for_each(|pf| {
            assets::collect_classes(pf.get_contents().unwrap(), &mut class_collector)
        });

        site_nodes
            .iter()
            .flat_map(|node| {
                node.site_entries
                    .iter()
                    .filter(|pf| matches!(pf.get_page_type(), PageType::Html))
            })
            .for_each(|pf| {
                assets::collect_classes(pf.get_contents().unwrap(), &mut class_collector)
            });

        assets::render_css(class_collector, true, &staging_dir).unwrap();
    }

    let renderer = Arc::new(Renderer::new(&config, "tw.css".to_string(), &liquids));

    tracing::debug!("collected {} site nodes", site_nodes.len());

    let (render_tx, render_rx) = channel::<(SiteEntry, Arc<RenderRules>)>();
    let (post_render_tx, post_render_rx) = channel::<(Page, String)>();

    let (render_listing_tx, render_listing_rx) = channel::<(String, Arc<RenderRules>)>();

    let staging_dir = Arc::new(staging_dir.path().to_path_buf());
    {
        let config = config.clone();
        let post_render_tx = post_render_tx.clone();
        let pool = pool.clone();
        let staging_dir = staging_dir.clone();

        thread::spawn(move || {
            let conn = &pool.get().unwrap();
            let liquids_were_modified = check_latest_modified_liquid(conn, &liquids);
            site_nodes
                .into_iter()
                .try_for_each(|node| -> Result<(), anyhow::Error> {
                    let result = node.site_entries.iter().try_for_each(
                        |site_entry| -> Result<(), anyhow::Error> {
                            // If `liquids_were_modified`, we know we have to rerender anyway.
                            if !liquids_were_modified {
                                if let Some((page, rendered)) =
                                    cache::restore_cached(conn, site_entry.clone())?
                                {
                                    // TODO probably should use tokio `copy` and do this with some
                                    // concurrency?
                                    match copy_previously_generated(
                                        &config,
                                        site_entry,
                                        staging_dir.as_path(),
                                    ) {
                                        Ok(_) => {
                                            tracing::debug!(
                                                "copied previously generated file for {:?}",
                                                &site_entry.out_path
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
                                .send((site_entry.clone(), node.render_rules.clone()))
                                .map_err(|e| anyhow!(e))
                        },
                    );
                    if result.is_ok() && node.render_rules.should_render_listing() {
                        render_listing_tx
                            .send((
                                node.dir.as_os_str().to_str().unwrap().to_string(),
                                node.render_rules.clone(),
                            ))
                            .unwrap();
                    }
                    result
                })
                .unwrap();
        });
    }

    {
        let renderer = renderer.clone();
        thread::spawn(move || {
            render_rx
                .into_iter()
                .par_bridge()
                .try_for_each(|(site_entry, render_rules)| -> Result<(), anyhow::Error> {
                    tracing::debug!("rendering page: {:?}", site_entry.rel_path);
                    let page = parse_page(site_entry.clone()).unwrap();
                    let rendered = renderer.render_page(&page, &render_rules)?;
                    post_render_tx.send((page, rendered)).unwrap();
                    Ok(())
                })
                .unwrap();
        });
    }

    // TODO in fact I think there can be other race conditions, for the other thread.
    // so maybe need something better than this.
    let mut join_set = JoinSet::new();

    for (page, rendered) in post_render_rx.iter() {
        let staging_path = staging_dir.clone();
        let pool = pool.clone();
        // TODO probably not even worth doing async... probably better to just thread it...
        join_set.spawn(async move {
            tracing::debug!("writing rendered page to disk");
            // TODO really should use async rusqlite for this...
            let conn = &pool.get().unwrap();
            cache::cache(conn, &page, &rendered).unwrap();
            diskio::write_html(staging_path.join(&page.file.out_path), &rendered).await?;
            Ok::<(), anyhow::Error>(())
        });
    }

    while (join_set.join_next().await).is_some() {}

    for (dir, render_rules) in render_listing_rx.iter() {
        generate_listing(&conn, &renderer, &render_rules, staging_dir.as_path(), &dir).unwrap();
    }

    // Replace the old output directory with the new one.
    std::fs::remove_dir_all(config.out_dir()).unwrap();
    std::fs::rename(staging_dir.as_path(), config.out_dir()).unwrap();
    tracing::info!("static site generated!");

    Ok(())
}
