use std::fs;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use anyhow::anyhow;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tempfile::{tempdir, TempDir};
use tokio_rusqlite::Connection;

use crate::assets::AssetMap;
use crate::common::*;
use crate::parsers::markdown;
use crate::utils::divide_round_up;
use crate::{assets, cache, diskio, Config, Renderer};

async fn get_latest_modified(templates: &[ContentFile]) -> Option<u64> {
    let mut futures = FuturesUnordered::new();
    for template in templates {
        futures.push(tokio::fs::metadata(&template.abs_path));
    }

    let mut max: Option<u64> = None;
    while let Some(Ok(metadata)) = futures.next().await {
        let modified = metadata
            .modified()
            .expect("Failed to get modified time")
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        max = Some(max.map_or(modified, |m| m.max(modified)));
    }

    max
}

async fn check_latest_modified_template(conn: &Connection, templates: &[ContentFile]) -> bool {
    let files_ts = get_latest_modified(templates).await.unwrap();
    if let Some(cache_ts) = cache::get_latest_template_modified(conn).await.unwrap() {
        // TODO really have to roll this back if we blow up later in the generation...
        tracing::debug!("files_ts: {}, cache_ts: {}", files_ts, cache_ts);
        if cache_ts < files_ts {
            cache::set_latest_template_modified(conn, files_ts)
                .await
                .unwrap();
            true
        } else {
            false
        }
    } else {
        cache::set_latest_template_modified(conn, files_ts)
            .await
            .unwrap();
        true
    }
}

fn collect_templates(config: &Config) -> Vec<ContentFile> {
    let layouts = diskio::walk(&config.layouts_dir(), &Some("liquid"))
        .map(|path| ContentFile::new(&config.layouts_dir(), path));
    let blocks = diskio::walk(&config.blocks_dir(), &Some("liquid"))
        .map(|path| ContentFile::new(&config.blocks_dir(), path));
    layouts.chain(blocks).collect()
}

fn parse_page_data(site_entry: &SiteEntry) -> anyhow::Result<PageData> {
    match site_entry.get_page_type() {
        PageType::Markdown => {
            let contents = site_entry.file.get_contents()?;
            let markdown = markdown::parse(contents)?;
            Ok(PageData::Markdown(markdown))
        }
        PageType::Liquid => Ok(PageData::Liquid),
        PageType::Html => Ok(PageData::Html),
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
            site_entry.file.rel_path
        ))
    }
}

type RenderChannelItem = (Arc<SiteEntry>, Arc<RenderRules>);
type PostRenderChannelItem = (Arc<SiteEntry>, PageData, Arc<String>);
type RenderListingChannelItem = (String, Arc<RenderRules>);

struct Generator {
    config: Arc<Config>,
    staging_dir: Arc<TempDir>, // TODO do an AsRef<Path> on this?
}

impl Generator {
    pub fn new(config: Arc<Config>) -> anyhow::Result<Self> {
        let staging_dir = Arc::new(tempdir()?);

        Ok(Self {
            config,
            staging_dir,
        })
    }

    async fn generate(&self) -> anyhow::Result<()> {
        let site_nodes = diskio::collect_site_nodes(self.config.clone());
        let templates = collect_templates(&self.config);

        let tailwind_alias = assets::TAILWIND_FILENAME.to_string();
        let (asset_map, assets_have_changed) = self.collect_assets(&site_nodes, &templates).await?;
        let renderer = Arc::new(Renderer::new(
            &self.config,
            asset_map,
            tailwind_alias,
            &templates,
        ));

        // TODO where will I get these numbers from... what are good numbers?
        let (render_tx, render_rx) = tokio::sync::mpsc::channel::<RenderChannelItem>(10);
        let (post_render_tx, post_render_rx) =
            tokio::sync::mpsc::channel::<PostRenderChannelItem>(10);
        let (render_listing_tx, mut render_listing_rx) =
            tokio::sync::mpsc::channel::<RenderListingChannelItem>(10);

        // Pre-render pipeline
        let force_render = {
            let conn = cache::new_connection().await?;
            self.config.no_cache
                || assets_have_changed
                || check_latest_modified_template(&conn, &templates).await
        };
        let pre_render_handle = {
            let post_render_tx = post_render_tx.clone();
            self.run_pre_render_pipeline(
                site_nodes,
                force_render,
                post_render_tx,
                render_tx,
                render_listing_tx,
            )
        };

        // Render pipeline
        let render_handle = self.run_render_pipeline(renderer.clone(), render_rx, post_render_tx);

        // Post-render pipeline
        let post_render_handle = self.run_post_render_pipeline(post_render_rx);

        tokio::join!(pre_render_handle, render_handle, post_render_handle,);

        while let Some((dir, render_rules)) = render_listing_rx.recv().await {
            self.generate_listing(&renderer, &render_rules, &dir)
                .await
                .unwrap();
        }

        // Replace the old output directory with the new one.
        std::fs::remove_dir_all(self.config.out_dir()).unwrap();
        std::fs::rename(self.staging_dir.path(), self.config.out_dir()).unwrap();
        tracing::info!("static site generated!");

        Ok(())
    }

