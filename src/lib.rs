#![feature(once_cell_try)]
#![feature(let_chains)]

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
pub(crate) mod assets;
pub(crate) mod cache;
pub(crate) mod core;
pub(crate) mod parsers;
pub mod project;
mod renderer;
mod utils;

pub use core::Markdown;

pub use config::Config;
pub use devserver::run as run_dev_server;
pub use generator::generate;
pub use renderer::Renderer;
