use colored::*;
use std::path::Path;
use std::process::Command;
use crate::config::Config;

const REPOS_DIR: &str = "/var/ror/packages";  

pub fn run_sync(cfg: &Config) {
    if cfg.repositories.is_empty() {
        eprintln!("{} No repositories configured.", "[ror]".red().bold());
        return;
    }

    for (name, repo) in &cfg.repositories {
        println!("{} Syncing repository '{}'...", "[ror]".blue().bold(), name);
        let repo_path = Path::new(REPOS_DIR).join(name);

        if repo_path.exists() {
            println!("{} Updating {}...", ">>>".green(), name);
            let status = Command::new("git")
                .args(&["-C", repo_path.to_str().unwrap(), "pull"])
                .status()
                .expect("Failed to execute git pull");

            if !status.success() && repo.mirror.is_some() {
                eprintln!("{} Pull failed, trying mirror...", "[ror]".red());
                let mirror_url = repo.mirror.as_ref().unwrap();
                let mirror_status = Command::new("git")
                    .args(&["-C", repo_path.to_str().unwrap(), "pull", mirror_url])
                    .status()
                    .expect("Failed to execute git pull from mirror");
                if mirror_status.success() {
                    println!("{} Updated from mirror.", ">>>".green());
                } else {
                    eprintln!("{} Mirror also failed.", "[ror]".red());
                }
            } else if !status.success() {
                eprintln!("{} Git pull failed, no mirror configured.", "[ror]".red());
            } else {
                println!("{} Repository updated.", ">>>".green());
            }
        } else {
            println!("{} Cloning {}...", ">>>".green(), name);
            let primary_url = &repo.url;  
            let mut status = Command::new("git")
                .args(&["clone", primary_url, repo_path.to_str().unwrap()])
                .status()
                .expect("Failed to execute git clone");

            if !status.success() && repo.mirror.is_some() {
                eprintln!("{} Clone from primary failed, trying mirror...", "[ror]".red());
                let mirror_url = repo.mirror.as_ref().unwrap();
                status = Command::new("git")
                    .args(&["clone", mirror_url, repo_path.to_str().unwrap()])
                    .status()
                    .expect("Failed to execute git clone from mirror");
                if status.success() {
                    println!("{} Cloned from mirror.", ">>>".green());
                } else {
                    eprintln!("{} Mirror also failed.", "[ror]".red());
                }
            } else if !status.success() {
                eprintln!("{} Git clone failed.", "[ror]".red());
            } else {
                println!("{} Repository cloned.", ">>>".green());
            }
        }
    }
}
