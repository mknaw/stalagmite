use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::*;

use crate::common::*;

const DB_PATH: &str = "./db.sqlite";

type ConnectionManager = SqliteConnectionManager;
pub type Pool = r2d2::Pool<ConnectionManager>;

pub fn new_pool() -> Pool {
    let manager = SqliteConnectionManager::file(DB_PATH);
    r2d2::Pool::new(manager).unwrap()
}

// TODO use migrations
pub fn init_cache(conn: &Connection) -> anyhow::Result<()> {
    // TODO don't actually _need_ parent_url, can define a custom function to get it from url.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS markdowns (
            id          INTEGER PRIMARY KEY,
            url         TEXT NOT NULL UNIQUE,
            parent_url  TEXT NOT NULL,
            hash        TEXT NOT NULL,
            timestamp   INTEGER NOT NULL,
            frontmatter BLOB NOT NULL,
            blocks      TEXT NOT NULL,
            rendered    BLOB NOT NULL
        )",
        // content ? for text search.
        (),
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pages (
            id       INTEGER PRIMARY KEY,
            url      TEXT NOT NULL UNIQUE,
            hash     TEXT NOT NULL,
            rendered BLOB NOT NULL
        )",
        // content ? for text search.
        (),
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS partial_checkpoint (
            id      INTEGER PRIMARY KEY,
            touched INTEGER NOT NULL
        )",
        (),
    )?;
    Ok(())
}

pub fn get_latest_template_modified(conn: &Connection) -> Result<Option<u64>> {
    conn.query_row(
        "SELECT touched FROM partial_checkpoint LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
}

pub fn set_latest_template_modified(conn: &Connection, time: u64) -> Result<()> {
    conn.execute("DELETE FROM partial_checkpoint", [])?;
    conn.execute(
        "INSERT INTO partial_checkpoint (touched) VALUES (?)",
        [time],
    )?;
    Ok(())
}

pub fn restore_cached(
    conn: &Connection,
    site_entry: SiteEntry,
) -> anyhow::Result<Option<(Page, String)>> {
    let query_result = match site_entry.get_page_type() {
        PageType::Markdown => restore_cached_markdown(conn, &site_entry)?
            .map(|(md, rendered)| (PageData::Markdown(md), rendered)),
        PageType::Liquid => {
            restore_cached_page(conn, &site_entry)?.map(|rendered| (PageData::Liquid, rendered))
        }
        PageType::Html => {
            restore_cached_page(conn, &site_entry)?.map(|rendered| (PageData::Html, rendered))
        }
    };
    Ok(query_result.map(|(data, rendered)| {
        (
            Page {
                file: site_entry,
                data,
            },
            rendered,
        )
    }))
}

/// Perform a query to fetch cached data for `Markdown` construction.
fn restore_cached_markdown(
    conn: &Connection,
    site_entry: &SiteEntry,
) -> Result<Option<(Markdown, String)>> {
    conn.query_row(
        "SELECT frontmatter, blocks, rendered FROM markdowns WHERE url=:url AND hash=:hash",
        named_params! {
            ":url": site_entry.url_path,
            ":hash": site_entry.get_hash().unwrap(),
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
}

fn restore_cached_page(conn: &Connection, site_entry: &SiteEntry) -> Result<Option<String>> {
    conn.query_row(
        "SELECT rendered FROM pages WHERE url=:url AND hash=:hash",
        named_params! {
            ":url": site_entry.url_path,
            ":hash": site_entry.get_hash().unwrap(),
        },
        |row| {
            let rendered: String = row.get(0)?;
            Ok(rendered)
        },
    )
    .optional()
}

pub fn cache(conn: &Connection, page: &Page, rendered: &str) -> anyhow::Result<()> {
    match &page.data {
        PageData::Markdown(md) => cache_markdown(conn, &page.file, md, rendered),
        PageData::Liquid => cache_page(conn, &page.file, rendered),
        PageData::Html => cache_page(conn, &page.file, rendered),
    }
}

fn cache_markdown(
    conn: &Connection,
    site_entry: &SiteEntry,
    markdown: &Markdown,
    rendered: &str,
) -> anyhow::Result<()> {
    let frontmatter = serde_yaml::to_string(&markdown.frontmatter).unwrap();
    let blocks = serde_yaml::to_string(&markdown.blocks).unwrap();
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
            ":hash": site_entry.get_hash().unwrap(),
            ":timestamp": markdown.frontmatter.timestamp.timestamp(),
            ":frontmatter": &frontmatter,
            ":blocks": &blocks,
            ":rendered": rendered,
        },
    )?;
    Ok(())
}

fn cache_page(conn: &Connection, site_entry: &SiteEntry, rendered: &str) -> anyhow::Result<()> {
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
            ":hash": site_entry.get_hash().unwrap(),
            ":rendered": rendered,
        },
    )?;
    Ok(())
}

// Iterator that encapsulates SQLite query execution with offset and limit.
pub struct MarkdownIterator<'a> {
    limit: u32,
    statement: Statement<'a>,
    parent_url: &'a str,
    offset: u32,
}

impl<'a> MarkdownIterator<'a> {
    fn new(
        conn: &'a Connection,
        parent_url: &'a str,
        query: &'a str,
        limit: u32,
    ) -> MarkdownIterator<'a> {
        MarkdownIterator {
            parent_url,
            limit,
            statement: conn.prepare(query).unwrap(),
            offset: 0,
        }
    }
}

impl<'a> Iterator for MarkdownIterator<'a> {
    type Item = Result<Vec<Markdown>>;

    fn next(&mut self) -> Option<Self::Item> {
        let rows =
            self.statement
                .query_map(params![self.parent_url, self.limit, self.offset], |row| {
                    let frontmatter: String = row.get(0)?;
                    let blocks: String = row.get(1)?;
                    let markdown = Markdown {
                        frontmatter: serde_yaml::from_str(&frontmatter).unwrap(),
                        blocks: serde_yaml::from_str(&blocks).unwrap(),
                    };
                    Ok(markdown)
                });

        self.offset += self.limit;

        match rows {
            Ok(rows) => {
                let markdowns = rows.collect::<Result<Vec<Markdown>>>();
                if let Ok(mds) = &markdowns {
                    if mds.is_empty() {
                        return None;
                    }
                }
                Some(markdowns)
            }
            Err(e) => Some(Err(e)),
        }
    }
}

pub fn get_markdown_info_listing_iterator<'a>(
    conn: &'a Connection,
    parent_url: &'a str,
    limit: u32,
) -> MarkdownIterator<'a> {
    MarkdownIterator::new(
        conn,
        parent_url,
        "SELECT frontmatter, blocks
        FROM markdowns
        WHERE parent_url = ?
        ORDER BY timestamp
        LIMIT ?
        OFFSET ?",
        limit,
    )
}
