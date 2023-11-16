use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use ignore::Walk;

use crate::core::{FileEntry, FileType, GenerationNode, RenderRuleSet, DEFAULT_RENDER_RULE_SET};
use crate::Config;

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

// let contents = unsafe { Mmap::map(&file).unwrap() };
// 1. construct VFS like tree of pages with frontmatter nodes
// 2. each node could then be processed in parallel
// 3. pagination only done at node level?

// let md_path = path.strip_prefix(&config.current_dir).unwrap();

pub fn collect_generation_nodes(config: Arc<Config>, tx: Sender<GenerationNode>) {
    fn walk(
        pages_dir: &Path,
        current_path: &Path,
        rules_stack: &mut Vec<Arc<RenderRuleSet>>,
        tx: &Sender<GenerationNode>,
    ) {
        // Check for 'rules.yaml' in the current directory
        let rules_path = current_path.join("rules.yaml");
        if rules_path.exists() && rules_path.is_file() {
            let raw = fs::read_to_string(&rules_path).unwrap();
            // TODO wanted to override the previous rules on the stack...
            // TODO might not need Arc anymore, now that SiteNodes own the RuleSets?
            let rule_set: Arc<RenderRuleSet> = Arc::new(serde_yaml::from_str(&raw).unwrap());
            rules_stack.push(rule_set);
        }

        if let Ok(dir_entries) = fs::read_dir(current_path) {
            let paths = dir_entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .collect::<Vec<_>>();
            let entries = paths
                .iter()
                .filter_map(|path| match path.extension().and_then(|ext| ext.to_str()) {
                    Some("md") => Some((path.to_owned(), FileType::Markdown)),
                    Some("liquid") => Some((path.to_owned(), FileType::Liquid)),
                    Some("html") => Some((path.to_owned(), FileType::Html)),
                    _ => None,
                })
                .map(|(abs_path, ftype)| {
                    let rel_path = abs_path.strip_prefix(pages_dir).unwrap().to_owned();
                    FileEntry {
                        abs_path,
                        rel_path,
                        ftype,
                    }
                })
                .collect::<Vec<_>>();

            if !entries.is_empty() {
                let md_path = current_path.strip_prefix(pages_dir).unwrap();
                let rule_set = rules_stack.last().expect("rules_stack is empty").clone();
                tx.send(GenerationNode {
                    dir_path: md_path.to_str().unwrap().to_owned(),
                    rules: rule_set,
                    entries,
                })
                .unwrap();
            }

            paths.iter().filter(|path| path.is_dir()).for_each(|path| {
                walk(pages_dir, path, rules_stack, tx);
            });
        }

        // Pop off the stack when backtracking
        if rules_path.exists() && rules_path.is_file() {
            rules_stack.pop();
        }
    }

    let mut rules_stack = vec![DEFAULT_RENDER_RULE_SET.clone()];
    let pages_dir = config.pages_dir();
    walk(&pages_dir, &pages_dir, &mut rules_stack, &tx);
}

// TODO this P, P2 thing is unseemly.
pub fn write_html<P: AsRef<Path>, P2: AsRef<Path>>(out_dir: P, path: P2, html: &str) -> PathBuf {
    let mut path = out_dir.as_ref().join(path.as_ref());
    // let mut path = PathBuf::from("./public/").join(path);
    fs::create_dir_all(&path).unwrap();
    path.push("index.html");
    fs::write(&path, html).unwrap();
    path
}
