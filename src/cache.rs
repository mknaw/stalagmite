use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::*;

use crate::core::*;

const DB_PATH: &str = "./db.sqlite";

type ConnectionManager = SqliteConnectionManager;
pub type Pool = r2d2::Pool<ConnectionManager>;

pub fn new_pool() -> Pool {
    let manager = SqliteConnectionManager::file(DB_PATH);
    r2d2::Pool::new(manager).unwrap()
}

// TODO use migrations
pub fn init_cache(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS markdowns (
            id          INTEGER PRIMARY KEY,
            path        TEXT NOT NULL UNIQUE,
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
            path     TEXT NOT NULL UNIQUE,
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
    page_file: PageFile,
) -> anyhow::Result<Option<(Page, String)>> {
    let query_result = match page_file.get_page_type() {
        PageType::Markdown => restore_cached_markdown(conn, &page_file)?.map(|(md, rendered)| {
            (
                Page::Markdown(MarkdownPage {
                    file: page_file,
                    markdown: md,
                }),
                rendered,
            )
        }),
        PageType::Liquid => restore_cached_page(conn, &page_file)?
            .map(|rendered| (Page::Liquid(page_file), rendered)),
        PageType::Html => {
            restore_cached_page(conn, &page_file)?.map(|rendered| (Page::Html(page_file), rendered))
        }
    };
    Ok(query_result)
}

/// Perform a query to fetch cached data for `Markdown` construction.
fn restore_cached_markdown(
    conn: &Connection,
    page_file: &PageFile,
) -> Result<Option<(Markdown, String)>> {
    conn.query_row(
        // "SELECT frontmatter, blocks FROM markdowns WHERE path=:path AND hash=:hash",
        "SELECT frontmatter, blocks, rendered FROM markdowns WHERE path=:path AND hash=:hash",
        named_params! {
            ":path": page_file.rel_path.to_str().unwrap(),
            ":hash": page_file.get_hash().unwrap(),
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

fn restore_cached_page(conn: &Connection, page_file: &PageFile) -> Result<Option<String>> {
    conn.query_row(
        "SELECT rendered FROM pages WHERE path=:path AND hash=:hash",
        named_params! {
            ":path": page_file.rel_path.to_str().unwrap(),
            ":hash": page_file.get_hash().unwrap(),
        },
        |row| {
            let rendered: String = row.get(0)?;
            Ok(rendered)
        },
    )
    .optional()
}

pub fn cache(conn: &Connection, page: &Page, rendered: &str) -> anyhow::Result<()> {
    match page {
        Page::Markdown(markdown_page) => cache_markdown(conn, markdown_page, rendered),
        Page::Liquid(page_file) => cache_page(conn, page_file, rendered),
        Page::Html(page_file) => cache_page(conn, page_file, rendered),
    }
}

fn cache_markdown(conn: &Connection, page: &MarkdownPage, rendered: &str) -> anyhow::Result<()> {
    let frontmatter = serde_yaml::to_string(&page.markdown.frontmatter).unwrap();
    let blocks = serde_yaml::to_string(&page.markdown.blocks).unwrap();
    conn.execute(
        "INSERT INTO markdowns (path, hash, timestamp, frontmatter, blocks, rendered)
         VALUES (:path, :hash, :timestamp, :frontmatter, :blocks, :rendered)
         ON CONFLICT(path) DO
             UPDATE
             SET
                hash = excluded.hash,
                frontmatter = excluded.frontmatter
        ",
        named_params! {
            ":path": page.file.rel_path.to_str().unwrap(),
            ":hash": page.file.get_hash().unwrap(),
            ":timestamp": page.markdown.frontmatter.timestamp.timestamp(),
            ":frontmatter": &frontmatter,
            ":blocks": &blocks,
            ":rendered": rendered,
        },
    )?;
    Ok(())
}

fn cache_page(conn: &Connection, page_file: &PageFile, rendered: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pages (path, hash, rendered)
         VALUES (:path, :hash, :rendered)
         ON CONFLICT(path) DO
             UPDATE
             SET
                hash = excluded.hash
        ",
        named_params! {
            ":path": page_file.rel_path.to_str().unwrap(),
            ":hash": page_file.get_hash().unwrap(),
            ":rendered": rendered,
        },
    )?;
    Ok(())
}
