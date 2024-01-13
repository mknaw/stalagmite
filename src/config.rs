use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("invalid layout file")]
    MissingLayout,
}

pub struct Config {
    pub project_dir: PathBuf,
    pub outdir: PathBuf,
}

impl Config {
    pub fn init() -> Result<Self, ConfigError> {
        let project_dir = std::env::current_dir().unwrap();
        let outdir = project_dir.join("public");
        // TODO the creation of outdir probably should happen somewhere else.
        if !outdir.is_dir() {
            std::fs::create_dir_all(&outdir).unwrap();
        }
        Ok(Self {
            project_dir,
            outdir,
        })
    }

    pub fn layouts_dir(&self) -> PathBuf {
        self.project_dir.join("layouts")
    }

    pub fn blocks_dir(&self) -> PathBuf {
        self.project_dir.join("blocks")
    }

    // TODO `partials_dir`?

    pub fn pages_dir(&self) -> PathBuf {
        self.project_dir.join("pages")
    }

    pub fn out_dir(&self) -> PathBuf {
        self.project_dir.join("public")
    }
}
