use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::os::unix::fs::symlink;
use std::sync::{Arc, Mutex};
use std::thread;
use colored::Colorize;
use serde::Serialize;
use serde_json;
use crate::config;
use crate::group::Group;
use crate::progress::ProgressBar;
use chrono::Local;
use crate::install::{
    Dependency, load_package, download_and_verify,
    extract_native, install_files_with_root, select_binary_for_arch,
    InstalledDB, InstalledPackage,
};

struct DownloadTask {
    pkg_name: String,
    urls: Vec<String>,
    sha256: String,
}

#[derive(Serialize)]
struct InstalledEntry {
    name: String,
    version: String,
}

#[derive(Serialize)]
struct ListInstalled {
    packages: Vec<InstalledEntry>,
}

fn collect_install_order(start_pkgs: &[String]) -> Result<Vec<String>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    fn visit(
        name: &str,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if visited.contains(name) {
            return Ok(());
        }
        visited.insert(name.to_string());

        let pkg = load_package(name)
            .ok_or_else(|| format!("Package '{}' not found in repository", name))?;

        for dep in &pkg.depends {
            match dep {
                Dependency::Single(d) => visit(d, visited, order)?,
                Dependency::Any(alts) => {
                    if let Some(first) = alts.first() {
                        visit(first, visited, order)?;
                    }
                }
            }
        }

        order.push(name.to_string());
        Ok(())
    }

    for pkg in start_pkgs {
        visit(pkg, &mut visited, &mut order)?;
    }

    Ok(order)
}

fn build_download_tasks(packages: &[String], target_arch: &str) -> Result<Vec<DownloadTask>, String> {
    let mut tasks = Vec::new();
    for pkg_name in packages {
        let pkg = load_package(pkg_name)
            .ok_or_else(|| format!("Package '{}' not found", pkg_name))?;

        let binary = select_binary_for_arch(&pkg, target_arch)
            .map_err(|e| format!("No binary for '{}': {}", pkg_name, e))?;

        let urls: Vec<String> = if !binary.url.is_empty() {
            vec![binary.url.clone()]
        } else {
            binary.mirrors.iter()
                .map(|m| m.replace("{filename}", &binary.filename))
                .collect()
        };

        if urls.is_empty() {
            return Err(format!("No download URLs for package '{}'", pkg_name));
        }

        tasks.push(DownloadTask {
            pkg_name: pkg_name.clone(),
            urls,
            sha256: binary.sha256.clone(),
        });
    }

    Ok(tasks)
}

