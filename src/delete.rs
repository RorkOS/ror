use std::fs;
use std::path::Path;
use colored::Colorize;
use std::process::Command;
use crate::install::{load_package, InstalledDB, Package, find_package_file};

fn has_role(pkg_name: &str, role: &str) -> bool {
    if let Some(pkg) = load_package(pkg_name) {
        return pkg.provides.contains(&role.to_string());
    }
    false
}

pub fn check_critical_removal(pkg_name: &str, db: &InstalledDB) -> Result<(), String> {
    let pkg = load_package(pkg_name).ok_or("Package info not found in repository")?;

    if pkg.provides.contains(&"init-system".to_string()) {
        let other_inits_count = db.packages.values()
            .filter(|p| p.name != pkg_name && has_role(&p.name, "init-system"))
            .count();

        if other_inits_count == 0 {
            return Err(format!(
                "{} CRITICAL ERROR: '{}' is your last init-system! Removing it will break RorkOS. Install an alternative first.",
                "[ror]".red().bold(),
                pkg_name
            ));
        }
    }
    Ok(())
}

pub fn remove_package(pkg_name: &str) {
    let mut db = InstalledDB::load();

    let record = match db.packages.get(pkg_name) {
        Some(r) => r,
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

    let mut failed = false;

    for rel_path in &record.files {
        let full_path = Path::new("/").join(rel_path);
        if full_path.exists() {
            if let Err(e) = fs::remove_file(&full_path) {
                eprintln!("{} Failed to remove {}: {}", "[ror]".red().bold(), full_path.display(), e);
                failed = true;
            } else {
                println!("{} Removed {}", ">>>".green(), full_path.display());
            }
        } else {
            println!("{} File {} already missing", ">>>".yellow(), full_path.display());
        }
    }

    if let Some(pkg_path) = find_package_file(pkg_name) {
        if let Ok(content) = fs::read_to_string(pkg_path) {
            if let Ok(pkg) = serde_yaml::from_str::<Package>(&content) {
                if !pkg.delete_steps.trim().is_empty() {
                    println!("{} Running delete steps...", ">>>".yellow());
                    let status = Command::new("sh")
                        .arg("-c")
                        .arg(&pkg.delete_steps)
                        .status()
                        .expect("Failed to execute delete steps");
                    if !status.success() {
                        eprintln!("{} Delete steps failed with exit code {:?}", "[ror]".red().bold(), status.code());
                        failed = true;
                    } else {
                        println!("{} Delete steps completed", ">>>".green());
                    }
                }
            }
        }
    } else {
        println!("{} Package recipe not found, skipping delete steps", ">>>".yellow());
    }
    db.packages.remove(pkg_name);
    if let Err(e) = db.save() {
        eprintln!("{} Failed to update installed DB: {}", "[ror]".red().bold(), e);
        failed = true;
    } else {
        println!("{} Package '{}' removed from database.", "[ror]".green(), pkg_name);
    }

    if failed {
        eprintln!("{} Some errors occurred during removal.", "[ror]".red().bold());
    } else {
        println!("{} Package '{}' successfully removed.", "[ror]".green().bold(), pkg_name);
    }
}
