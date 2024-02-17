use camino::Utf8PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("invalid layout file")]
    MissingLayout,
}

#[derive(Debug)]
pub struct Config {
    pub project_dir: Utf8PathBuf,
    pub outdir: Utf8PathBuf,
}

impl Config {
    pub fn init(project_dir: Option<Utf8PathBuf>) -> Result<Self, ConfigError> {
        let project_dir = project_dir.unwrap_or_else(|| {
            Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap()
        });
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

    pub fn layouts_dir(&self) -> Utf8PathBuf {
        self.project_dir.join("layouts")
    }

    pub fn blocks_dir(&self) -> Utf8PathBuf {
        self.project_dir.join("blocks")
    }

    // TODO `partials_dir`?

    pub fn pages_dir(&self) -> Utf8PathBuf {
        self.project_dir.join("pages")
    }

    pub fn out_dir(&self) -> Utf8PathBuf {
        self.project_dir.join("public")
    }
}