    async fn collect_assets(
        &self,
        site_nodes: &[SiteNode],
        templates: &[ContentFile],
    ) -> anyhow::Result<(AssetMap, bool)> {
        let conn = cache::new_connection().await.unwrap();
        let (mut asset_map, assets_have_changed) =
            assets::collect(&self.config, self.staging_dir.path(), &conn)
                .await
                .unwrap();

        let tailwind_alias = assets::TAILWIND_FILENAME.to_string();
        let tailwind_cache_busted = {
            let mut class_collector = assets::ClassCollector::new();
            templates.iter().for_each(|pf| {
                assets::collect_classes(pf.get_contents().unwrap(), &mut class_collector)
            });

            site_nodes
                .iter()
                .flat_map(|node| {
                    node.site_entries
                        .iter()
                        .filter(|se| matches!(se.get_page_type(), PageType::Html))
                })
                .for_each(|se| {
                    assets::collect_classes(se.file.get_contents().unwrap(), &mut class_collector)
                });

            assets::render_css(
                &tailwind_alias,
                class_collector,
                true,
                self.staging_dir.path(),
            )?
        };
        asset_map.insert(tailwind_alias.clone(), tailwind_cache_busted);
        Ok((asset_map, assets_have_changed))
    }

    async fn run_pre_render_pipeline(
        &self,
        site_nodes: Vec<SiteNode>,
        force_render: bool,
        post_render_tx: tokio::sync::mpsc::Sender<PostRenderChannelItem>,
        render_tx: tokio::sync::mpsc::Sender<RenderChannelItem>,
        render_listing_tx: tokio::sync::mpsc::Sender<RenderListingChannelItem>,
    ) {
        let conn = cache::new_connection().await.unwrap();
        for site_node in site_nodes {
            self.route_node(
                &conn,
                site_node,
                force_render,
                &post_render_tx,
                &render_tx,
                &render_listing_tx,
            )
            .await
            .unwrap();
        }
    }

    /// See what of the `node` can be restored from the cache.
    /// Copy what can, and send what cannot for further processing in the pipeline.
    async fn route_node(
        &self,
        conn: &Connection,
        node: SiteNode,
        force_render: bool,
        post_render_tx: &tokio::sync::mpsc::Sender<PostRenderChannelItem>,
        render_tx: &tokio::sync::mpsc::Sender<RenderChannelItem>,
        render_listing_tx: &tokio::sync::mpsc::Sender<RenderListingChannelItem>,
    ) -> anyhow::Result<()> {
        for site_entry in node.site_entries {
            let site_entry = Arc::new(site_entry);
            if !force_render && let Some((page_data, rendered)) =
                // TODO this should be an async fn
                self.try_restore_from_cache(conn, &site_entry).await?
            {
                post_render_tx
                    .send((site_entry, page_data, Arc::new(rendered)))
                    .await?;
            } else {
                render_tx
                    .send((site_entry, node.render_rules.clone()))
                    .await
                    .map_err(|e| anyhow!(e))?;
            }
        }

        if node.render_rules.should_render_listing() {
            render_listing_tx
                .send((
                    node.dir.as_os_str().to_str().unwrap().to_string(),
                    node.render_rules.clone(),
                ))
                .await?;
        }
        Ok(())
    }

