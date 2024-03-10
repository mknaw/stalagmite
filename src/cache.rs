use std::sync::Arc;

use futures::stream::unfold;
use futures::Stream;
use tokio_rusqlite::*;

use crate::common::*;
use crate::utils;

const DB_PATH: &str = "./db.sqlite";

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("./migrations");
}

// TODO _could_ impl Deadpool for tokio_rusqlite, but really.. it's probably overkill...
pub async fn new_connection() -> tokio_rusqlite::Result<Connection> {
    Connection::open(DB_PATH).await
}

pub async fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.call(|conn| {
        embedded::migrations::runner().run(&mut *conn).unwrap();
        Ok(())
    })
    .await?;
    Ok(())
}

// TODO also would be good to purge the old ones.
pub async fn check_asset_changed(
    conn: &Connection,
    filename: &str,
    hash: &str,
) -> anyhow::Result<bool> {
    let filename = filename.to_string();
    let hash = hash.to_string();
    conn.call(move |conn| {
        let previous: Option<String> = conn
            .query_row(
                "SELECT hash FROM assets WHERE filename = (?)",
                [&filename],
                |row| row.get(0),
            )
            .optional()?;
        let changed = previous.map_or(true, |previous| previous != hash);
        if changed {
            conn.execute(
                "INSERT INTO assets (filename, hash) VALUES (?, ?)
                     ON CONFLICT(filename) DO
                         UPDATE
                         SET
                            hash = excluded.hash
                    ",
                [&filename, &hash],
            )?;
        }
        Ok(changed)
    })
    .await
    .map_err(Into::into)
}

