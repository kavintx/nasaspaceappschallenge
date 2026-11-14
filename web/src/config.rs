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
}
