use std::io::Write;
use std::path::PathBuf;
use std::{env, fs, io};

use chrono::prelude::*;

use crate::utils::slugify;

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

fn get_pages_dir() -> io::Result<PathBuf> {
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

pub fn initialize() -> io::Result<()> {
    // TODO initialize git repo + `.gitignore`?
    fs::create_dir("pages")?;
    let mut layout_file = fs::File::create("layout.liquid")?;
    layout_file.write_all(include_str!("../assets/layout.liquid").as_bytes())?;
    Ok(())
}

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
