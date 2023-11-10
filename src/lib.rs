#[macro_use]
extern crate lazy_static;

mod config;
mod devserver;
pub mod diskio;
mod generator;
pub(crate) mod markdown;
pub(crate) mod pages;
pub mod project;
mod render;
pub mod styles;
mod utils;

pub use config::Config;
pub use devserver::run as run_dev_server;
pub use generator::generate;
pub use pages::Page;
pub use render::Renderer;
