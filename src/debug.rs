use std::env;

pub fn debug(msg: &str) {
    if env::var("ROR_DEBUG").unwrap_or_default() == "1" {
        eprintln!("[DEBUG] {}", msg);
    }
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::debug::debug(&format!($($arg)*));
    };
}