fn download_parallel(
    tasks: Vec<DownloadTask>,
    cfg: Arc<config::Config>,
    n_parallel: usize,
) -> Result<HashMap<String, std::path::PathBuf>, String> {
    let n = n_parallel.max(1).min(tasks.len().max(1));
    let total = tasks.len();
    println!(
        "{} Downloading {} packages ({} in parallel)...",
        ">>>".cyan().bold(),
        total,
        n
    );

    let queue: Arc<Mutex<VecDeque<DownloadTask>>> =
        Arc::new(Mutex::new(tasks.into_iter().collect()));

    let results: Arc<Mutex<HashMap<String, std::path::PathBuf>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let first_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let done_count: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));

    let mut handles = vec![];

    for _ in 0..n {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);
        let first_error = Arc::clone(&first_error);
        let done_count = Arc::clone(&done_count);
        let cfg = Arc::clone(&cfg);

        handles.push(thread::spawn(move || {
            loop {
                if first_error.lock().unwrap().is_some() {
                    return;
                }

                let task = queue.lock().unwrap().pop_front();
                let task = match task {
                    Some(t) => t,
                    None => return,
                };

                let mut last_err = String::new();
                let mut downloaded = false;

                for url in &task.urls {
                    match download_and_verify(url, &task.sha256, &cfg) {
                        Ok(path) => {
                            results.lock().unwrap().insert(task.pkg_name.clone(), path);
                            downloaded = true;

                            let mut count = done_count.lock().unwrap();
                            *count += 1;
                            eprint!(
                                "\r\x1b[K{} [{}/{}] {} ",
                                ">>>".green(),
                                count,
                                total,
                                task.pkg_name
                            );
                            use std::io::Write;
                            std::io::stderr().flush().ok();
                            break;
                        }
                        Err(e) => last_err = e,
                    }
                }

                if !downloaded {
                    let mut err = first_error.lock().unwrap();
                    if err.is_none() {
                        *err = Some(format!(
                            "Failed to download '{}': {}",
                            task.pkg_name, last_err
                        ));
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().ok();
    }
    eprintln!();

    if let Some(e) = first_error.lock().unwrap().take() {
        return Err(e);
    }

    Ok(Arc::try_unwrap(results).unwrap().into_inner().unwrap())
}

fn install_from_archive(
    pkg_name: &str,
    archive_path: &std::path::Path,
    root_dir: &Path,
    binary_files: &[String],
) -> Result<Vec<String>, String> {
    let work_dir = std::path::Path::new("/tmp/ror-rootfs").join(pkg_name);
    if work_dir.exists() {
        fs::remove_dir_all(&work_dir)
            .map_err(|e| format!("Failed to clean temp dir for '{}': {}", pkg_name, e))?;
    }
    fs::create_dir_all(&work_dir)
        .map_err(|e| format!("Failed to create temp dir for '{}': {}", pkg_name, e))?;

    let extracted = extract_native(archive_path, &work_dir)
        .map_err(|e| format!("Extraction failed for '{}': {}", pkg_name, e))?;

    let file_list = if binary_files.is_empty() {
        extracted
    } else {
        binary_files.to_vec()
    };

    let installed = install_files_with_root(&work_dir, root_dir, &file_list)?;

    let _ = fs::remove_dir_all(&work_dir);
    let _ = fs::remove_file(archive_path);

    Ok(installed)
}

fn write_listinstalled(target_dir: &Path, packages: &[String]) -> Result<(), String> {
    let entries: Vec<InstalledEntry> = packages.iter()
        .filter_map(|name| {
            let pkg = load_package(name)?;
            Some(InstalledEntry { name: pkg.name, version: pkg.version })
        })
        .collect();

    let list = ListInstalled { packages: entries };
    let yaml = serde_yaml::to_string(&list)
        .map_err(|e| format!("Failed to serialize listinstalled: {}", e))?;

    let out_dir = target_dir.join("var/ror");
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create var/ror: {}", e))?;

    fs::write(out_dir.join("listinstalled.yaml"), yaml)
        .map_err(|e| format!("Failed to write listinstalled.yaml: {}", e))?;

    Ok(())
}

fn setup_usr_merge(target_dir: &Path) -> Result<(), String> {
    for dir in &["bin", "lib", "lib64", "sbin"] {
        let path = target_dir.join(dir);
        if path.symlink_metadata().is_ok() {
            if path.is_dir() && !path.is_symlink() {
                fs::remove_dir_all(&path)
                    .map_err(|e| format!("Failed to remove directory {}: {}", dir, e))?;
            } else {
                fs::remove_file(&path)
                    .map_err(|e| format!("Failed to remove {}: {}", dir, e))?;
            }
        }
    }

    for dir in &["usr/bin", "usr/lib", "usr/lib64", "usr/sbin"] {
        fs::create_dir_all(target_dir.join(dir))
            .map_err(|e| format!("Failed to create {}: {}", dir, e))?;
    }

    for (link, target) in &[
        ("bin", "usr/bin"),
        ("lib", "usr/lib"),
        ("lib64", "usr/lib64"),
        ("sbin", "usr/sbin"),
] {
        symlink(target, target_dir.join(link))
            .map_err(|e| format!("Failed to create symlink {} -> {}: {}", link, target, e))?;
    }

    Ok(())
}

fn create_fhs_skeleton(target_dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let dirs = &[
        "boot", "dev", "dev/pts", "dev/shm", "etc", "etc/ror",
        "home", "mnt", "opt", "proc", "root", "run", "srv", "sys",
        "tmp", "usr/include", "usr/local/bin", "usr/local/lib",
        "usr/local/share", "usr/share", "usr/share/doc",
        "var", "var/cache", "var/empty", "var/lib", "var/lock",
        "var/log", "var/run", "var/spool", "var/tmp",
    ];

    for dir in dirs {
        fs::create_dir_all(target_dir.join(dir))
            .map_err(|e| format!("Failed to create FHS dir {}: {}", dir, e))?;
    }

    for dir in &["tmp", "var/tmp"] {
        fs::set_permissions(target_dir.join(dir), fs::Permissions::from_mode(0o1777))
            .map_err(|e| format!("Failed to set sticky bit on {}: {}", dir, e))?;
    }

    for dir in &["root", "var/empty"] {
        fs::set_permissions(target_dir.join(dir), fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to set permissions on {}: {}", dir, e))?;
    }

    Ok(())
}

fn mount_virtual_fs(root_dir: &Path) -> Result<(), String> {
    let mounts: &[(&str, &str, &str, bool)] = &[
        ("/dev", "dev", "devtmpfs", true),
        ("/dev/pts", "dev/pts", "devpts", true),
        ("/dev/shm", "dev/shm", "tmpfs", true),
        ("proc", "proc", "proc", false),
        ("sysfs", "sys", "sysfs", false),
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

const SHELL_CANDIDATES: &[(&str, Option<&str>)] = &[
    ("/bin/sh", None),
    ("/usr/bin/sh", None),
    ("/bin/dash", None),
    ("/usr/bin/dash", None),
    ("/bin/bash", None),
    ("/usr/bin/bash", None),
    ("/bin/busybox", Some("sh")),
    ("/usr/bin/busybox", Some("sh")),
    ("/bin/toybox", Some("sh")),
    ("/bin/ash", None),
    ("/usr/bin/ash", None),
];

fn find_working_shell(root_dir: &Path) -> Option<(&'static str, Option<&'static str>)> {
    for &(shell_path, subcmd) in SHELL_CANDIDATES {
        if !root_dir.join(shell_path.trim_start_matches('/')).exists() {
            continue;
        }

        let mut cmd = Command::new("chroot");
        cmd.arg(root_dir).arg(shell_path);
        if let Some(sub) = subcmd {
            cmd.arg(sub);
        }
        cmd.arg("-c").arg("exit 0");

        match cmd.output() {
            Ok(out) if out.status.success() => return Some((shell_path, subcmd)),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                eprintln!(
                    "{} Shell {} failed in chroot ({}), trying next...",
                    "[ror]".yellow(),
                    shell_path,
                    stderr.lines().next().unwrap_or("unknown error").trim()
                );
            }
            Err(e) => {
                eprintln!("{} Cannot test shell {}: {}, trying next...", "[ror]".yellow(), shell_path, e);
            }
        }
    }
    None
}

fn run_install_steps_in_chroot(
    pkg_name: &str,
    steps: &str,
    root_dir: &Path,
    shell: &str,
    subcmd: Option<&str>,
) -> Result<(), String> {
    let script_rel = "/.ror-install-steps.sh";
    let script_host = root_dir.join(script_rel.trim_start_matches('/'));

    fs::write(&script_host, format!("#!/bin/sh\nset -e\n{}", steps))
        .map_err(|e| format!("Failed to write install script: {}", e))?;

    let mut cmd = Command::new("chroot");
    cmd.arg(root_dir).arg(shell);
    if let Some(sub) = subcmd {
        cmd.arg(sub);
    }
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
    for e in &extras {
        queue.push(e.as_str());
    }

    let steps_pkgs: Vec<String> = queue.iter()
        .filter(|&&pkg_name| {
            all_installed.contains(pkg_name) &&
            load_package(pkg_name).map(|p| !p.install_steps.trim().is_empty()).unwrap_or(false)
        })
        .map(|s| s.to_string())
        .collect();

    let mut pb = ProgressBar::new(steps_pkgs.len().max(1), "Running install steps...");

    let (shell, subcmd) = find_working_shell(target_dir)
        .ok_or_else(|| "No working shell found in rootfs. Tried: sh, dash, bash, busybox, ash".to_string())?;

    println!("{} Using shell: {}{}", ">>>".cyan(), shell,
        subcmd.map(|s| format!(" {}", s)).unwrap_or_default());

    for pkg_name in queue {
        if done.contains(pkg_name) || !all_installed.contains(pkg_name) {
            continue;
        }
        done.insert(pkg_name.to_string());

        if let Some(pkg) = load_package(pkg_name) {
            if !pkg.install_steps.trim().is_empty() {
                pb.inc(&format!("install_steps: {}", pkg_name));
                run_install_steps_in_chroot(pkg_name, &pkg.install_steps, target_dir, shell, subcmd)?;
            }
        }
    }
    pb.finish("All install steps done");
    Ok(())
}

fn write_installed_json(
    target_dir: &Path,
    packages: &[String],
    files_map: &HashMap<String, Vec<String>>,
) -> Result<(), String> {
    let mut db = InstalledDB::default();
    let now = Local::now().to_rfc3339();
    for name in packages {
        if let Some(pkg) = load_package(name) {
            let files = files_map.get(name).cloned().unwrap_or_default();
            db.packages.insert(name.clone(), InstalledPackage {
                name: pkg.name,
                version: pkg.version,
                files,
                installed_at: now.clone(),
            });
        }
    }
    let json = serde_json::to_string_pretty(&db)
        .map_err(|e| format!("Failed to serialize installed.json: {}", e))?;
    let out_dir = target_dir.join("etc/ror");
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create etc/ror: {}", e))?;
    fs::write(out_dir.join("installed.json"), json)
        .map_err(|e| format!("Failed to write installed.json: {}", e))?;
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

    println!(
        "{} Building rootfs in {} with group '{}'",
        "[ror]".blue().bold(),
        target_dir.display(),
        group_name
    );
    if let Some(desc) = &group.description {
        println!("{} {}", "Description:".yellow(), desc);
    }

    println!("\n{} Phase 0: UsrMerge...", ">>>".cyan().bold());
    setup_usr_merge(target_dir)?;
    println!("{} bin/lib/lib64/sbin are now symlinks into usr/", ">>>".green());

    println!("{} Creating FHS skeleton...", ">>>".cyan());
    create_fhs_skeleton(target_dir)?;
    println!("{} FHS skeleton created.", ">>>".green());

    println!("\n{} Phase 1a: Resolving packages...", ">>>".cyan().bold());
    let all_packages = collect_install_order(&group.packages)?;
    println!("{} {} packages to install (including dependencies)", ">>>".green(), all_packages.len());

    println!("\n{} Phase 1b: Building download queue...", ">>>".cyan().bold());
    let tasks = build_download_tasks(&all_packages, target_arch)?;

    let n_parallel = cfg.global.parallel_downloads.max(1);
    let cfg_arc = Arc::new(cfg.clone());
    let downloaded = download_parallel(tasks, cfg_arc, n_parallel)?;
    println!("{} All downloads complete.", ">>>".green().bold());

    println!("\n{} Phase 1c: Installing files...", ">>>".cyan().bold());
    let mut installing: HashSet<String> = HashSet::new();
    let mut installed_files_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut pb = ProgressBar::new(all_packages.len().max(1), "Installing...");

    for pkg_name in &all_packages {
        let pkg = load_package(pkg_name)
            .ok_or_else(|| format!("Package '{}' not found", pkg_name))?;

        let binary = select_binary_for_arch(&pkg, target_arch)
            .map_err(|e| format!("Arch selection failed for '{}': {}", pkg_name, e))?;

        let archive_path = downloaded.get(pkg_name)
            .ok_or_else(|| format!("Archive for '{}' was not downloaded", pkg_name))?;

        pb.inc(&format!("Installing {}", pkg_name));
        let files = install_from_archive(pkg_name, archive_path, target_dir, &binary.files)?;
        installed_files_map.insert(pkg_name.clone(), files);
        installing.insert(pkg_name.clone());
    }
    pb.finish(&format!("{} packages installed", installing.len()));

    println!("\n{} Writing listinstalled.yaml...", ">>>".cyan());
    write_listinstalled(target_dir, &all_packages)?;
    println!("{} listinstalled.yaml written to var/ror/", ">>>".green());

    println!("{} Writing installed.json...", ">>>".cyan());
    write_installed_json(target_dir, &all_packages, &installed_files_map)?;
    println!("{} installed.json written to etc/ror/", ">>>".green());

    if run_ldconfig {
        println!("{} Running ldconfig...", ">>>".cyan());
        let ld_conf = target_dir.join("etc/ld.so.conf");
        if !ld_conf.exists() {
            let _ = fs::write(&ld_conf, "/usr/lib\n/usr/lib64\n");
        }
        match Command::new("ldconfig").arg("-r").arg(target_dir).status() {
            Ok(s) if s.success() => println!("{} ldconfig done.", ">>>".green()),
            _ => eprintln!("{} ldconfig failed (non-fatal)", "[ror]".yellow()),
        }
    }

    println!("\n{} Phase 2: Running install_steps in chroot...", ">>>".cyan().bold());
    println!("{} Mounting /dev /proc /sys...", ">>>".cyan());
    mount_virtual_fs(target_dir)?;

    let result = run_all_install_steps(&group.packages, &installing, target_dir);

    println!("{} Unmounting virtual filesystems...", ">>>".cyan());
    umount_virtual_fs(target_dir);

    result?;

    println!(
        "\n{} Rootfs built successfully at {} ",
        "[ror]".green().bold(),
        target_dir.display()
    );
    Ok(())
}
