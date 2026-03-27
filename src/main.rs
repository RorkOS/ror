use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use colored::Colorize;

mod sync;
mod install;
mod config;
mod info;
mod group;
mod delete;
mod listinstalled;
mod update;
mod rootfs;
mod parallel;
mod debug;
mod search;
mod repo;
mod progress;

#[derive(Parser, Debug)]
#[command(author, about, arg_required_else_help = true)]
struct Args {
    #[arg(long = "search", value_name = "QUERY")]
    search: Option<String>,

    #[arg(short = 's', long = "sync")]
    sync: bool,

    #[arg(short = 'd', long = "delete", value_name = "PACKAGE")]
    delete: Option<String>,

    #[arg(short = 'l', long = "list-installed")]
    listinstalled: bool,

    #[arg(long = "info", value_name = "PACKAGE")]
    info: Option<String>,

    #[arg(short = 'v', long = "version")]
    version: bool,

    #[arg(short = 'g', long = "gen-config")]
    gen_config: bool,

    #[arg(short = 'i', long = "install", value_name = "PACKAGE_OR_GROUP", num_args = 1..)]
    install: Vec<String>,

    #[arg(long = "update", value_name = "PACKAGE")]
    update: Option<String>,

    #[arg(long = "dry-run")]
    dry_run: bool,

    #[arg(short = 'u', long = "upgrade")]
    upgrade: bool,

    #[arg(long = "repo-add", value_names = ["NAME", "URL"])]
    repo_add: Option<Vec<String>>,

    #[arg(long = "repo-remove", value_name = "NAME")]
    repo_remove: Option<String>,

    #[arg(long = "repo-list")]
    repo_list: bool,

    #[arg(long = "build-rootfs")]
    build_rootfs: bool,

    #[arg(long = "rootfs-group", value_name = "GROUP")]
    rootfs_group: Option<String>,

    #[arg(long = "rootfs-target", value_name = "TARGET")]
    rootfs_target: Option<PathBuf>,

    #[arg(long = "rootfs-arch", default_value = "native")]
    rootfs_arch: String,

    #[arg(long = "di", value_names = ["DELETE", "INSTALL"], num_args = 2)]
    delete_install: Option<Vec<String>>,
    
    #[arg(long = "lg", alias = "listgroups")]
    listgroups: bool,
}

