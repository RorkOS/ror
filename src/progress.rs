use std::io::{self, Write};

pub struct ProgressBar {
    total: usize,
    current: usize,
    message: String,
    width: usize,
}

impl ProgressBar {
    pub fn new(total: usize, message: &str) -> Self {
        Self {
            total,
            current: 0,
            message: message.to_string(),
            width: 38,
        }
    }


    pub fn inc(&mut self, msg: &str) {
        self.current += 1;
        self.message = msg.to_string();
        self.render();
    }

    fn render(&self) {
        let filled = if self.total == 0 {
            0
        } else {
            self.current * self.width / self.total
        };
        let arrow = if filled < self.width { ">" } else { "=" };
        let bar = format!(
            "{}{}{}",
            "=".repeat(filled.saturating_sub(1)),
            arrow,
            " ".repeat(self.width.saturating_sub(filled))
        );
        eprint!(
            "\r\x1b[K\x1b[36m[{}]\x1b[0m \x1b[32m{}/{}\x1b[0m  {}",
            bar,
            self.current,
            self.total,
            self.message
        );
        io::stderr().flush().ok();
    }

    pub fn finish(&self, msg: &str) {
        let bar = "=".repeat(self.width);
        eprintln!(
            "\r\x1b[K\x1b[36m[{}]\x1b[0m \x1b[32m{}/{}\x1b[0m  {}",
            bar, self.total, self.total, msg
        );
    }
}

pub struct Spinner {
    frames: &'static [&'static str],
    idx: usize,
    message: String,
}

impl Spinner {
    pub fn new(message: &str) -> Self {
        Self {
            frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            idx: 0,
            message: message.to_string(),
        }
    }

    pub fn tick(&mut self, msg: &str) {
        self.message = msg.to_string();
        eprint!("\r\x1b[K\x1b[36m{}\x1b[0m {}", self.frames[self.idx], self.message);
        io::stderr().flush().ok();
        self.idx = (self.idx + 1) % self.frames.len();
    }

    pub fn finish(&self, msg: &str) {
        eprintln!("\r\x1b[K\x1b[32m✓\x1b[0m {}", msg);
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
