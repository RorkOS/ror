use serde::Deserialize;
use std::fs;
use std::path::Path;
use colored::Colorize;
use crate::config;
use crate::install;

#[derive(Debug, Deserialize)]
pub struct Group {
    #[allow(dead_code)]
    pub name: String,
    pub description: Option<String>,
    pub packages: Vec<String>,
}

#[allow(dead_code)]
pub fn install_group(group_name: &str, cfg: &config::Config) {
    let group_file = format!("/var/ror/packages/groups/{}.yaml", group_name);
    if !Path::new(&group_file).exists() {
        eprintln!("{} Group '{}' not found!", "[ror]".red().bold(), group_name);
        return;
    }

    let content = match fs::read_to_string(&group_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} Failed to read group file: {}", "[ror]".red().bold(), e);
            return;
        }
    };

    let group: Group = match serde_yaml::from_str(&content) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{} Group YAML error: {}", "[ror]".red().bold(), e);
            return;
        }
    };

    println!(
        "{} {} Installing group '{}' ({} packages)",
        "[ror]".blue().bold(),
        ">>>".green(),
        group.name,
        group.packages.len()
    );
    if let Some(desc) = &group.description {
        println!("{} {}", "Description:".yellow(), desc);
    }

    for pkg in &group.packages {
        println!("{} Installing package: {}", ">>>".cyan(), pkg);
        install::install_package(pkg, cfg);
    }

    println!("{} Group '{}' installation completed.", "[ror]".blue().bold(), group.name);
}

pub fn load_group(group_name: &str) -> Result<Group, String> {
    let group_file = format!("/var/ror/packages/groups/{}.yaml", group_name);
    let content = fs::read_to_string(&group_file)
        .map_err(|e| format!("Failed to read group file: {}", e))?;
    let group: Group = serde_yaml::from_str(&content)
        .map_err(|e| format!("Group YAML error: {}", e))?;
    Ok(group)
}
pub fn list_groups() {
    let groups_dir = "/var/ror/packages/groups";
    if !Path::new(groups_dir).exists() {
        println!("{} Groups directory not found.", "[ror]".yellow());
        return;
    }

    println!("{} Available package groups:", "[ror]".blue().bold());
    if let Ok(entries) = std::fs::read_dir(groups_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".yaml") {
                    let group_name = name.trim_end_matches(".yaml");
                    println!("  {}", group_name.green());
                }
            }
        }
    }
}
