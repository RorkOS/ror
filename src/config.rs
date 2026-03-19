use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use colored::Colorize;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RepositoryConfig {
    pub url: String,
    #[serde(default)]
    pub mirror: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct GlobalConfig {
    #[serde(default = "default_ignore_speed")]
    pub ignore_speed: bool,
    #[serde(default = "default_strict_gpg")]
    pub strict_gpg: bool,
    #[serde(default)]
    pub allow_external_binaries: bool,
}

fn default_ignore_speed() -> bool { false }
fn default_strict_gpg() -> bool { false }

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub repositories: HashMap<String, RepositoryConfig>,
}

impl Config {
    pub fn load() -> Self {
        let path = Path::new("/var/ror/ror.conf");
        if path.exists() {
            let content = fs::read_to_string(path).unwrap_or_default();
            Self::from_ini(&content).unwrap_or_else(|e| {
                eprintln!(
                    "{} Failed to parse config: {}, using defaults",
                    "[ror]".red().bold(),
                    e
                );
                Config::default()
            })
        } else {
            Config::default()
        }
    }
    pub fn to_ini(&self) -> Result<String, config::ConfigError> {
        let mut lines = vec!["[global]".to_string()];
        lines.push(format!("ignore_speed = {}", self.global.ignore_speed));
        lines.push(format!("strict_gpg = {}", self.global.strict_gpg));
        lines.push("".to_string());
        for (name, repo) in &self.repositories {
            lines.push(format!("[repositories.{}]", name));
            lines.push(format!("url = \"{}\"", repo.url));
            if let Some(m) = &repo.mirror {
                lines.push(format!("mirror = \"{}\"", m));
            }
            lines.push("".to_string());
        }
        Ok(lines.join("\n"))
    }

    pub fn from_ini(ini: &str) -> Result<Self, config::ConfigError> {
        let builder = config::Config::builder()
            .add_source(config::File::from_str(ini, config::FileFormat::Ini));
        let cfg = builder.build()?;
        cfg.try_deserialize()
    }

    pub fn create_default_config(path: &Path) -> std::io::Result<()> {
        let ini = r#"[global]
ignore_speed = false
strict_gpg = false

[repositories.rorkos]
url = "https:
mirror = "https:
"#;
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, ini)
    }
}

