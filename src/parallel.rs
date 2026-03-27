use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread;
use crate::config::Config;
use crate::install::{self, Dependency, InstalledDB, load_package};
use colored::Colorize;
use crate::debug;

type Result<T> = std::result::Result<T, String>;

struct DependencyGraph {
    edges: HashMap<String, Vec<String>>,
    reverse: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    fn new() -> Self {
        debug!("DependencyGraph::new()");
        Self { edges: HashMap::new(), reverse: HashMap::new() }
    }

    fn add_dep(&mut self, pkg: String, dep: String) {
        debug!("DependencyGraph::add_dep: {} -> {}", pkg, dep);
        self.edges.entry(pkg.clone()).or_insert_with(Vec::new).push(dep.clone());
        self.reverse.entry(dep).or_insert_with(Vec::new).push(pkg);
    }

    fn compute_levels(&self) -> Vec<Vec<String>> {
        debug!("DependencyGraph::compute_levels: start");
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for (pkg, deps) in &self.edges {
            in_degree.entry(pkg.clone()).or_insert(0);
            for dep in deps {
                *in_degree.entry(dep.clone()).or_insert(0) += 1;
            }
        }
        for (pkg, children) in &self.reverse {
            in_degree.entry(pkg.clone()).or_insert(0);
            for child in children {
                in_degree.entry(child.clone()).or_insert(0);
            }
        }
        let mut levels = Vec::new();
        let mut queue: VecDeque<String> = in_degree.iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(pkg, _)| pkg.clone())
            .collect();
        debug!("compute_levels: initial queue: {:?}", queue);
        while !queue.is_empty() {
            let level_pkgs: Vec<String> = queue.drain(..).collect();
            debug!("compute_levels: level {}: {:?}", levels.len(), level_pkgs);
            levels.push(level_pkgs.clone());
            for pkg in &level_pkgs {
                if let Some(children) = self.reverse.get(pkg) {
                    for child in children {
                        if let Some(deg) = in_degree.get_mut(child) {
                            *deg -= 1;
                            if *deg == 0 {
                                queue.push_back(child.clone());
                            }
                        }
                    }
                }
            }
        }
        let cyclic: Vec<_> = in_degree.iter()
            .filter(|&(_, &d)| d > 0)
            .map(|(name, _)| name.as_str())
            .collect();
        if !cyclic.is_empty() {
            eprintln!(
                "{} Warning: cycle detected among packages, they will be installed in arbitrary order",
                "[ror]".yellow().bold()
            );
            debug!("compute_levels: circular dependency among: {:?}", cyclic);
        }
        debug!("compute_levels: levels count = {}", levels.len());
        levels
    }
}

fn collect_required_packages(packages: &[String], installed_db: &InstalledDB) -> Result<HashSet<String>> {
    debug!("collect_required_packages: start with packages {:?}", packages);
    let mut required = HashSet::new();
    let mut stack: Vec<String> = packages.iter().cloned().collect();
    while let Some(pkg_name) = stack.pop() {
        debug!("collect_required_packages: processing {}", pkg_name);
        if installed_db.is_installed(&pkg_name) || required.contains(&pkg_name) {
            debug!("collect_required_packages: {} already installed or in required set", pkg_name);
            continue;
        }
        required.insert(pkg_name.clone());
        let pkg = match load_package(&pkg_name) {
            Some(p) => p,
            None => {
                debug!("collect_required_packages: package {} not found", pkg_name);
                return Err(format!("Package '{}' not found", pkg_name));
            }
        };
        for dep in &pkg.depends {
            match dep {
                Dependency::Single(name) => {
                    debug!("collect_required_packages: single dependency {}", name);
                    if !installed_db.is_installed(name) && !required.contains(name) {
                        stack.push(name.clone());
                    }
                }
                Dependency::Any(alternatives) => {
                    debug!("collect_required_packages: any dependency {:?}", alternatives);

                    let installed_alt = alternatives.iter().find(|alt| installed_db.is_installed(*alt));
                    if installed_alt.is_some() {
                        debug!("collect_required_packages: an alternative already installed, skipping");
                        continue;
                    }

                    if let Some(primary_dep) = alternatives.first().cloned() {
                        debug!("collect_required_packages: chosen primary alternative {}", primary_dep);
                        if !required.contains(&primary_dep) {
                            stack.push(primary_dep);
                        }
                    } else {
                        return Err(format!("Empty alternative list in package {}", pkg_name));
                    }
                }
            }
        }
    }
    debug!("collect_required_packages: required set = {:?}", required);
    Ok(required)
}

