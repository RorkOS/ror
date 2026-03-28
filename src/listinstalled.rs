use colored::Colorize;
use crate::install::InstalledDB;

pub fn print_installed() {
    let db = InstalledDB::load();
    let packages = db.packages;

    if packages.is_empty() {
        println!("{} No packages installed.", "[ror]".yellow());
        return;
    }

    println!("{} Installed packages:", "[ror]".blue().bold());
    println!("{:-<45}", "");
    println!("{:<20} {:<15}", "Name", "Version");
    println!("{:-<45}", "");

    let mut sorted: Vec<_> = packages.values().collect();
    sorted.sort_by_key(|p| &p.name);

    for pkg in sorted {
        println!(
            "{:<20} {:<15}",
            pkg.name.green(),
            pkg.version.cyan(),
        );
    }
    println!("{:-<45}", "");
    println!("{} Total: {} packages", ">>>".green(), packages.len());
}
