use colored::Colorize;
use std::fs;
use std::path::Path;
use crate::install::Package;

const REPOS_ROOT: &str = "/var/ror/packages";

fn load_package_from_path(path: &Path) -> Option<Package> {
    let content = fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&content).ok()
}

fn find_all_package_files() -> Vec<String> {
    let mut results = Vec::new();
    let root = Path::new(REPOS_ROOT);
    let Ok(level1) = fs::read_dir(root) else { return results };

    for entry1 in level1.flatten() {
        let path1 = entry1.path();
        if !path1.is_dir() || path1.file_name().unwrap_or_default() == "groups" {
            continue;
        }

        
        let Ok(level2) = fs::read_dir(&path1) else { continue };
        for entry2 in level2.flatten() {
            let path2 = entry2.path();
            if !path2.is_dir() {
                continue;
            }

            
            let yaml_path = path2.join(path2.file_name().unwrap()).with_extension("yaml");
            if yaml_path.exists() {
                if let Some(s) = yaml_path.to_str() {
                    results.push(s.to_string());
                }
                continue; 
            }

            
            let Ok(level3) = fs::read_dir(&path2) else { continue };
            for entry3 in level3.flatten() {
                let path3 = entry3.path();
                if !path3.is_dir() {
                    continue;
                }
                let yaml_path = path3.join(path3.file_name().unwrap()).with_extension("yaml");
                if yaml_path.exists() {
                    if let Some(s) = yaml_path.to_str() {
                        results.push(s.to_string());
                    }
                }
            }
        }
    }
    results
}

pub fn search_packages(query: &str) {
    let query_lower = query.to_lowercase();
    let files = find_all_package_files();

    if files.is_empty() {
        println!("{} No packages found in repositories.", "[ror]".yellow());
        return;
    }

    let mut matches = Vec::new();
    for file in files {
        if let Some(pkg) = load_package_from_path(Path::new(&file)) {
            let name_lower = pkg.name.to_lowercase();
            let desc_lower = pkg.description.clone().unwrap_or_default().to_lowercase();
            if name_lower.contains(&query_lower) || desc_lower.contains(&query_lower) {
                matches.push(pkg);
            }
        }
    }

    if matches.is_empty() {
        println!("{} No packages matching '{}'.", "[ror]".yellow(), query);
        return;
    }

    println!("{} Packages matching '{}':", "[ror]".blue().bold(), query);
    println!("{:-<60}", "");
    for pkg in matches {
        let name_ver = format!("{} {}", pkg.name, pkg.version).cyan();
        let desc = pkg.description.unwrap_or_default();
        println!("  {}  {}", name_ver, desc);
    }
}
