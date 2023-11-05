use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::Walk;

fn make_layout_key(path: &Path) -> String {
    // TODO should check for non `layout` duplicates rather than permitting
    match path.file_stem().unwrap().to_str().unwrap() {
        "layout" => path.to_string_lossy().to_string(),
        name => name.to_string(),
    }
}

pub fn collect_layouts(dir: &Path) -> HashMap<String, String> {
    HashMap::from_iter(Walk::new(dir).filter_map(|entry| {
        if let Ok(entry) = entry {
            if entry.path().is_file() && entry.path().extension() == Some("liquid".as_ref()) {
                let layout = fs::read_to_string(entry.path()).unwrap();
                return Some((
                    make_layout_key(entry.path().strip_prefix(dir).unwrap()),
                    layout,
                ));
            }
        }
        None
    }))
}

/// Recursively walk `dir` and return an iterator of all markdown files.
pub fn walk_markdowns(dir: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let iter = Walk::new(dir).flatten().filter_map(|entry| {
        let path = entry.path();
        if path.is_file() && path.extension() == Some("md".as_ref()) {
            Some(path.to_owned())
        } else {
            None
        }
    });
    Box::new(iter)
}

pub fn write_html(path: &Path, slug: &str, html: &str) -> PathBuf {
    let mut path = PathBuf::from("./public/").join(path).join(slug);
    dbg!(&path);
    fs::create_dir_all(&path).unwrap();
    path.push("index.html");
    fs::write(&path, html).unwrap();
    path
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;

    use super::*;

    #[test]
    fn collect_layouts_test() {
        let dir = TempDir::new("root").unwrap();
        fs::create_dir_all(dir.path().join("nest1/nest2")).unwrap();
        fs::write(dir.path().join("a.liquid"), "content a").unwrap();
        fs::write(dir.path().join("nest1/b.liquid"), "content b").unwrap();
        fs::write(dir.path().join("nest1/nest2/c.liquid"), "content c").unwrap();

        assert_eq!(
            collect_layouts(dir.path())
                .iter()
                .map(|(k, v)| { (&k[..], &v[..]) })
                .collect::<HashMap<&str, &str>>(),
            HashMap::from_iter(vec![
                ("a", "content a"),
                ("b", "content b"),
                ("c", "content c"),
            ]),
        )
    }

    // TODO maybe tempdir here
    #[test]
    fn walk_markdowns_test() {}
}
