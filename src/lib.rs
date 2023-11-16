#[macro_use]
extern crate lazy_static;

mod config;
mod devserver;
pub mod diskio;
mod generator;
pub(crate) mod liquid {
    pub(crate) mod filters {
        mod block;
        pub use block::FirstBlockOfKind;
    }
    pub(crate) mod tags {
        mod render_block;
        mod tailwind;
        pub use render_block::RenderBlockTag;
        pub use tailwind::TailwindTag;
    }
}
pub(crate) mod core;
pub(crate) mod markdown;
pub mod project;
mod render;
pub mod styles;
mod utils;

pub use core::MarkdownPage;

pub use config::Config;
pub use devserver::run as run_dev_server;
pub use generator::generate;
pub use render::Renderer;
