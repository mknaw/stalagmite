use std::sync::Arc;

use camino::Utf8PathBuf;
use criterion::{criterion_group, criterion_main, Criterion};
use stalagmite::{generate, Config};

/// This is probably not a _great_ benchmark, but it's a start.
fn benchmark_generate(c: &mut Criterion) {
    let example_project_dir =
        Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap().join("example")).unwrap();

    std::fs::remove_dir_all(example_project_dir.join("public")).ok();
    std::fs::remove_file(example_project_dir.join("db.sqlite")).ok();

    c.bench_function("generate", |b| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        b.to_async(rt).iter(|| {
            // TODO probably best to not include this within the benchmark...
            let config = Arc::new(Config::init(Some(example_project_dir.clone())).unwrap());
            // TODO should have an in-memory sqlite for testing.
            generate(config)
        });
    });
}

criterion_group!(benches, benchmark_generate);
criterion_main!(benches);
