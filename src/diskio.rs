use std::path::{Path, PathBuf};
use std::sync::Arc;

use ignore::Walk;
use memmap2::Mmap;

use crate::core::{RenderRules, SiteEntry, SiteNode, DEFAULT_RENDER_RULE_SET};
use crate::Config;

/// Read a `path` to an `Mmap`.
pub fn read_file_contents<P: AsRef<Path>>(path: P) -> Mmap {
    let file = std::fs::File::open(path).unwrap();
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
/// Technically the site topology is a tree, but currently have no need to represent it as such.
pub fn collect_site_nodes(config: Arc<Config>) -> Vec<SiteNode> {
    // TODO probably could do less hand rolling if we just walkdir, get directories, and then
    // output a `SiteNode` for each of those.
    fn recurse(
        site_nodes: &mut Vec<SiteNode>,
        pages_dir: &Path,
        current_path: &Path,
        rules_stack: &mut Vec<Arc<RenderRules>>,
    ) {
        let rules_path = current_path.join("rules.yaml");
        if rules_path.exists() && rules_path.is_file() {
            let raw = std::fs::read_to_string(&rules_path).unwrap();
            // TODO wanted to override the previous rules on the stack...
            // TODO might not need Arc anymore, now that SiteNodes own the RuleSets?
            let rule_set: Arc<RenderRules> = Arc::new(serde_yaml::from_str(&raw).unwrap());
            rules_stack.push(rule_set);
        }

        if let Ok(dir_entries) = std::fs::read_dir(current_path) {
            let paths: Vec<PathBuf> = dir_entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .collect();
            let site_entries: Vec<SiteEntry> = paths
                .iter()
                .filter_map(|path| SiteEntry::try_new(pages_dir, path).ok())
                .collect();

            if !site_entries.is_empty() {
                site_nodes.push(SiteNode {
                    dir: current_path.strip_prefix(pages_dir).unwrap().to_path_buf(),
                    render_rules: rules_stack.last().expect("rules_stack is empty").clone(),
                    site_entries,
                });
            }

            paths.iter().filter(|path| path.is_dir()).for_each(|path| {
                recurse(site_nodes, pages_dir, path, rules_stack);
            });
        }

        // Pop off the stack when backtracking
        if rules_path.exists() && rules_path.is_file() {
            rules_stack.pop();
        }
    }

    let mut site_nodes: Vec<SiteNode> = Vec::new();
    let mut rules_stack = vec![DEFAULT_RENDER_RULE_SET.clone()];
    let pages_dir = config.pages_dir();
    recurse(&mut site_nodes, &pages_dir, &pages_dir, &mut rules_stack);
    site_nodes
}

// TODO this P, P2 thing is unseemly.
// TODO should callers just take care of joining the path?
// TODO probably also want to minify + compress
pub async fn write_html<P: AsRef<Path>>(out_path: P, html: &str) -> anyhow::Result<()> {
    tracing::debug!("Writing HTML to {}", out_path.as_ref().display());
    tokio::fs::create_dir_all(out_path.as_ref().parent().unwrap()).await?;
    tokio::fs::write(&out_path, html).await?;
    Ok(())
}

// TODO what color is my function?
pub fn write_html_sync<P: AsRef<Path>>(out_path: P, html: &str) -> anyhow::Result<()> {
    tracing::debug!("Writing HTML to {}", out_path.as_ref().display());
    std::fs::create_dir_all(out_path.as_ref().parent().unwrap())?;
    std::fs::write(&out_path, html)?;
    Ok(())
}
