use std::fs;
use std::path::Path;
use colored::Colorize;
use crate::install::{load_package, InstalledDB};

const CRITICAL_ROLES: &[&str] = &["init-system", "sh"];

fn has_role(pkg_name: &str, role: &str) -> bool {
    load_package(pkg_name)
        .map(|p| p.provides.contains(&role.to_string()))
        .unwrap_or(false)
}

pub fn check_critical_removal(pkg_name: &str, db: &InstalledDB) -> Result<(), String> {
    let pkg = load_package(pkg_name).ok_or("Package info not found in repository")?;

    for &role in CRITICAL_ROLES {
        if !pkg.provides.contains(&role.to_string()) {
            continue;
        }
        let others = db.packages.values()
            .filter(|p| p.name != pkg_name && has_role(&p.name, role))
            .count();
        if others == 0 {
            return Err(format!(
                "{} CRITICAL: '{}' is your only provider of '{}'. Install an alternative first, or use --di to replace atomically.",
                "[ror]".red().bold(),
                pkg_name,
                role
            ));
        }
    }

    Ok(())
}

pub fn check_critical_removal_with_replacement(pkg_name: &str, replacement: &str, db: &InstalledDB) -> Result<(), String> {
    let pkg = load_package(pkg_name).ok_or("Package info not found in repository")?;
    let replacement_pkg = load_package(replacement).ok_or("Replacement package not found in repository")?;

    for &role in CRITICAL_ROLES {
        if !pkg.provides.contains(&role.to_string()) {
            continue;
        }
        let replacement_covers = replacement_pkg.provides.contains(&role.to_string());
        let other_installed = db.packages.values()
            .filter(|p| p.name != pkg_name && has_role(&p.name, role))
            .count();
        if !replacement_covers && other_installed == 0 {
            return Err(format!(
                "{} CRITICAL: removing '{}' would leave the system without a '{}' provider, and '{}' does not provide it.",
                "[ror]".red().bold(),
                pkg_name,
                role,
                replacement
            ));
        }
    }

    Ok(())
}

pub fn check_critical_install(pkg_name: &str, db: &InstalledDB) -> Result<(), String> {
    let pkg = load_package(pkg_name).ok_or("Package info not found in repository")?;

    for &role in CRITICAL_ROLES {
        if !pkg.provides.contains(&role.to_string()) {
            continue;
        }
        if let Some(existing) = db.packages.values().find(|p| has_role(&p.name, role)) {
            return Err(format!(
                "{} CRITICAL: '{}' provides '{}', but '{}' already provides it. Use --di {} {} to replace it atomically.",
                "[ror]".red().bold(),
                pkg_name,
                role,
                existing.name,
                existing.name,
                pkg_name
            ));
        }
    }

    Ok(())
}

pub fn remove_package_files_only(_pkg_name: &str, files: &[String]) -> bool {
    let mut failed = false;

    for rel_path in files {
        let full_path = Path::new("/").join(rel_path);
        if full_path.exists() {
            if let Err(e) = fs::remove_file(&full_path) {
                eprintln!("{} Failed to remove {}: {}", "[ror]".red().bold(), full_path.display(), e);
                failed = true;
            } else {
                println!("{} Removed {}", ">>>".green(), full_path.display());
            }
        }
    }

    !failed
}

pub fn remove_package(pkg_name: &str) {
    let mut db = InstalledDB::load();

    let files: Vec<String> = match db.packages.get(pkg_name) {
        Some(r) => r.files.iter().cloned().collect(),
        None => {
            eprintln!("{} Package '{}' is not installed.", "[ror]".red().bold(), pkg_name);
            return;
        }
    };

    if let Err(e) = check_critical_removal(pkg_name, &db) {
        eprintln!("{}", e);
        return;
    }

    println!("{} Removing package '{}'...", "[ror]".blue().bold(), pkg_name);

    let success = remove_package_files_only(pkg_name, &files);

    db.packages.remove(pkg_name);
    if let Err(e) = db.save() {
        eprintln!("{} Failed to update installed DB: {}", "[ror]".red().bold(), e);
    } else {
        println!("{} Package '{}' removed from database.", "[ror]".green(), pkg_name);
    }

    if !success {
        eprintln!("{} Some errors occurred while removing '{}'.", "[ror]".red().bold(), pkg_name);
    } else {
        println!("{} Package '{}' successfully removed.", "[ror]".green().bold(), pkg_name);
    }
}
