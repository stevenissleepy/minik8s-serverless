use anyhow::{Result, bail};
use std::path::Path;

use crate::FuncConfig;

mod python;
mod rustlang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionRuntime {
    Python,
    Rust,
}

impl FunctionRuntime {
    pub(crate) fn from_language(language: &str) -> Result<Self> {
        match normalize(language).as_str() {
            "python" | "py" => Ok(Self::Python),
            "rust" | "rs" => Ok(Self::Rust),
            other => {
                bail!("unsupported function runtime {other}; supported runtimes: python, rust")
            }
        }
    }

    pub(crate) fn from_config(runtime: &str) -> Result<Self> {
        Self::from_language(runtime)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Rust => "rust",
        }
    }

    pub(crate) fn create(self, dir: &Path, name: &str) -> Result<()> {
        match self {
            Self::Python => python::create(dir, name),
            Self::Rust => rustlang::create(dir, name),
        }
    }

    pub(crate) fn build_image(
        self,
        source_dir: &Path,
        config: &FuncConfig,
        image: &str,
    ) -> Result<()> {
        match self {
            Self::Python => python::build_image(source_dir, config, image),
            Self::Rust => rustlang::build_image(source_dir, config, image),
        }
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
