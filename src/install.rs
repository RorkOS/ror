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
    pub const PATH: &'static str = "/var/ror/installed.json";

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
    pkg.binaries.iter().find(|b| b.arch == target_arch || b.arch == "all" || b.arch == "x86_64")
        .ok_or_else(|| format!("No binary package for architecture {}", arch)) 
}

pub fn find_package_file(pkg_name: &str) -> Option<PathBuf> {
    let repo = Path::new(REPO_ROOT);
    let read_dir = fs::read_dir(repo).ok()?;
    
    for entry in read_dir.flatten() {
        let cat_path = entry.path();
        if !cat_path.is_dir() { continue; }
        
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
            return Err(format!("Package '{}' conflicts with installed package '{}'", pkg_name, conflict));
        }
    }

    let urls_to_try: Vec<String> = if !binary.url.is_empty() {
        vec![binary.url.clone()]
    } else if !binary.mirrors.is_empty() {
        binary.mirrors.iter().map(|m| m.replace("{filename}", &binary.filename)).collect()
    } else {
        return Err(format!("No URL or mirrors provided for package {}", pkg_name));
    };

    let mut last_err = None;
    let mut all_installed_files = Vec::new();
    let mut downloaded = false;

    for url in urls_to_try {
        println!("{} Trying source: {}", ">>>".yellow(), url);

        let archive_path = match download_and_verify(&url, &binary.sha256, cfg) {
            Ok(p) => p,
            Err(e) => {
                last_err = Some(e);
                continue; 
            }
        };

        let work_dir = Path::new("/tmp/ror-install").join(&pkg.name);
        let _ = fs::remove_dir_all(&work_dir);
        fs::create_dir_all(&work_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

        let pkg_type = binary.pkg_type.as_deref().unwrap_or("tar.xz");

        if pkg_type == "deb" {
            println!("{} Processing external .deb for {}", ">>>".green(), pkg.name);
            let mut files = extract_deb(&archive_path, Path::new("/"))?;
            all_installed_files.append(&mut files);
        } else {
            let files = extract_native(&archive_path, &work_dir)
                .map_err(|e| format!("Extraction failed: {}", e))?;
            let file_list = if binary.files.is_empty() { files } else { binary.files.clone() };
            let mut installed = install_files(&work_dir, &binary.install_prefix, &file_list)
                .map_err(|e| format!("Failed to install files: {}", e))?;
            all_installed_files.append(&mut installed);
        }

        let _ = fs::remove_dir_all(&work_dir);
        let _ = fs::remove_file(&archive_path);
        downloaded = true;
        break; 
    }

    if !downloaded {
        return Err(last_err.unwrap_or_else(|| "All mirrors/URLs failed".to_string()));
    }

    if !pkg.install_steps.is_empty() {
        run_commands(&pkg.install_steps, Path::new("/"))?;
    }

    Ok(InstalledPackage {
        name: pkg.name.clone(),
        version: pkg.version.clone(),
        build_type: "binary".to_string(),
        files: all_installed_files,
        installed_at: Local::now().to_rfc3339(),
    })
}


pub fn download_and_verify(url: &str, expected_sha: &str, cfg: &config::Config) -> Result<PathBuf, String> {
    
    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client.get(url).send()
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

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
        loop {
            use std::io::Read;
            let n = reader.read(&mut buf)
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            if n == 0 { break; }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| format!("Failed to write to temp file: {}", e))?;
        }
        if !expected_sha.is_empty() {
            let hash = format!("{:x}", hasher.finalize());
            if hash != expected_sha {
                let _ = fs::remove_file(&file_path);
                return Err(format!("SHA256 mismatch: expected {}, got {}", expected_sha, hash));
            }
        }
    }

    if let Err(e) = verify_gpg_signature(&file_path, url, cfg.global.strict_gpg) {
        if cfg.global.strict_gpg {
            let _ = fs::remove_file(&file_path);
            return Err(format!("GPG ERROR: {}", e));
        } else {
            eprintln!("{} GPG warning: {}", "[ror]".yellow(), e);
        }
    }

    Ok(file_path)
}


fn verify_gpg_signature(file_path: &Path, original_url: &str, _strict_mode: bool) -> Result<(), String> {
    let sig_url = format!("{}.sig", original_url);
    let client = Client::builder().timeout(std::time::Duration::from_secs(10)).build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = match client.get(&sig_url).send() {
        Ok(r) => r,
        Err(_) => return Err("Failed to reach server for .sig file".to_string())
    };

    if !response.status().is_success() {
        return Err(format!("Signature file not found on server (HTTP {})", response.status()));
    }

    let sig_bytes = response.bytes()
        .map_err(|e| format!("Failed to download signature: {}", e))?.to_vec();

    let sig_path = file_path.with_extension("sig");
    fs::write(&sig_path, &sig_bytes).map_err(|e| format!("Failed to save signature: {}", e))?;

    println!("{} Verifying GPG signature...", ">>>".cyan());
    let status = Command::new("gpg")
        .arg("--verify")
        .arg(&sig_path)
        .arg(file_path)
        .status()
        .map_err(|e| format!("Failed to execute 'gpg' (is it installed?): {}", e))?;

    let _ = fs::remove_file(&sig_path);

    if status.success() {
        Ok(())
    } else {
        Err("GPG Signature mismatch! File might be tampered with.".to_string())
    }
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
    let version = content.lines()
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
