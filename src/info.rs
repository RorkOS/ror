use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use colored::Colorize;

const REPO_ROOT: &str = "/var/ror/packages";

fn find_package_file(pkg_name: &str) -> Option<PathBuf> {
    let repo = Path::new(REPO_ROOT);
    for entry in fs::read_dir(repo).ok()? {
        let entry = entry.ok()?;
        let cat_path = entry.path();
        if !cat_path.is_dir() {
            continue;
        }
        let pkg_path = cat_path.join(pkg_name).join(format!("{}.yaml", pkg_name));
        if pkg_path.exists() && pkg_path.is_file() {
            return Some(pkg_path);
        }
    }
    None
}

#[derive(Debug, Serialize, Deserialize)]
struct Package {
    name: String,
    version: String,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    binaries: Vec<BinaryPackage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BinaryPackage {
    #[serde(rename = "type", default)]
    pkg_type: Option<String>,
    arch: String,
    filename: String,
    mirrors: Vec<String>,
    sha256: String,
}
pub fn print_package_info(pkg_name: &str) {
    let pkg_path = match find_package_file(pkg_name) {
        Some(p) => p,
        None => {
            eprintln!(
                "{} Package '{}' not found!",
                "[ror]".red().bold(),
                pkg_name
            );
            return;
        }
    };

    let content = match fs::read_to_string(&pkg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to read package file: {}",
                "[ror]".red().bold(),
                e
            );
            return;
        }
    };

    let pkg: Package = match serde_yaml::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{} YAML structure error: {}",
                "[ror]".red().bold(),
                e
            );
            return;
        }
    };

    println!("\n{} {}", "[ Package Info ]".cyan().bold(), "=".repeat(50).cyan());

    println!("{} {}{}",
        "Name:".yellow().bold(),
        pkg.name,
        if let Some(rel) = &pkg.release { format!("-{}", rel) } else { String::new() }
    );
    println!("{} {}", "Version:".yellow().bold(), pkg.version);

    if let Some(lic) = &pkg.license {
        println!("{} {}", "License:".yellow().bold(), lic);
    }
    if let Some(home) = &pkg.homepage {
        println!("{} {}", "Homepage:".yellow().bold(), home);
    }
    if let Some(desc) = &pkg.description {
        println!("{} {}", "Description:".yellow().bold(), desc);
    }
  
    if !pkg.binaries.is_empty() {
    println!("\n{}", "[ Binaries ]".green().bold());
    for bin in &pkg.binaries {
        if let Some(t) = &bin.pkg_type {
            println!("  {} [{}]", "Type:".white(), t);
        }
        println!("  {} {}", "Arch:".white(), bin.arch);
        println!("  {} {}", "Filename:".white(), bin.filename);
        println!("  {} {}...", "SHA256:".white(), &bin.sha256[..16]);
        println!("  {} {} mirrors", "Mirrors:".white(), bin.mirrors.len());
        println!();
    }
}
    println!("{}", "=".repeat(60).cyan());
}
