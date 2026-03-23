use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use colored::Colorize;
use std::process::Command;
use reqwest::blocking::Client;
use sha2::{Sha256, Digest};
use walkdir::WalkDir;
use chrono::Local;
use std::collections::HashMap;
use std::os::unix::fs::symlink;
use crate::config;
use crate::progress::{Spinner, format_bytes};

const REPO_ROOT: &str = "/var/ror/packages";

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Single(String),
    Any(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub binaries: Vec<BinaryPackage>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub install_steps: String,
    #[serde(default)]
    pub delete_steps: String,
    #[serde(default)]
    pub depends: Vec<Dependency>,
    #[serde(default)]
    pub conflicts: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BinaryPackage {
    pub arch: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub mirrors: Vec<String>,
    #[serde(rename = "type", default)]
    pub pkg_type: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default = "default_install_prefix")]
    pub install_prefix: String,
    #[serde(default)]
    pub files: Vec<String>,
}

fn default_install_prefix() -> String {
    "/".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub build_type: String,
    pub files: Vec<String>,
    pub installed_at: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct InstalledDB {
    pub packages: HashMap<String, InstalledPackage>,
}

impl InstalledDB {
    pub const PATH: &'static str = "/etc/ror/installed.json";

    pub fn load() -> Self {
        if Path::new(Self::PATH).exists() {
            let content = fs::read_to_string(Self::PATH).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let path = Path::new(Self::PATH);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)
    }

    pub fn add_package(&mut self, pkg: InstalledPackage) {
        self.packages.insert(pkg.name.clone(), pkg);
    }

    pub fn is_installed(&self, name: &str) -> bool {
        self.packages.contains_key(name)
    }
}

fn sort_mirrors_by_speed(mirrors: &[String]) -> Vec<String> {
    use std::time::Instant;
    let client = match Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return mirrors.to_vec(),
    };

    let mut timed: Vec<(u128, String)> = mirrors
        .iter()
        .map(|url| {
            let start = Instant::now();
            let ok = client
                .head(url)
                .send()
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            let elapsed = start.elapsed().as_millis();
            if ok { (elapsed, url.clone()) } else { (u128::MAX, url.clone()) }
        })
        .collect();

    timed.sort_by_key(|(t, _)| *t);
    timed.into_iter().map(|(_, url)| url).collect()
}

pub fn extract_deb(archive_path: &Path, dest_root: &Path) -> Result<Vec<String>, String> {
    let file = fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let mut archive = ar::Archive::new(file);
    let mut installed_files = Vec::new();

    while let Some(entry_result) = archive.next_entry() {
        let entry = entry_result.map_err(|e| format!("Failed to read ar entry: {}", e))?;
        let identifier = std::str::from_utf8(entry.header().identifier()).unwrap_or("");

        if identifier.contains("data.tar") {
            let reader: Box<dyn std::io::Read> = if identifier.ends_with(".xz") {
                Box::new(xz2::read::XzDecoder::new(entry))
            } else {
                Box::new(entry)
            };

            let mut tar_archive = tar::Archive::new(reader);
            let entries = tar_archive.entries().map_err(|e: std::io::Error| e.to_string())?;

            for file_result in entries {
                let mut f = file_result.map_err(|e: std::io::Error| e.to_string())?;
                let path = f.path().map_err(|e: std::io::Error| e.to_string())?.to_path_buf();

                f.unpack_in(dest_root).map_err(|e: std::io::Error| e.to_string())?;
                installed_files.push(path.to_string_lossy().to_string());
            }
        }
    }
    Ok(installed_files)
}

pub fn select_binary_for_arch<'a>(pkg: &'a Package, arch: &str) -> Result<&'a BinaryPackage, String> {
    let target_arch = match arch {
        "native" => match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => other,
        },
        other => other,
    };
    pkg.binaries
        .iter()
        .find(|b| b.arch == target_arch || b.arch == "all" || b.arch == "x86_64")
        .ok_or_else(|| format!("No binary package for architecture {}", arch))
}

