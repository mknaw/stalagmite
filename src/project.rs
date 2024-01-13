use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

use chrono::prelude::*;
use include_dir::{include_dir, Dir};
use thiserror::Error;

use crate::utils::slugify;

static INIT_ASSETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/init");

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error(transparent)]
    InitializationError(#[from] io::Error),
}

type ProjectResult<T> = Result<T, ProjectError>;

// TODO need to de-triplicate with respect to the Frontmatter stuff
// and then the globals being constructed for use in the template.
struct PageMeta {
    title: String,
    // TODO we should make timezone variable so users see the timezone
    // their machine has, instead of the less direct UTCized time.
    timestamp: DateTime<Utc>,
}

impl PageMeta {
    fn new(title: String) -> Self {
        Self {
            title,
            timestamp: Utc::now(),
        }
    }

    fn to_frontmatter(&self) -> String {
        // Believe we can be a more parsimonious with allocations here by using
        // `push_str` more extensively, but I think currently we have bigger
        // issues around that than we will encounter here.
        [
            "---",
            &format!("title: {}", self.title),
            &format!("timestamp: {}", self.timestamp.to_rfc3339()),
            "---",
        ]
        .join("\n")
    }
}

/// Locate the `/pages/` directory in which we expect to find markdown and HTML files.
fn get_pages_dir() -> io::Result<PathBuf> {
    // TODO should we get this from Config or something?
    let dir = env::current_dir()?;
    let pages_dir = dir.join("pages");
    if pages_dir.exists() && pages_dir.is_dir() {
        Ok(pages_dir)
    } else {
        // TODO really ought to define our own `Error`s, as usual
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No stalagmite project found",
        ))
    }
}

/// Copy over the starter assets to the current directory when the CLI `init` is called.
// TODO starting to doubt this is the way, maybe should just fallback on some include_str!?
fn copy_init_assets(asset_dir: &Dir, fs_dir: &Path) -> io::Result<()> {
    for file in asset_dir.files() {
        let file_path = fs_dir.to_path_buf().join(file.path());
        let mut new_file = fs::File::create(file_path)?;
        new_file.write_all(file.contents())?;
    }
    for sub_dir in asset_dir.dirs() {
        fs::create_dir(fs_dir.join(sub_dir.path()))?;
        copy_init_assets(sub_dir, fs_dir)?;
    }
    Ok(())
}

pub fn initialize() -> ProjectResult<()> {
    // TODO initialize git repo + `.gitignore`?
    // TODO convert the "already exists" errors to something readable.
    // TODO initalize sqlite cache
    fs::create_dir("pages")?;
    copy_init_assets(&INIT_ASSETS_DIR, &env::current_dir()?)?;
    Ok(())
}

/// Create a new markdown within the specified ./pages `path`.
pub fn add_page(path: &str, title: &str) -> io::Result<()> {
    let pages_dir = get_pages_dir()?;
    // TODO there are all sorts of fucked up strings users could pass,
    // so maybe greater caution before performing this join would be necessary.
    let mut page_path = pages_dir.join(path);
    fs::create_dir_all(&page_path)?;

    // TODO the `slugify` should be the default, but should also allow
    // specifying an explicit slug, which would obviously be preferred.
    page_path = page_path.join(format!("{}.md", slugify(title)));
    if page_path.exists() {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Page {:?} already exists", page_path),
        ))
    } else {
        let page_meta = PageMeta::new(title.to_owned());
        fs::write(page_path, page_meta.to_frontmatter())?;
        Ok(())
    }
}

/// Create a new `rules.yaml` within the specified ./pages `path`.
pub fn add_rule_set(path: &str) -> io::Result<()> {
    let pages_dir = get_pages_dir()?;
    let dir = pages_dir.join(path);
    if !dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Path {:?} is not present", path),
        ));
    }
    let rules_path = dir.join("rules.yaml");
    if rules_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Rules file {:?} already exists", rules_path),
        ));
    }
    let mut rules_file = fs::File::create(rules_path)?;
    rules_file.write_all(include_str!("../assets/rules.yaml").as_bytes())?;
    Ok(())
}
