use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ignore::Walk;

use crate::markdown::parse_markdown_file;
use crate::pages::{RenderRuleSet, DEFAULT_RENDER_RULE_SET};
use crate::{Config, Page};

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

// TODO there are probably ways to parallelize this proccess...
pub fn collect_pages(config: &Config) -> Vec<Page> {
    fn walk<P: AsRef<Path>>(
        config: &Config,
        path: P,
        pages: &mut Vec<Page>,
        rules_stack: &mut Vec<Arc<RenderRuleSet>>,
    ) {
        // Check for 'rules.yaml' in the current directory
        let rules_path = path.as_ref().join("rules.yaml");
        if rules_path.exists() && rules_path.is_file() {
            let raw = fs::read_to_string(&rules_path).unwrap();
            // TODO should probably overwrite only specified fields on the last one.
            let rule_set: Arc<RenderRuleSet> = Arc::new(serde_yaml::from_str(&raw).unwrap());
            rules_stack.push(rule_set);
        }

        // Process subdirectories
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    walk(config, &path, pages, rules_stack);
                } else if path.extension() == Some("md".as_ref()) {
                    let md_path = path.strip_prefix(&config.current_dir).unwrap();
                    let (frontmatter, blocks) = parse_markdown_file(md_path).unwrap();
                    let rule_set = rules_stack.last().expect("rules_stack is empty");
                    pages.push(Page {
                        path: md_path
                            .parent()
                            .unwrap()
                            .strip_prefix("pages/")
                            .unwrap()
                            .to_path_buf(),
                        frontmatter,
                        blocks,
                        render_rules: rule_set.clone(),
                    });
                }
            }
        }

        // Pop off the stack when backtracking
        if rules_path.exists() && rules_path.is_file() {
            rules_stack.pop();
        }
    }

    let mut pages = vec![];
    let mut rules_stack = vec![DEFAULT_RENDER_RULE_SET.clone()];
    walk(config, &config.current_dir, &mut pages, &mut rules_stack);
    pages
}

pub fn write_html(path: &Path, slug: &str, html: &str) -> PathBuf {
    let mut path = PathBuf::from("./public/").join(path).join(slug);
    fs::create_dir_all(&path).unwrap();
    path.push("index.html");
    fs::write(&path, html).unwrap();
    path
}
