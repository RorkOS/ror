use colored::Colorize;
use crate::install::{self, InstalledDB, get_package_info};
use crate::delete;
use crate::config;
use std::collections::HashSet;
use semver::Version;

fn is_newer(current: &str, available: &str) -> bool {
    match (Version::parse(current), Version::parse(available)) {
        (Ok(cur), Ok(ava)) => ava > cur,
        _ => {
            
            current != available && available > current
        }
    }
}

pub fn update_package(pkg_name: &str, cfg: &config::Config, dry_run: bool) -> bool {
    let db = InstalledDB::load();
    let installed = match db.packages.get(pkg_name) {
        Some(p) => p,
        None => {
            eprintln!("{} Package '{}' is not installed.", "[ror]".red().bold(), pkg_name);
            return false;
        }
    };

    let info = match get_package_info(pkg_name) {
        Some(info) => info,
        None => {
            eprintln!("{} Package '{}' not found in repository.", "[ror]".red().bold(), pkg_name);
            return false;
        }
    };
    let available_version = info.1;

    if !is_newer(&installed.version, &available_version) {
        println!("{} Package '{}' is already up-to-date (version {}).", "[ror]".green(), pkg_name, installed.version);
        return false;
    }

    println!("{} Updating '{}' from {} to {}...", "[ror]".blue().bold(), pkg_name, installed.version, available_version);

    if dry_run {
        println!("{} DRY RUN: would remove and reinstall {}", ">>>".yellow(), pkg_name);
        return true;
    }

    delete::remove_package(pkg_name);
    install::install_package(pkg_name, cfg);
    true
}

pub fn upgrade_all(cfg: &config::Config, dry_run: bool) {
    let db = InstalledDB::load();
    let installed: Vec<String> = db.packages.keys().cloned().collect();
    if installed.is_empty() {
        println!("{} No packages installed.", "[ror]".yellow());
        return;
    }

    let mut updated = HashSet::new();
    let mut failed = Vec::new();

    for pkg in &installed {
        if updated.contains(pkg) {
            continue;
        }
        if update_package(pkg, cfg, dry_run) {
            updated.insert(pkg.clone());
        } else {
            failed.push(pkg.clone());
        }
    }

    if dry_run {
        println!("{} DRY RUN: {} packages would be updated.", ">>>".yellow(), updated.len());
    } else {
        println!("{} Upgrade completed. Updated: {}, failed: {}.", "[ror]".green().bold(), updated.len(), failed.len());
        if !failed.is_empty() {
            println!("{} Failed packages: {:?}", "[ror]".red(), failed);
        }
    }
}

pub fn list_upgradable() -> Vec<String> {
    let db = InstalledDB::load();
    let mut upgradable = Vec::new();
    for pkg_name in db.packages.keys() {
        let installed_version = &db.packages[pkg_name].version;
        if let Some(info) = get_package_info(pkg_name) {
            let available_version = info.1;
            if is_newer(installed_version, &available_version) {
                upgradable.push(pkg_name.clone());
            }
        }
    }
    upgradable
}