pub fn find_package_file(pkg_name: &str) -> Option<PathBuf> {
    let repo = Path::new(REPO_ROOT);
    let read_dir = fs::read_dir(repo).ok()?;

    for entry in read_dir.flatten() {
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

pub fn install_package(pkg_name: &str, cfg: &config::Config) {
    match install_package_with_result(pkg_name, cfg) {
        Ok(installed_pkg) => {
            let mut db = InstalledDB::load();
            db.add_package(installed_pkg);
            if let Err(e) = db.save() {
                eprintln!("{} Failed to save installed DB: {}", "[ror]".red().bold(), e);
            } else {
                println!("{} Package registered in database", "[ror]".green());
            }
            println!("{} {} Package installed successfully.", "[ror]".blue().bold(), ">>>".green());
        }
        Err(e) => {
            eprintln!("{} {}", "[ror]".red().bold(), e);
        }
    }
}

pub fn install_package_with_result(pkg_name: &str, cfg: &config::Config) -> Result<InstalledPackage, String> {
    let mut spinner = Spinner::new(&format!("Resolving {}", pkg_name));
    spinner.tick(&format!("Resolving {}", pkg_name));

    let pkg_path = find_package_file(pkg_name)
        .ok_or_else(|| format!("Package '{}' not found", pkg_name))?;

    let content = fs::read_to_string(&pkg_path)
        .map_err(|e| format!("Failed to read package file: {}", e))?;

    let pkg: Package = serde_yaml::from_str(&content)
        .map_err(|e| format!("YAML structure error: {}", e))?;

    if pkg.binaries.is_empty() {
        return Err("No binary packages defined".to_string());
    }

    let binary = select_binary_for_arch(&pkg, "native")?;
    let db = InstalledDB::load();

    for conflict in &pkg.conflicts {
        if db.is_installed(conflict) {
            return Err(format!(
                "Package '{}' conflicts with installed package '{}'",
                pkg_name, conflict
            ));
        }
    }

    let mut urls_to_try: Vec<String> = if !binary.url.is_empty() {
        vec![binary.url.clone()]
    } else if !binary.mirrors.is_empty() {
        binary
            .mirrors
            .iter()
            .map(|m| m.replace("{filename}", &binary.filename))
            .collect()
    } else {
        return Err(format!("No URL or mirrors provided for package {}", pkg_name));
    };

    if !cfg.global.ignore_speed && urls_to_try.len() > 1 {
        spinner.tick("Testing mirror speeds...");
        urls_to_try = sort_mirrors_by_speed(&urls_to_try);
        spinner.finish(&format!("Fastest mirror selected: {}", urls_to_try[0]));
    }

    let mut last_err = None;
    let mut all_installed_files = Vec::new();
    let mut downloaded = false;

    for url in urls_to_try {
        spinner.tick(&format!("Downloading {} ...", url.split('/').last().unwrap_or(&url)));

        let archive_path = match download_and_verify(&url, &binary.sha256, cfg) {
            Ok(p) => p,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };

        spinner.finish(&format!("Downloaded {}", pkg_name));

        let work_dir = Path::new("/tmp/ror-install").join(&pkg.name);
        let _ = fs::remove_dir_all(&work_dir);
        fs::create_dir_all(&work_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let pkg_type = binary.pkg_type.as_deref().unwrap_or("tar.xz");

        let mut sp2 = Spinner::new(&format!("Extracting {}", pkg_name));
        sp2.tick(&format!("Extracting {}...", pkg_name));

        if pkg_type == "deb" {
            let mut files = extract_deb(&archive_path, Path::new("/"))?;
            all_installed_files.append(&mut files);
        } else {
            let files = extract_native(&archive_path, &work_dir)
                .map_err(|e| format!("Extraction failed: {}", e))?;
            let file_list = if binary.files.is_empty() { files } else { binary.files.clone() };
            sp2.tick(&format!("Installing files for {}...", pkg_name));
            let mut installed = install_files(&work_dir, &binary.install_prefix, &file_list)
                .map_err(|e| format!("Failed to install files: {}", e))?;
            all_installed_files.append(&mut installed);
        }

        sp2.finish(&format!("Installed {} files", all_installed_files.len()));

        let _ = fs::remove_dir_all(&work_dir);
        let _ = fs::remove_file(&archive_path);
        downloaded = true;
        break;
    }

    if !downloaded {
        return Err(last_err.unwrap_or_else(|| "All mirrors/URLs failed".to_string()));
    }

    if !pkg.install_steps.is_empty() {
        let mut sp3 = Spinner::new(&format!("Running install steps for {}", pkg_name));
        sp3.tick(&format!("Running install steps for {}...", pkg_name));
        run_commands(&pkg.install_steps, Path::new("/"))?;
        sp3.finish("Install steps done");
    }

    Ok(InstalledPackage {
        name: pkg.name.clone(),
        version: pkg.version.clone(),
        build_type: "binary".to_string(),
        files: all_installed_files,
        installed_at: Local::now().to_rfc3339(),
    })
}

pub fn download_and_verify(url: &str, expected_sha: &str, _cfg: &config::Config) -> Result<PathBuf, String> {
    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let total_size = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let tmp_dir = Path::new("/tmp/ror-download");
    fs::create_dir_all(tmp_dir).map_err(|e| format!("Failed to create tmp dir: {}", e))?;

    let file_name = url.split('/').last().unwrap_or("archive.tmp");
    let file_path = tmp_dir.join(file_name);

    {
        use std::io::Write;
        let mut file = fs::File::create(&file_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        let mut hasher = Sha256::new();
        let mut reader = response;
        let mut buf = [0u8; 65536];
        let mut downloaded: u64 = 0;

        loop {
            use std::io::Read;
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| format!("Failed to write to temp file: {}", e))?;
            downloaded += n as u64;

            match total_size {
                Some(total) if total > 0 => {
                    let pct = downloaded * 40 / total;
                    let bar = format!(
                        "{}>{}", 
                        "=".repeat(pct.saturating_sub(1) as usize),
                        " ".repeat((40 - pct) as usize)
                    );
                    eprint!(
                        "\r\x1b[K\x1b[36m[{}]\x1b[0m {} / {}",
                        bar,
                        format_bytes(downloaded),
                        format_bytes(total)
                    );
                }
                _ => {
                    eprint!("\r\x1b[K{} downloaded", format_bytes(downloaded));
                }
            }
            use std::io::stderr;
            stderr().flush().ok();
        }

        eprintln!("\r\x1b[K\x1b[32m✓\x1b[0m Downloaded {}", format_bytes(downloaded));

        if !expected_sha.is_empty() {
            let hash = format!("{:x}", hasher.finalize());
            if hash != expected_sha {
                let _ = fs::remove_file(&file_path);
                return Err(format!(
                    "SHA256 mismatch: expected {}, got {}",
                    expected_sha, hash
                ));
            }
        }
    }

    Ok(file_path)
}


pub fn extract_native(archive_path: &Path, target_dir: &Path) -> Result<Vec<String>, String> {
    let status = Command::new("tar")
        .arg("-xf")
        .arg(archive_path)
        .arg("-C")
        .arg(target_dir)
        .status()
        .map_err(|e| format!("Failed to execute tar: {}", e))?;

    if !status.success() {
        return Err(format!("tar failed with exit code {:?}", status.code()));
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(target_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() || entry.file_type().is_symlink() {
            let rel_path = entry.path().strip_prefix(target_dir).unwrap();
            files.push(rel_path.to_string_lossy().to_string());
        }
    }
    Ok(files)
}

fn install_files(source_dir: &Path, prefix: &str, files: &[String]) -> Result<Vec<String>, String> {
    let prefix_path = Path::new(prefix);
    let mut installed = Vec::new();

    for rel_path in files {
        let src = source_dir.join(rel_path);
        let dst = prefix_path.join(rel_path);

        let meta = fs::symlink_metadata(&src)
            .map_err(|_| format!("File {} not found in package", rel_path))?;

        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir {}: {}", parent.display(), e))?;
        }

        if meta.is_symlink() {
            let target = fs::read_link(&src)
                .map_err(|e| format!("Failed to read link {}: {}", src.display(), e))?;
            let _ = fs::remove_file(&dst);
            symlink(&target, &dst)
                .map_err(|e| format!("Failed to create symlink {}: {}", dst.display(), e))?;
        } else {
            fs::copy(&src, &dst)
                .map_err(|e| format!("Failed to copy {}: {}", src.display(), e))?;
        }
        installed.push(rel_path.clone());
    }
    Ok(installed)
}

pub fn install_files_with_root(source_dir: &Path, root: &Path, files: &[String]) -> Result<Vec<String>, String> {
    let mut installed = Vec::new();
    for rel_path in files {
        let src = source_dir.join(rel_path);
        let dst = root.join(rel_path);

        let meta = fs::symlink_metadata(&src)
            .map_err(|_| format!("File {} not found in package", rel_path))?;

        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir {}: {}", parent.display(), e))?;
        }

        if meta.is_symlink() {
            let target = fs::read_link(&src)
                .map_err(|e| format!("Failed to read link {}: {}", src.display(), e))?;
            let _ = fs::remove_file(&dst);
            symlink(&target, &dst)
                .map_err(|e| format!("Failed to create symlink {}: {}", dst.display(), e))?;
        } else {
            fs::copy(&src, &dst)
                .map_err(|e| format!("Failed to copy {}: {}", src.display(), e))?;
        }
        installed.push(rel_path.clone());
    }
    Ok(installed)
}

pub fn run_commands(steps: &str, work_dir: &Path) -> Result<(), String> {
    if steps.trim().is_empty() {
        return Ok(());
    }
    let status = Command::new("sh")
        .current_dir(work_dir)
        .arg("-c")
        .arg(steps)
        .status()
        .map_err(|e| format!("Failed to execute 'sh -c': {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Command failed with exit code {:?}", status.code()))
    }
}

pub fn get_package_info(pkg_name: &str) -> Option<(String, String)> {
    let pkg_path = find_package_file(pkg_name)?;
    let content = fs::read_to_string(pkg_path).ok()?;
    let name = pkg_name.to_string();
    let version = content
        .lines()
        .find(|l| l.starts_with("version:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    Some((name, version))
}

pub fn load_package(pkg_name: &str) -> Option<Package> {
    let pkg_path = find_package_file(pkg_name)?;
    let content = fs::read_to_string(pkg_path).ok()?;
    serde_yaml::from_str(&content).ok()
}
