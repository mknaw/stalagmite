use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use ignore::Walk;
use memmap2::Mmap;

use crate::core::{FileType, PageFile, RenderRuleSet, SiteNode, DEFAULT_RENDER_RULE_SET};
use crate::Config;

/// Read a `path` to an `Mmap`.
pub fn read_file_contents<P: AsRef<Path>>(path: P) -> Mmap {
    let file = fs::File::open(path).unwrap();
    unsafe { Mmap::map(&file).unwrap() }
}

/// Recursively iterate over all files in the given directory with the given extension.
pub fn walk<'a, P: AsRef<Path>>(dir: P, ext: &'a str) -> Box<dyn Iterator<Item = PathBuf> + 'a> {
    let walk = Walk::new(dir).flatten().filter_map(|entry| {
        let path = entry.path();
        if path.is_file() && path.extension() == Some(ext.as_ref()) {
            Some(path.to_owned())
        } else {
            None
        }
    });
    Box::new(walk)
}

/// Recursively iterate over pages directory, sending data pertaining to each directory to a `sink`.
pub fn collect_generation_nodes(config: Arc<Config>, sink: Sender<SiteNode>) {
    let mut rules_stack = vec![DEFAULT_RENDER_RULE_SET.clone()];
    let pages_dir = config.pages_dir();
    recurse(&pages_dir, &pages_dir, &mut rules_stack, &sink);

    fn recurse(
        pages_dir: &Path,
        current_path: &Path,
        rules_stack: &mut Vec<Arc<RenderRuleSet>>,
        tx: &Sender<SiteNode>,
    ) {
        let rules_path = current_path.join("rules.yaml");
        if rules_path.exists() && rules_path.is_file() {
            let raw = fs::read_to_string(&rules_path).unwrap();
            // TODO wanted to override the previous rules on the stack...
            // TODO might not need Arc anymore, now that SiteNodes own the RuleSets?
            let rule_set: Arc<RenderRuleSet> = Arc::new(serde_yaml::from_str(&raw).unwrap());
            rules_stack.push(rule_set);
        }

        if let Ok(dir_entries) = fs::read_dir(current_path) {
            let paths: Vec<PathBuf> = dir_entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .collect();
            let page_files: Vec<PageFile> = paths
                .iter()
                .filter_map(|path| match path.extension().and_then(|ext| ext.to_str()) {
                    Some("md") => Some((path.to_owned(), FileType::Markdown)),
                    Some("liquid") => Some((path.to_owned(), FileType::Liquid)),
                    Some("html") => Some((path.to_owned(), FileType::Html)),
                    _ => None,
                })
                .map(|(abs_path, ftype)| {
                    let rel_path = abs_path.strip_prefix(pages_dir).unwrap().to_owned();
                    PageFile {
                        abs_path,
                        rel_path,
                        ftype,
                    }
                })
                .collect();

            if !page_files.is_empty() {
                tx.send(SiteNode {
                    dir: current_path.strip_prefix(pages_dir).unwrap().to_path_buf(),
                    rules: rules_stack.last().expect("rules_stack is empty").clone(),
                    entries: page_files,
                })
                .unwrap();
            }

            paths.iter().filter(|path| path.is_dir()).for_each(|path| {
                recurse(pages_dir, path, rules_stack, tx);
            });
        }

        // Pop off the stack when backtracking
        if rules_path.exists() && rules_path.is_file() {
            rules_stack.pop();
        }
    }
}

// TODO this P, P2 thing is unseemly.
// TODO should callers just take care of joining the path?
// TODO probably also want to minify + compress
pub fn write_html<P: AsRef<Path>, P2: AsRef<Path>>(
    out_dir_path: P,
    rel_path: P2,
    html: &str,
) -> PathBuf {
    let mut path = out_dir_path.as_ref().join(rel_path.as_ref());
    // let mut path = PathBuf::from("./public/").join(path);
    fs::create_dir_all(&path).unwrap();
    path.push("index.html");
    fs::write(&path, html).unwrap();
    path
}
