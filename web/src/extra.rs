//! Burn tests from sources outside the research database: published lab values such as
//! the limiting oxygen index, each with the page it was read from.
//!
//! This is an optional layer. It is off unless MORE_SOURCES=on, it never changes the NASA
//! verdicts, and its rows are shown in their own block under their own source names.
//! The rows live in data/more_sources.json, built into the program. A row applies to a
//! material only when the name (or one of the row's listed aliases) matches exactly.

use serde_json::Value;
use std::sync::OnceLock;

static RAW: &str = include_str!("../data/more_sources.json");

fn rows() -> &'static Vec<Value> {
    static ROWS: OnceLock<Vec<Value>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str::<Vec<Value>>(RAW).unwrap_or_default())
}

fn names_of(row: &Value) -> impl Iterator<Item = &str> {
    row["material"].as_str().into_iter().chain(row["aliases"].as_array().into_iter().flatten().filter_map(Value::as_str))
}

/// A name without a trailing word that only says what form it is in ("ABS plastic" is ABS).
fn bare(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    // Acrylic cloth is polyacrylonitrile. Plain "acrylic" is the sheet plastic (PMMA), a different material.
    if matches!(lower.as_str(), "acrylic fabric" | "acrylic fibre" | "acrylic fiber" | "acrylic yarn" | "acrylic knit") {
        return "polyacrylonitrile".to_string();
    }
    [" plastic", " fabric", " fibre", " fiber", " film"]
        .iter()
        .find_map(|form| lower.strip_suffix(form))
        .unwrap_or(&lower)
        .trim()
        .to_string()
}

/// Every row for any of these names, in file order.
pub fn for_names(names: &[&str]) -> Vec<Value> {
    let wanted: Vec<String> = names.iter().map(|q| bare(q)).collect();
    rows()
        .iter()
        .filter(|row| names_of(row).any(|n| wanted.iter().any(|q| q.eq_ignore_ascii_case(n.trim()))))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_names_its_source() {
        for row in rows() {
            for key in ["material", "test", "publisher", "url", "quote", "source_title"] {
                assert!(row[key].as_str().is_some_and(|v| !v.trim().is_empty()), "row missing {key}: {row}");
            }
            assert!(row["url"].as_str().unwrap().starts_with("https://"), "not an https link: {row}");
            assert!(!row["value"].is_null(), "row has no value: {row}");
        }
    }

    #[test]
    fn the_file_is_read() {
        assert!(rows().len() > 50, "data/more_sources.json did not parse");
        assert!(!for_names(&["ABS plastic"]).is_empty());
        // Kevlar and Nomex are both aramids, but a Nomex test is not a Kevlar test.
        assert!(for_names(&["aramid"]).is_empty());
        assert!(for_names(&["acrylic fabric"]).iter().all(|row| row["material"] == "polyacrylonitrile"));
        assert!(for_names(&["acrylic"]).iter().all(|row| row["material"] == "acrylic (pmma)"));
        assert!(for_names(&["kevlar"]).iter().all(|row| row["material"] == "kevlar"));
        assert!(for_names(&["nomex"]).iter().all(|row| row["material"] == "nomex"));
    }

    #[test]
    fn unknown_names_get_nothing() {
        assert!(for_names(&["definitely not a material"]).is_empty());
    }
}
