use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("invalid layout file")]
    MissingLayout,
}

pub struct Config {
    pub layout: PathBuf,
    pub outdir: PathBuf,
}

impl Config {
    pub fn init() -> Result<Self, ConfigError> {
        let layout = PathBuf::from("layout.liquid");
        let outdir = PathBuf::from("public");
        if !layout.exists() {
            return Err(ConfigError::MissingLayout);
        }
        // TODO the creation of outdir probably should happen somewhere else.
        if !outdir.is_dir() {
            std::fs::create_dir_all(&outdir).unwrap();
        }
        Ok(Self { layout, outdir })
    }
}
