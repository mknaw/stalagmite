use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("invalid layout file")]
    MissingLayout,
}

pub struct Config {
    pub current_dir: PathBuf,
    pub outdir: PathBuf,
}

impl Config {
    pub fn init() -> Result<Self, ConfigError> {
        let current_dir = std::env::current_dir().unwrap();
        let outdir = current_dir.join("public");
        // TODO the creation of outdir probably should happen somewhere else.
        if !outdir.is_dir() {
            std::fs::create_dir_all(&outdir).unwrap();
        }
        Ok(Self {
            current_dir,
            outdir,
        })
    }

    pub fn layouts_dir(&self) -> PathBuf {
        self.current_dir.join("layouts")
    }

    pub fn blocks_dir(&self) -> PathBuf {
        self.current_dir.join("blocks")
    }

    pub fn pages_dir(&self) -> PathBuf {
        self.current_dir.join("pages")
    }

    pub fn out_dir(&self) -> PathBuf {
        self.current_dir.join("public")
    }
}
