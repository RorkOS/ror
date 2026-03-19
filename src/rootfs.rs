use std::fs;
use std::path::Path;
use std::collections::HashSet;
use std::process::Command;
use colored::Colorize;
use crate::config;
use crate::group::Group;
use crate::install::{
    Dependency, Package, load_package, download_and_verify,
    extract_native, install_files_with_root, select_binary_for_arch,
};





fn mount_virtual_fs(root_dir: &Path) -> Result<(), String> {
    
    let mounts: &[(&str, &str, &str, bool)] = &[
        ("/dev",     "dev",     "devtmpfs", true),
        ("/dev/pts", "dev/pts", "devpts",   true),
        ("/dev/shm", "dev/shm", "tmpfs",    true),
        ("proc",     "proc",    "proc",     false),
        ("sysfs",    "sys",     "sysfs",    false),
    ];

    for (src, rel, fstype, bind) in mounts {
        let target = root_dir.join(rel);
        fs::create_dir_all(&target)
            .map_err(|e| format!("Failed to create mountpoint {}: {}", rel, e))?;

        let status = if *bind {
            Command::new("mount")
                .args(["--bind", src, target.to_str().unwrap()])
                .status()
        } else {
            Command::new("mount")
                .args(["-t", fstype, src, target.to_str().unwrap()])
                .status()
        };

        match status {
            Ok(s) if s.success() => {}
            Ok(_) => eprintln!("{} Warning: failed to mount {} (non-fatal)", "[ror]".yellow(), rel),
            Err(e) => eprintln!("{} Warning: mount {}: {} (non-fatal)", "[ror]".yellow(), rel, e),
        }
    }
    Ok(())
}


fn umount_virtual_fs(root_dir: &Path) {
    for rel in &["dev/pts", "dev/shm", "dev", "proc", "sys"] {
        let target = root_dir.join(rel);
        if target.exists() {
            let _ = Command::new("umount").args(["-l", target.to_str().unwrap()]).status();
        }
    }
}







fn find_shell_in_rootfs(root_dir: &Path) -> Option<(&'static str, Option<&'static str>)> {
    let candidates: &[(&str, Option<&str>)] = &[
        ("/bin/sh",          None),
        ("/usr/bin/sh",      None),
        ("/bin/bash",        None),
        ("/usr/bin/bash",    None),
        ("/bin/busybox",     Some("sh")),
        ("/usr/bin/busybox", Some("sh")),
    ];
    candidates.iter()
        .find(|(p, _)| root_dir.join(p.trim_start_matches('/')).exists())
        .copied()
}





fn run_install_steps_in_chroot(pkg_name: &str, steps: &str, root_dir: &Path) -> Result<(), String> {
    let (shell, subcmd) = find_shell_in_rootfs(root_dir)
        .ok_or_else(|| format!(
            "No shell found in rootfs for '{}'. Checked: /bin/sh, /bin/bash, /bin/busybox",
            pkg_name
        ))?;

    println!("{} Using shell: {}{}", ">>>".cyan(), shell,
        subcmd.map(|s| format!(" {}", s)).unwrap_or_default());

    let script_rel = "/.ror-install-steps.sh";
    let script_host = root_dir.join(script_rel.trim_start_matches('/'));
    
    fs::write(&script_host, format!("#!/bin/sh\nset -e\n{}", steps))
        .map_err(|e| format!("Failed to write install script: {}", e))?;

    let mut cmd = Command::new("chroot");
    cmd.arg(root_dir).arg(shell);
    if let Some(sub) = subcmd { cmd.arg(sub); }
    cmd.arg(script_rel);

    let status = cmd.status()
        .map_err(|e| format!("Failed to execute chroot for {}: {}", pkg_name, e))?;

    let _ = fs::remove_file(&script_host);

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "install_steps for '{}' failed with exit code {:?}",
            pkg_name, status.code()
        ))
    }
}





fn process_dependencies_chroot(
    pkg: &Package,
    cfg: &config::Config,
    root_dir: &Path,
    installing: &mut HashSet<String>,
    target_arch: &str,
) -> Result<(), String> {
    for dep in &pkg.depends {
        match dep {
            Dependency::Single(name) => {
                if !installing.contains(name) {
                    install_package_in_chroot(name, cfg, root_dir, installing, target_arch)?;
                }
            }
            Dependency::Any(alternatives) => {
                if alternatives.iter().any(|alt| installing.contains(alt)) {
                    continue;
                }
                let chosen = alternatives.first()
                    .ok_or_else(|| format!("Empty alternative list in package {}", pkg.name))?;
                install_package_in_chroot(chosen, cfg, root_dir, installing, target_arch)?;
            }
        }
    }
    Ok(())
}

