//! "Is the air safe to breathe?" — a small table entered by hand from two NASA
//! documents. Nothing here is estimated: every entry names its document and page.
//!
//! 1. Wilson, Limero, Beck, James (1993), "A Combustion Products Analyzer for
//!    Contingency Use During Thermodegradation Events on Spacecraft",
//!    NTRS 19940007083. Table I (p. 591) lists the hazardous gases four materials
//!    give off; pp. 590–591 name NASA's four marker gases.
//! 2. JSC 20584 Rev C (2024), "Spacecraft Maximum Allowable Concentrations for
//!    Airborne Contaminants", NTRS 20240007713. The 1-hour and 24-hour limits are
//!    on PDF pages 11 (carbon monoxide) and 19 (the three hydrogen gases). The
//!    printed page numbers on those pages overlap, so PDF page numbers are cited.

pub const ANALYZER_URL: &str = "https://ntrs.nasa.gov/citations/19940007083";
pub const LIMITS_URL: &str = "https://ntrs.nasa.gov/citations/20240007713";

pub struct Gas {
    pub formula: &'static str,
    pub name: &'static str,
    /// ppm, JSC 20584 Rev C. None when that document sets no limit for the gas.
    pub limit_1h: Option<&'static str>,
    pub limit_24h: Option<&'static str>,
    pub limit_page: &'static str,
}

/// NASA's four marker gases (analyzer paper, pp. 590–591), then carbonyl fluoride,
/// which Table I lists for Teflon but which has no entry in JSC 20584 Rev C.
pub const GASES: [Gas; 5] = [
    Gas { formula: "CO", name: "Carbon monoxide", limit_1h: Some("425"), limit_24h: Some("100"), limit_page: "PDF p. 11" },
    Gas { formula: "HF", name: "Hydrogen fluoride", limit_1h: Some("3"), limit_24h: Some("3"), limit_page: "PDF p. 19" },
    Gas { formula: "HCl", name: "Hydrogen chloride", limit_1h: Some("5"), limit_24h: Some("2"), limit_page: "PDF p. 19" },
    Gas { formula: "HCN", name: "Hydrogen cyanide", limit_1h: Some("8"), limit_24h: Some("4"), limit_page: "PDF p. 19" },
    Gas { formula: "COF₂", name: "Carbonyl fluoride", limit_1h: None, limit_24h: None, limit_page: "" },
];

pub struct Source {
    /// The name used in Table I of the analyzer paper.
    pub material: &'static str,
    /// Lower-case words that identify this material in a fuel name. Only names the
    /// paper itself uses (Table I, and the materials list on p. 595).
    words: &'static [&'static str],
    pub gases: &'static [&'static str],
}

/// Table I, p. 591: "Hazardous Thermodegradation Products From Selected Materials".
pub const SOURCES: [Source; 4] = [
    Source { material: "Kapton", words: &["kapton", "polyimide"], gases: &["CO", "HCN"] },
    Source { material: "Teflon", words: &["teflon"], gases: &["CO", "HF", "COF₂"] },
    Source { material: "PVC", words: &["pvc", "polyvinyl chloride"], gases: &["CO", "HCl"] },
    Source { material: "Polyurethane", words: &["polyurethane", "pyrell"], gases: &["CO", "HCN"] },
];

pub const TABLE_PAGE: &str = "Table I, p. 591";

/// The Table I row for a fuel name from the database, if the paper covers it.
pub fn source_for(fuel: &str) -> Option<&'static Source> {
    let fuel = fuel.to_lowercase();
    SOURCES.iter().find(|s| s.words.iter().any(|w| fuel.contains(w)))
}

pub fn gas(formula: &str) -> Option<&'static Gas> {
    GASES.iter().find(|g| g.formula == formula)
}
