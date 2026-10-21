//! Options and recent destinations, kept in %LOCALAPPDATA%\SuperFastCopy\settings.txt.

use superfastcopy::{shell, Existing};

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub existing: Existing,
    pub auto_threads: bool,
    pub threads: usize,
    pub contents_only: bool,
    pub native: bool,
    /// Close the paste window automatically after a clean finish.
    pub auto_close: bool,
    pub recent: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            existing: Existing::Overwrite,
            auto_threads: true,
            threads: 16,
            contents_only: false,
            native: false,
            auto_close: true,
            recent: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Ok(text) = std::fs::read_to_string(shell::data_dir().join("settings.txt")) else {
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let on = v == "1";
            match k {
                "existing" => {
                    s.existing = match v {
                        "skip-same" => Existing::SkipSame,
                        "skip" => Existing::SkipExisting,
                        _ => Existing::Overwrite,
                    }
                }
                "auto_threads" => s.auto_threads = on,
                "threads" => s.threads = v.parse().unwrap_or(16).clamp(1, 256),
                "contents_only" => s.contents_only = on,
                "native" => s.native = on,
                "auto_close" => s.auto_close = on,
                "recent" if !v.is_empty() => s.recent.push(v.to_string()),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let b = |v: bool| if v { "1" } else { "0" };
        let mut text = format!(
            "existing={}\nauto_threads={}\nthreads={}\ncontents_only={}\nnative={}\nauto_close={}\n",
            match self.existing {
                Existing::Overwrite => "overwrite",
                Existing::SkipSame => "skip-same",
                Existing::SkipExisting => "skip",
            },
            b(self.auto_threads),
            self.threads,
            b(self.contents_only),
            b(self.native),
            b(self.auto_close),
        );
        for r in &self.recent {
            text.push_str(&format!("recent={r}\n"));
        }
        let _ = std::fs::create_dir_all(shell::data_dir());
        let _ = std::fs::write(shell::data_dir().join("settings.txt"), text);
    }

    pub fn remember(&mut self, dest: &str) {
        self.recent.retain(|r| !r.eq_ignore_ascii_case(dest));
        self.recent.insert(0, dest.to_string());
        self.recent.truncate(6);
    }
}