fn build_graph(packages: &HashSet<String>, installed_db: &InstalledDB) -> Result<DependencyGraph> {
    debug!("build_graph: building graph for {} packages", packages.len());
    let mut graph = DependencyGraph::new();
    for pkg_name in packages {
        debug!("build_graph: processing {}", pkg_name);
        let pkg = match load_package(pkg_name) {
            Some(p) => p,
            None => {
                debug!("build_graph: package {} not found", pkg_name);
                return Err(format!("Package '{}' not found", pkg_name));
            }
        };
        for dep in &pkg.depends {
            match dep {
                Dependency::Single(name) => {
                    if packages.contains(name) {
                        debug!("build_graph: adding edge {} -> {}", pkg_name, name);
                        graph.add_dep(pkg_name.clone(), name.clone());
                    } else if !installed_db.is_installed(name) {
                        debug!("build_graph: missing dependency {} for {}", name, pkg_name);
                        return Err(format!("Dependency '{}' of '{}' is missing", name, pkg_name));
                    }
                }
                Dependency::Any(alternatives) => {
                    debug!("build_graph: any dependency {:?} for {}", alternatives, pkg_name);
                    let found = alternatives.iter().find(|alt| packages.contains(*alt));
                    if let Some(chosen) = found {
                        debug!("build_graph: adding edge {} -> {}", pkg_name, chosen);
                        graph.add_dep(pkg_name.clone(), chosen.clone());
                    } else {
                        let installed = alternatives.iter().any(|alt| installed_db.is_installed(alt));
                        if !installed {
                            debug!("build_graph: no alternative available for {}", pkg_name);
                            return Err(format!("No alternative from {:?} for package '{}' is available", alternatives, pkg_name));
                        }
                    }
                }
            }
        }
    }
    Ok(graph)
}

fn check_conflicts(required: &HashSet<String>, installed_db: &InstalledDB) -> Result<()> {
    debug!("check_conflicts: checking {} packages", required.len());
    let mut pkg_map = HashMap::new();
    for pkg_name in required {
        let pkg = match load_package(pkg_name) {
            Some(p) => p,
            None => {
                debug!("check_conflicts: package {} not found", pkg_name);
                return Err(format!("Package '{}' not found", pkg_name));
            }
        };
        pkg_map.insert(pkg_name.clone(), pkg);
    }
    for (name, pkg) in &pkg_map {
        for conflict in &pkg.conflicts {
            if installed_db.is_installed(conflict) {
                debug!("check_conflicts: {} conflicts with installed {}", name, conflict);
                return Err(format!("Package '{}' conflicts with installed package '{}'", name, conflict));
            }
        }
    }
    let names: Vec<_> = pkg_map.keys().cloned().collect();
    for i in 0..names.len() {
        for j in i+1..names.len() {
            let a = &names[i];
            let b = &names[j];
            let pkg_a = &pkg_map[a];
            let pkg_b = &pkg_map[b];
            if pkg_a.conflicts.contains(b) || pkg_b.conflicts.contains(a) {
                debug!("check_conflicts: {} and {} conflict", a, b);
                return Err(format!("Packages '{}' and '{}' conflict with each other", a, b));
            }
        }
    }
    debug!("check_conflicts: no conflicts found");
    Ok(())
}