pub fn install_package_in_chroot(
    pkg_name: &str,
    cfg: &config::Config,
    root_dir: &Path,
    installing: &mut HashSet<String>,
    target_arch: &str,
) -> Result<(), String> {
    if installing.contains(pkg_name) {
        eprintln!("{} Circular dependency detected: {}", "[ror]".red().bold(), pkg_name);
        return Ok(());
    }
    installing.insert(pkg_name.to_string());

    let pkg = load_package(pkg_name)
        .ok_or_else(|| format!("Package '{}' not found", pkg_name))?;

    process_dependencies_chroot(&pkg, cfg, root_dir, installing, target_arch)?;

    let binary = select_binary_for_arch(&pkg, target_arch)
        .map_err(|e| format!("Failed to select binary for {}: {}", pkg_name, e))?;

    let mut last_err = None;
    for mirror in &binary.mirrors {
        let url = mirror.replace("{filename}", &binary.filename);
        println!("{} Trying mirror: {}", ">>>".yellow(), url);

        let archive_path = match download_and_verify(&url, &binary.sha256, cfg) {
            Ok(p) => p,
            Err(e) => { last_err = Some(e); continue; }
        };

        let work_dir = Path::new("/tmp/ror-install").join(&pkg.name);
        if work_dir.exists() {
            fs::remove_dir_all(&work_dir)
                .map_err(|e| format!("Failed to clean temp dir: {}", e))?;
        }
        fs::create_dir_all(&work_dir)
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let files = extract_native(&archive_path, &work_dir)
            .map_err(|e| format!("Extraction failed: {}", e))?;

        let file_list = if binary.files.is_empty() { files } else { binary.files.clone() };

        install_files_with_root(&work_dir, root_dir, &file_list)?;

        let _ = fs::remove_dir_all(&work_dir);
        let _ = fs::remove_file(&archive_path);

        println!("{} Installed {} into {}", ">>>".green(), pkg_name, root_dir.display());
        return Ok(());
    }

    Err(last_err.unwrap_or_else(|| "All mirrors failed".into()))
}





fn run_all_install_steps(
    group_order: &[String],
    all_installed: &HashSet<String>,
    target_dir: &Path,
) -> Result<(), String> {
    let mut done: HashSet<String> = HashSet::new();

    
    let mut queue: Vec<&str> = group_order.iter().map(|s| s.as_str()).collect();
    
    let mut extras: Vec<String> = all_installed.iter()
        .filter(|p| !group_order.contains(p))
        .cloned()
        .collect();
    extras.sort(); 
    for e in &extras { queue.push(e.as_str()); }

    for pkg_name in queue {
        if done.contains(pkg_name) || !all_installed.contains(pkg_name) { continue; }
        done.insert(pkg_name.to_string());

        if let Some(pkg) = load_package(pkg_name) {
            if !pkg.install_steps.trim().is_empty() {
                println!("{} install_steps: {}", ">>>".cyan(), pkg_name);
                run_install_steps_in_chroot(pkg_name, &pkg.install_steps, target_dir)?;
            }
        }
    }
    Ok(())
}





pub fn build_rootfs(
    group_name: &str,
    target_dir: &Path,
    cfg: &config::Config,
    target_arch: &str,
    run_ldconfig: bool,
) -> Result<(), String> {
    if !target_dir.exists() {
        fs::create_dir_all(target_dir)
            .map_err(|e| format!("Failed to create target directory: {}", e))?;
    } else {
        let mut entries = target_dir.read_dir()
            .map_err(|e| format!("Failed to read target directory: {}", e))?;
        if entries.next().is_some() {
            return Err(format!("Target directory {} is not empty", target_dir.display()));
        }
    }

    let group_file = format!("/var/ror/packages/groups/{}.yaml", group_name);
    let content = fs::read_to_string(&group_file)
        .map_err(|e| format!("Failed to read group file: {}", e))?;
    let group: Group = serde_yaml::from_str(&content)
        .map_err(|e| format!("Group YAML error: {}", e))?;

    println!("{} Building rootfs in {} with group '{}'",
        "[ror]".blue().bold(), target_dir.display(), group_name);
    if let Some(desc) = &group.description {
        println!("{} {}", "Description:".yellow(), desc);
    }

    
    println!("\n{} Phase 1: installing files...", ">>>".cyan().bold());
    let mut installing: HashSet<String> = HashSet::new();
    for pkg in &group.packages {
        install_package_in_chroot(pkg, cfg, target_dir, &mut installing, target_arch)?;
    }
    println!("{} Phase 1 complete: {} packages.", ">>>".green(), installing.len());

    if run_ldconfig {
        println!("{} Running ldconfig...", ">>>".cyan());
        match Command::new("ldconfig").arg("-r").arg(target_dir).status() {
            Ok(s) if s.success() => println!("{} ldconfig done.", ">>>".green()),
            _ => eprintln!("{} ldconfig failed (non-fatal)", "[ror]".yellow()),
        }
    }

    println!("\n{} Phase 2: running install_steps in chroot...", ">>>".cyan().bold());
    println!("{} Mounting /dev /proc /sys...", ">>>".cyan());
    mount_virtual_fs(target_dir)?;

    let result = run_all_install_steps(&group.packages, &installing, target_dir);

    println!("{} Unmounting virtual filesystems...", ">>>".cyan());
    umount_virtual_fs(target_dir);

    result?;

    println!("\n{} Rootfs created successfully at {}",
        "[ror]".green().bold(), target_dir.display());
    Ok(())
}
