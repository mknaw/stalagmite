#![feature(once_cell_try)]
#![feature(let_chains)]

#[macro_use]
extern crate lazy_static;

mod config;
pub mod diskio;
mod generator;
mod server;
pub(crate) mod liquid {
    pub(crate) mod filters {
        mod block;
        pub use block::FirstBlockOfKind;
    }
    pub(crate) mod tags {
        mod render_block;
        mod static_asset;
        mod tailwind;
        pub use render_block::RenderBlockTag;
        pub use static_asset::StaticAssetTag;
        pub use tailwind::TailwindTag;
    }
}
pub(crate) mod assets;
pub(crate) mod cache;
pub(crate) mod common;
pub(crate) mod parsers;
pub mod project;
mod renderer;
mod utils;

pub use common::Markdown;
pub use config::Config;
pub use generator::generate;
pub use renderer::Renderer;
pub use server::run as run_server;
