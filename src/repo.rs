
use colored::Colorize;
use std::fs;
use std::path::Path;
use crate::config::{Config, RepositoryConfig};

pub fn add_repository(name: &str, url: &str, mirror: Option<&str>) -> Result<(), String> {
    let mut cfg = Config::load();
    if cfg.repositories.contains_key(name) {
        return Err(format!("Repository '{}' already exists", name));
    }
    let repo = RepositoryConfig {
        url: url.to_string(),
        mirror: mirror.map(String::from),
    };
    cfg.repositories.insert(name.to_string(), repo);
    save_config(&cfg)?;
    println!("{} Repository '{}' added.", "[ror]".green().bold(), name);
    Ok(())
}

pub fn remove_repository(name: &str) -> Result<(), String> {
    let mut cfg = Config::load();
    if !cfg.repositories.contains_key(name) {
        return Err(format!("Repository '{}' not found", name));
    }
    cfg.repositories.remove(name);
    save_config(&cfg)?;
    println!("{} Repository '{}' removed.", "[ror]".green().bold(), name);
    Ok(())
}

pub fn list_repositories() {
    let cfg = Config::load();
    if cfg.repositories.is_empty() {
        println!("{} No repositories configured.", "[ror]".yellow());
        return;
    }
    println!("{} Configured repositories:", "[ror]".blue().bold());
    for (name, repo) in &cfg.repositories {
        println!("  {} {}", name.green(), repo.url);
        if let Some(m) = &repo.mirror {
            println!("    mirror: {}", m);
        }
    }
}

fn save_config(cfg: &Config) -> Result<(), String> {
    let path = Path::new("/var/ror/ror.conf");
    let ini = cfg.to_ini().map_err(|e| format!("Failed to serialize config: {}", e))?;
    fs::write(path, ini).map_err(|e| format!("Failed to write config: {}", e))?;
    Ok(())
}



