//! Settings come from environment variables, or from a `.env` file next to
//! where the app is started (handy on Windows).

use std::collections::HashMap;

pub struct Config(HashMap<String, String>);

impl Config {
    pub fn load() -> Self {
        let mut map = HashMap::new();
        if let Ok(raw) = std::fs::read_to_string(".env") {
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    map.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
                }
            }
        }
        Config(map)
    }

    pub fn get(&self, key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .or_else(|| self.0.get(key).cloned())
            .filter(|v| !v.is_empty())
    }

    /// Same as `get`, but the `.env` file wins over process env vars. Vision
    /// uses this so a value edited in `web/.env` (directly, or by
    /// `main.py --configure`) beats the copy the container was launched with.
    pub fn get_file_first(&self, key: &str) -> Option<String> {
        self.0
            .get(key)
            .cloned()
            .or_else(|| std::env::var(key).ok())
            .filter(|v| !v.is_empty())
    }
}