pub async fn get_latest_template_modified(conn: &Connection) -> Result<Option<u64>> {
    conn.call(|conn| {
        conn.query_row(
            "SELECT touched FROM partial_checkpoint LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    })
    .await
}

pub async fn set_latest_template_modified(conn: &Connection, time: u64) -> Result<()> {
    conn.call(move |conn| {
        conn.execute("DELETE FROM partial_checkpoint", [])?;
        conn.execute(
            "INSERT INTO partial_checkpoint (touched) VALUES (?)",
            [time],
        )?;
        Ok(())
    })
    .await
}

pub async fn restore_cached(
    conn: &Connection,
    site_entry: &SiteEntry,
) -> anyhow::Result<Option<(PageData, String)>> {
    let query_result = match site_entry.get_page_type() {
        PageType::Markdown => restore_cached_markdown(conn, site_entry)
            .await?
            .map(|(md, rendered)| (PageData::Markdown(md), rendered)),
        PageType::Liquid => restore_cached_page(conn, site_entry)
            .await?
            .map(|rendered| (PageData::Liquid, rendered)),
        PageType::Html => restore_cached_page(conn, site_entry)
            .await?
            .map(|rendered| (PageData::Html, rendered)),
    };
    Ok(query_result)
}

/// Perform a query to fetch cached data for `Markdown` construction.
async fn restore_cached_markdown(
    conn: &Connection,
    site_entry: &SiteEntry,
) -> Result<Option<(Markdown, String)>> {
    let url_path = site_entry.url_path.clone();
    let hash = utils::stringify_hash(site_entry.file.hash);
    conn.call(move |conn| {
        conn.query_row(
            "SELECT frontmatter, blocks, rendered FROM markdowns WHERE url=:url AND hash=:hash",
            named_params! {
                ":url": url_path,
                ":hash": hash,
            },
            |row| {
                let frontmatter: String = row.get(0)?;
                let blocks: String = row.get(1)?;
                let markdown = Markdown {
                    frontmatter: serde_yaml::from_str(&frontmatter).unwrap(),
                    blocks: serde_yaml::from_str(&blocks).unwrap(),
                };
                let rendered: String = row.get(2)?;
                Ok((markdown, rendered))
            },
        )
        .optional()
        .map_err(Into::into)
    })
    .await
}

async fn restore_cached_page(conn: &Connection, site_entry: &SiteEntry) -> Result<Option<String>> {
    let url_path = site_entry.url_path.clone();
    let hash = utils::stringify_hash(site_entry.file.hash);
    conn.call(move |conn| {
        conn.query_row(
            "SELECT rendered FROM pages WHERE url=:url AND hash=:hash",
            named_params! {
                ":url": url_path,
                ":hash": hash,
            },
            |row| {
                let rendered: String = row.get(0)?;
                Ok(rendered)
            },
        )
        .optional()
        .map_err(Into::into)
    })
    .await
}

pub async fn cache(
    conn: &Connection,
    page_data: PageData,
    site_entry: Arc<SiteEntry>,
    rendered: Arc<String>,
) -> anyhow::Result<()> {
    match page_data {
        PageData::Markdown(md) => cache_markdown(conn, site_entry, &md, rendered).await,
        PageData::Liquid => cache_page(conn, site_entry, rendered).await,
        PageData::Html => cache_page(conn, site_entry, rendered).await,
        PageData::Listing(..) => unimplemented!(),
    }
}

async fn cache_markdown(
    conn: &Connection,
    site_entry: Arc<SiteEntry>,
    markdown: &Markdown,
    rendered: Arc<String>,
) -> anyhow::Result<()> {
    let frontmatter = serde_yaml::to_string(&markdown.frontmatter).unwrap();
    let blocks = serde_yaml::to_string(&markdown.blocks).unwrap();
    let timestamp = markdown.frontmatter.timestamp.timestamp();
    let hash = utils::stringify_hash(site_entry.file.hash);
    conn.call(move |conn| {
        conn.execute(
            "INSERT INTO markdowns (url, parent_url, hash, timestamp, frontmatter, blocks, rendered)
             VALUES (:url, :parent_url, :hash, :timestamp, :frontmatter, :blocks, :rendered)
             ON CONFLICT(url) DO
                 UPDATE
                 SET
                    hash = excluded.hash,
                    frontmatter = excluded.frontmatter
            ",
            named_params! {
                ":url": site_entry.url_path,
                ":parent_url": site_entry.parent_url(),
                ":hash": hash,
                ":timestamp": timestamp,
                ":frontmatter": &frontmatter,
                ":blocks": &blocks,
                ":rendered": rendered,
            },
        )?;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

async fn cache_page(
    conn: &Connection,
    site_entry: Arc<SiteEntry>,
    rendered: Arc<String>,
) -> anyhow::Result<()> {
    let hash = utils::stringify_hash(site_entry.file.hash);
    conn.call(move |conn| {
        conn.execute(
            "INSERT INTO pages (url, hash, rendered)
             VALUES (:url, :hash, :rendered)
             ON CONFLICT(url) DO
                 UPDATE
                 SET
                    hash = excluded.hash
            ",
            named_params! {
                ":url": site_entry.url_path,
                ":hash": hash,
                ":rendered": rendered,
            },
        )
        .map_err(Into::into)
    })
    .await?;
    Ok(())
}

pub async fn get_page_group_count(conn: &Connection, parent_url: &str) -> anyhow::Result<u8> {
    let parent_url = parent_url.to_string();
    conn.call(move |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM markdowns WHERE parent_url = ?",
            [parent_url],
            |row| row.get(0),
        )
        .map_err(Into::into)
    })
    .await
    .map_err(Into::into)
}

struct MarkdownIteratorState<'a> {
    conn: Connection, // Consider Arc<Mutex<Connection>> for shared access
    parent_url: &'a str,
    limit: u8,
    offset: u8,
}

async fn fetch_next_batch(
    state: MarkdownIteratorState<'_>,
) -> Option<(Vec<(Markdown, String)>, MarkdownIteratorState<'_>)> {
    let MarkdownIteratorState {
        conn,
        parent_url,
        limit,
        offset,
    } = state;

    // Execute the query in a blocking task
    let results: Vec<(Markdown, String)> = {
        let parent_url = parent_url.to_string();
        conn.call(move |conn| {
            let mut stmt = conn.prepare(
                "
                SELECT frontmatter, blocks, url
                FROM markdowns
                WHERE parent_url = ?
                ORDER BY timestamp
                LIMIT ?
                OFFSET ?
                ",
            )?;

            let results = stmt
                .query_map(params![parent_url, limit, offset], |row| {
                    let frontmatter: String = row.get(0)?;
                    let blocks: String = row.get(1)?;
                    let url: String = row.get(2)?;

                    let markdown = Markdown {
                        frontmatter: serde_yaml::from_str(&frontmatter).unwrap(),
                        blocks: serde_yaml::from_str(&blocks).unwrap(),
                    };
                    Ok((markdown, url))
                })?
                .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

            Ok(results)
        })
        .await
        .unwrap()
    };
    if results.is_empty() {
        None
    } else {
        Some((
            results,
            MarkdownIteratorState {
                conn,
                parent_url,
                limit,
                offset: offset + limit,
            },
        ))
    }
}

pub fn markdown_stream(
    conn: Connection,
    parent_url: &str,
    limit: u8,
) -> impl Stream<Item = Vec<(Markdown, String)>> + '_ {
    unfold(
        MarkdownIteratorState {
            conn,
            parent_url,
            limit,
            offset: 0,
        },
        |state| async { fetch_next_batch(state).await },
    )
}
