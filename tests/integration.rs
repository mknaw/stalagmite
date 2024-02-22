use std::sync::Arc;

use camino::Utf8PathBuf;
use stalagmite::diskio::walk;
use stalagmite::{bootstrap_cache, generate, Config};

#[tokio::test]
async fn generate_example_site() {
    let example_project_dir =
        Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap().join("example")).unwrap();

    std::fs::remove_dir_all(example_project_dir.join("public")).ok();
    std::fs::remove_file(example_project_dir.join("db.sqlite")).ok();

    let config = Arc::new(Config::init(Some(example_project_dir.clone())).unwrap());

    // TODO should have an in-memory sqlite for testing.
    let pool = bootstrap_cache().unwrap();
    generate(config, Arc::new(pool)).await.unwrap();

    let out_dir = example_project_dir.join("public");
    let mut files: Vec<Utf8PathBuf> = walk(&out_dir, &Some("html"))
        .map(|p| p.strip_prefix(&out_dir).unwrap().to_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec![
            "blog/0/index.html",
            "blog/welcome-to-my-blog/index.html",
            "index.html",
        ]
    );

    for file in walk(example_project_dir.join("public"), &Some("html")) {
        let contents = std::fs::read_to_string(&file).unwrap();
        insta::assert_yaml_snapshot!(contents.split('\n').collect::<Vec<&str>>());
    }
}