    async fn try_restore_from_cache(
        &self,
        conn: &Connection,
        site_entry: &SiteEntry,
    ) -> anyhow::Result<Option<(PageData, String)>> {
        if let Some((page_data, rendered)) = cache::restore_cached(conn, site_entry).await? {
            match copy_previously_generated(&self.config, site_entry, self.staging_dir.as_ref()) {
                Ok(_) => {
                    tracing::info!(
                        "copied previously generated file for {:?}",
                        site_entry.out_path
                    );

                    return Ok(Some((page_data, rendered)));
                }
                Err(e) => {
                    tracing::warn!("error copying previously generated file: {:?}", e);
                }
            }
        }
        Ok(None)
    }

    async fn run_render_pipeline(
        &self,
        renderer: Arc<Renderer>,
        mut render_rx: tokio::sync::mpsc::Receiver<RenderChannelItem>,
        post_render_tx: tokio::sync::mpsc::Sender<PostRenderChannelItem>,
    ) {
        // Must use an unbounded channel to synchronously send from the rayon threads.
        // Backpressure _should_ be handled by the `render_rx` channel.
        let (rayon_tx, mut rayon_rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            while let Some((site_entry, page_data, rendered)) = rayon_rx.recv().await {
                post_render_tx
                    .send((site_entry, page_data, rendered))
                    .await
                    .unwrap();
            }
        });

        while let Some((site_entry, render_rules)) = render_rx.recv().await {
            let rayon_tx = rayon_tx.clone();
            let renderer = renderer.clone();
            rayon::spawn(move || {
                tracing::debug!("rendering page: {:?}", site_entry.file.rel_path);
                // TODO err handle
                let page_data = parse_page_data(&site_entry).unwrap();
                let rendered = renderer
                    .render_page(&page_data, &site_entry, &render_rules)
                    .unwrap();
                rayon_tx
                    .send((site_entry, page_data, Arc::new(rendered)))
                    .unwrap_or_else(|_| unreachable!());
            });
        }
    }

    async fn run_post_render_pipeline(
        &self,
        mut post_render_rx: tokio::sync::mpsc::Receiver<PostRenderChannelItem>,
    ) {
        while let Some((site_entry, page_data, rendered)) = post_render_rx.recv().await {
            let staging_path = self.staging_dir.path().to_path_buf();
            tracing::debug!("writing rendered page to disk");
            // TODO really should use async rusqlite for this...
            let conn = cache::new_connection().await.unwrap();
            cache::cache(&conn, page_data, site_entry.clone(), rendered.clone())
                .await
                .unwrap();
            diskio::write_html(staging_path.join(&site_entry.out_path), &rendered)
                .await
                .unwrap();
        }
    }

    async fn generate_listing<R: Deref<Target = RenderRules>>(
        &self,
        renderer: &Renderer,
        render_rules: &R,
        group_path: &str,
    ) -> anyhow::Result<()> {
        let conn = cache::new_connection().await?;
        // Should be OK to unwrap here.
        let page_size = render_rules
            .listing
            .as_ref()
            .unwrap()
            .page_size
            .unwrap_or(DEFAULT_LISTING_PAGE_SIZE);
        // TODO need to get the right pagination count.
        // TODO also should be able to restore cached renders from the db!
        let page_count = divide_round_up(
            cache::get_page_group_count(&conn, group_path).await?,
            page_size,
        );
        let stream = cache::markdown_stream(conn, group_path, page_size);
        futures::pin_mut!(stream);
        let index = 0; // TODO enumerate
        while let Some(group) = stream.next().await {
            // TODO need to get the page count (sqlite also).
            let rendered = renderer.render_listing_page(
                &group,
                render_rules,
                (index.try_into().unwrap(), page_count),
            )?;
            let out_path = self
                .staging_dir
                .path()
                .join(group_path)
                .join(format!("{}/index.html", index));
            fs::create_dir_all(out_path.parent().unwrap())?;
            diskio::write_html_sync(out_path, &rendered)?;
        }
        Ok(())
    }
}

/// Generate the site.
pub async fn generate(config: Arc<Config>) -> anyhow::Result<()> {
    let generator = Generator::new(config)?;
    generator.generate().await?;
    Ok(())
}