fn install_single_package(pkg_name: &str, cfg: Arc<Config>, db_mutex: &Mutex<InstalledDB>) -> Result<()> {
    debug!("install_single_package: starting {}", pkg_name);
    println!("{} Installing {}...", ">>>".cyan(), pkg_name);
    let installed_pkg = match install::install_package_with_result(pkg_name, &*cfg) {
        Ok(p) => {
            debug!("install_single_package: install_package_with_result OK for {}", pkg_name);
            p
        }
        Err(e) => {
            debug!("install_single_package: install_package_with_result failed for {}: {}", pkg_name, e);
            return Err(e);
        }
    };
    let mut db = db_mutex.lock().unwrap();
    db.add_package(installed_pkg);
    debug!("install_single_package: added to memory DB, saving...");
    if let Err(e) = db.save() {
        eprintln!("{} Failed to save installed DB: {}", "[ror]".red().bold(), e);
        debug!("install_single_package: save failed: {}", e);
        return Err(format!("Failed to save installed DB: {}", e));
    }
    debug!("install_single_package: save successful for {}", pkg_name);
    println!("{} Package registered in database", "[ror]".green());
    Ok(())
}

pub fn install_packages_parallel(packages: &[String], cfg: Arc<Config>) -> Result<()> {
    debug!("install_packages_parallel: called with {:?}", packages);
    let installed_db = InstalledDB::load();
    let required = collect_required_packages(packages, &installed_db)?;
    if required.is_empty() {
        println!("{} All packages already installed.", "[ror]".green());
        debug!("install_packages_parallel: no packages required");
        return Ok(());
    }
    check_conflicts(&required, &installed_db)?;
    let graph = build_graph(&required, &installed_db)?;
    let mut levels = graph.compute_levels();
    if levels.is_empty() && !required.is_empty() {
        debug!("install_packages_parallel: no dependencies, creating single level");
        levels.push(required.iter().cloned().collect());
    }
    debug!("install_packages_parallel: levels: {:?}", levels);
    let total_pkgs: usize = levels.iter().map(|l| l.len()).sum();
    let mut installed_count = 0usize;
    println!(
        "{} Installing {} package(s) across {} level(s)",
        "[ror]".blue().bold(),
        total_pkgs,
        levels.len()
    );
    let db_mutex = Arc::new(Mutex::new(InstalledDB::load()));
    for (level_idx, level_pkgs) in levels.into_iter().enumerate() {
        println!(
            "{} Level {}: {} package(s)",
            ">>>".cyan(),
            level_idx,
            level_pkgs.len()
        );
        debug!("install_packages_parallel: starting level {} with {:?}", level_idx, level_pkgs);
        let level_total = level_pkgs.len();
        let mut handles = Vec::new();
        for pkg in level_pkgs {
            let pkg_name = pkg.clone();
            let cfg = Arc::clone(&cfg);
            let db_mutex = Arc::clone(&db_mutex);
            let handle = thread::spawn(move || {
                install_single_package(&pkg_name, cfg, &db_mutex)
            });
            handles.push((pkg, handle));
        }
        let mut errors = Vec::new();
        let mut level_done = 0usize;
        for (pkg_name, handle) in handles {
            match handle.join() {
                Ok(Ok(())) => {
                    installed_count += 1;
                    level_done += 1;
                    eprint!(
                        "\r\x1b[K\x1b[36m[{:>3}/{:<3}]\x1b[0m  level {}, total {}/{}  {}",
                        level_done, level_total,
                        level_idx,
                        installed_count, total_pkgs,
                        pkg_name
                    );
                    use std::io::Write;
                    std::io::stderr().flush().ok();
                }
                Ok(Err(e)) => {
                    debug!("install_packages_parallel: thread error: {}", e);
                    errors.push(e);
                }
                Err(_) => {
                    debug!("install_packages_parallel: thread panicked");
                    errors.push("Thread panicked".to_string());
                }
            }
        }
        eprintln!();
        if !errors.is_empty() {
            eprintln!("{} Errors during level {}: {:?}", "[ror]".red().bold(), level_idx, errors);
            debug!("install_packages_parallel: level {} failed with {} errors", level_idx, errors.len());
            return Err(format!("Installation failed at level {}", level_idx));
        }
    }
    println!("{} All packages installed successfully.", "[ror]".green().bold());
    debug!("install_packages_parallel: completed successfully");
    Ok(())
}