fn main() {
    let args = Args::parse();

    if args.version {
        println!("ror {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.gen_config {
        let config_path = Path::new("/var/ror/ror.conf");
        match config::Config::create_default_config(config_path) {
            Ok(()) => println!(
                "{} Default config created at {:?}",
                "[ror]".blue().bold(),
                config_path
            ),
            Err(e) => eprintln!(
                "{} Failed to create config: {}",
                "[ror]".red().bold(),
                e
            ),
        }
        return;
    }

    let cfg = Arc::new(config::Config::load());

    if let Some(di_args) = args.delete_install {
        let del_pkg = di_args[0].clone();
        let inst_pkg = di_args[1].clone();

        let db = install::InstalledDB::load();

        if !db.is_installed(&del_pkg) {
            eprintln!("{} Package '{}' is not installed.", "[ror]".red().bold(), del_pkg);
            return;
        }

        if let Err(e) = delete::check_critical_removal_with_replacement(&del_pkg, &inst_pkg, &db) {
            eprintln!("{}", e);
            return;
        }

        if let Err(e) = delete::check_critical_install(&inst_pkg, &db) {
            let already_installed_by_same = db.packages.get(&inst_pkg).is_some();
            if !already_installed_by_same {
                let conflict_is_the_deleted = e.contains(&del_pkg);
                if !conflict_is_the_deleted {
                    eprintln!("{}", e);
                    return;
                }
            }
        }

        let record_files = db.packages
            .get(&del_pkg)
            .map(|r| r.files.clone())
            .unwrap_or_default();

        println!(
            "{} Replacing '{}' with '{}'...",
            "[ror]".blue().bold(),
            del_pkg,
            inst_pkg
        );

        let del_pkg_for_thread = del_pkg.clone();
        let inst_pkg_for_thread = inst_pkg.clone();
        let cfg_for_thread = Arc::clone(&cfg);

        let del_handle = thread::spawn(move || {
            println!("{} [remove] Starting removal of '{}'", ">>>".yellow(), del_pkg_for_thread);
            let ok = delete::remove_package_files_only(&del_pkg_for_thread, &record_files);
            if ok {
                println!("{} [remove] Files removed for '{}'", ">>>".green(), del_pkg_for_thread);
            } else {
                eprintln!("{} [remove] Errors during removal of '{}'", "[ror]".red().bold(), del_pkg_for_thread);
            }
            ok
        });

        let inst_handle = thread::spawn(move || {
            println!("{} [install] Starting install of '{}'", ">>>".cyan(), inst_pkg_for_thread);
            install::install_package_with_result(&inst_pkg_for_thread, &cfg_for_thread)
        });

        let del_ok = del_handle.join().unwrap_or(false);
        let inst_result = inst_handle.join().unwrap_or_else(|_| Err("install thread panicked".to_string()));

        let mut db = install::InstalledDB::load();

        if del_ok {
            db.packages.remove(&del_pkg);
            println!("{} '{}' removed from database", "[ror]".green(), del_pkg);
        }

        match inst_result {
            Ok(installed_pkg) => {
                db.add_package(installed_pkg);
                println!("{} '{}' added to database", "[ror]".green(), inst_pkg);
            }
            Err(e) => {
                eprintln!("{} Failed to install '{}': {}", "[ror]".red().bold(), inst_pkg, e);
            }
        }

        if let Err(e) = db.save() {
            eprintln!("{} Failed to save installed DB: {}", "[ror]".red().bold(), e);
        } else {
            println!(
                "{} Replace operation finished: '{}' -> '{}'",
                "[ror]".green().bold(),
                del_pkg,
                inst_pkg
            );
        }

        return;
    }

    if let Some(pkg) = args.info {
        info::print_package_info(&pkg);
        return;
    }

    if args.sync {
        sync::run_sync(&cfg);
        return;
    }

    if args.listinstalled {
        listinstalled::print_installed();
        return;
    }

    if args.build_rootfs {
        let arch = if args.rootfs_arch == "native" {
            "native"
        } else {
            &args.rootfs_arch
        };
        if let (Some(group), Some(target)) = (args.rootfs_group, args.rootfs_target) {
            if let Err(e) = rootfs::build_rootfs(&group, &target, &cfg, arch, true) {
                eprintln!("{} {}", "[ror]".red().bold(), e);
            }
        } else {
            eprintln!("{} --build-rootfs requires --rootfs-group and --rootfs-target", "[ror]".red().bold());
        }
        return;
    }

    if let Some(pkg) = args.delete {
        delete::remove_package(&pkg);
        return;
    }

    if let Some(query) = args.search {
        search::search_packages(&query);
        return;
    }

    if !args.install.is_empty() {
        let db = install::InstalledDB::load();
        for pkg_name in &args.install {
            if pkg_name.starts_with('@') {
                continue;
            }
            if let Err(e) = delete::check_critical_install(pkg_name, &db) {
                eprintln!("{}", e);
                return;
            }
        }

        if args.install[0].starts_with('@') {
            let group_name = &args.install[0][1..];
            match group::load_group(group_name) {
                Ok(group) => {
                    if let Err(e) = parallel::install_packages_parallel(&group.packages, Arc::clone(&cfg)) {
                        eprintln!("{} {}", "[ror]".red().bold(), e);
                    }
                }
                Err(e) => eprintln!("{} {}", "[ror]".red().bold(), e),
            }
        } else {
            if let Err(e) = parallel::install_packages_parallel(&args.install, Arc::clone(&cfg)) {
                eprintln!("{} {}", "[ror]".red().bold(), e);
            }
        }
        return;
    }

    if args.upgrade {
        if args.dry_run {
            let upgradable = update::list_upgradable();
            if upgradable.is_empty() {
                println!("{} No packages to upgrade.", "[ror]".green());
            } else {
                println!("{} Packages that would be upgraded:", ">>>".yellow());
                for pkg in upgradable {
                    println!("  - {}", pkg);
                }
            }
        } else {
            update::upgrade_all(&cfg, false);
        }
        return;
    }

    if args.repo_list {
        repo::list_repositories();
        return;
    }

    if let Some(repo_args) = args.repo_remove {
        if let Err(e) = repo::remove_repository(&repo_args) {
            eprintln!("{} {}", "[ror]".red().bold(), e);
        }
        return;
    }

    if let Some(repo_args) = args.repo_add {
        if repo_args.len() < 2 {
            eprintln!("{} Usage: --repo-add <name> <url> [mirror]", "[ror]".red().bold());
            return;
        }
        let name = &repo_args[0];
        let url = &repo_args[1];
        let mirror = if repo_args.len() > 2 { Some(repo_args[2].as_str()) } else { None };
        if let Err(e) = repo::add_repository(name, url, mirror) {
            eprintln!("{} {}", "[ror]".red().bold(), e);
        }
        return;
    }

    if let Some(pkg) = args.update {
        update::update_package(&pkg, &cfg, args.dry_run);
        return;
    }

    if args.listgroups {
        group::list_groups();
        return;
    }
}
