use std::sync::Arc;

use stalagmite::diskio::walk;
use stalagmite::{generate, Config};

#[tokio::test]
async fn generate_example_site() {
    let example_project_dir = std::env::current_dir().unwrap().join("example");

    std::fs::remove_dir_all(example_project_dir.join("public")).ok();
    std::fs::remove_file(example_project_dir.join("db.sqlite")).ok();

    let config = Arc::new(Config::init(Some(example_project_dir.clone())).unwrap());

    generate(config).await.unwrap();

    let out_dir = example_project_dir.join("public");
    let files = walk(&out_dir, "html").collect::<Vec<_>>();
    // TODO kind of a fragile and tedious comparison, better to stringify + sort them.
    assert_eq!(
        files,
        vec![
            out_dir.join("index.html"),
            out_dir
                .join("blog")
                .join("welcome-to-my-blog")
                .join("index.html"),
            out_dir.join("blog").join("0").join("index.html"),
        ]
    );

    for file in walk(example_project_dir.join("public"), "html") {
        let contents = std::fs::read_to_string(&file).unwrap();
        insta::assert_yaml_snapshot!(contents);
    }
}
