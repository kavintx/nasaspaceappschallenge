//! The one reader for oxygen and pressure in the `conditions` table.
//!
//! The backend is adding numeric columns (oxygen_pct_min/max, pressure_kpa_min/max). When a
//! row has them, `column` is used. Until then, and for rows where they are null, the text
//! columns are read here: oxygen as a percentage, pressure converted to kPa.
//!
//! Nothing is guessed. A number counts only when the text states its unit, so "0.17" on its
//! own, "air", "elevated" or "near ambient" give no value, and the test stays off the chart.

/// One stated value (min == max) or a stated range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub min: f64,
    pub max: f64,
}

impl Span {
    fn of(a: f64, b: f64) -> Span {
        Span { min: a.min(b), max: a.max(b) }
    }
    pub fn is_range(&self) -> bool {
        self.max > self.min
    }
}

/// A span from the numeric columns. A single value has min == max; one side alone is a single value.
pub fn column(min: Option<f64>, max: Option<f64>) -> Option<Span> {
    let ok = |v: &f64| v.is_finite() && *v >= 0.0;
    match (min.filter(ok), max.filter(ok)) {
        (Some(a), Some(b)) => Some(Span::of(a, b)),
        (Some(a), None) | (None, Some(a)) => Some(Span::of(a, a)),
        (None, None) => None,
    }
}

/// How two neighbouring numbers in the text are joined.
#[derive(Clone, Copy, PartialEq)]
enum Link {
    Range,
    List,
    Apart,
}

struct Num {
    value: f64,
    /// The text after this number, up to the next number.
    after: String,
    /// How it is joined to the next number.
    link: Link,
}

/// Every number in the text with what follows it. Digits inside a name ("O2", "C4H10") are not numbers.
fn numbers(text: &str) -> Vec<Num> {
    let chars: Vec<char> = text.chars().collect();
    let mut found: Vec<(f64, usize, usize)> = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut dot = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && !dot && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())) {
                dot |= chars[i] == '.';
                i += 1;
            }
            let in_name = start > 0 && chars[start - 1].is_alphabetic();
            if !in_name {
                if let Ok(value) = chars[start..i].iter().collect::<String>().parse::<f64>() {
                    found.push((value, start, i));
                }
            }
        } else {
            i += 1;
        }
    }
    (0..found.len())
        .map(|k| {
            let end = found.get(k + 1).map(|n| n.1).unwrap_or(chars.len());
            let after: String = chars[found[k].2..end].iter().collect::<String>().to_lowercase();
            let link = if k + 1 < found.len() { link(&after) } else { Link::Apart };
            Num { value: found[k].0, after, link }
        })
        .collect()
}

/// Words that may sit between two numbers of one range or list without separating them.
const BETWEEN: [&str; 22] = [
    "o2", "oxygen", "percent", "mol", "mole", "vol", "volume", "fraction", "approx", "approximately", "about", "psi", "psia", "kpa", "mpa", "pa", "atm",
    "torr", "mmhg", "mm", "hg", "bar",
];

fn link(between: &str) -> Link {
    let mut joiners: Vec<String> = vec![];
    for raw in between.split_whitespace() {
        let word = raw.trim_matches(|c: char| matches!(c, '%' | '~' | '(' | ')' | '.'));
        if word.is_empty() || BETWEEN.contains(&word) {
            continue;
        }
        joiners.push(word.to_string());
    }
    match joiners.join(" ").as_str() {
        "-" | "–" | "—" | "to" | "through" => Link::Range,
        "," | "and" | "or" | ", and" | ", or" | "&" | ";" | "/" => Link::List,
        _ => Link::Apart,
    }
}

/// Groups the kept numbers: neighbours joined as a range become one span, the rest stand alone.
fn spans(kept: &[(usize, f64)], nums: &[Num]) -> Vec<Span> {
    let mut out: Vec<Span> = vec![];
    let mut k = 0;
    while k < kept.len() {
        let (mut lo, mut hi) = (kept[k].1, kept[k].1);
        while k + 1 < kept.len() && kept[k + 1].0 == kept[k].0 + 1 && nums[kept[k].0].link == Link::Range {
            k += 1;
            lo = lo.min(kept[k].1);
            hi = hi.max(kept[k].1);
        }
        let span = Span::of(lo, hi);
        if !out.contains(&span) {
            out.push(span);
        }
        k += 1;
    }
    out
}

fn round(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

#[derive(Clone, Copy, PartialEq)]
enum O2Unit {
    Percent,
    Fraction,
}

fn oxygen_unit(after: &str) -> Option<O2Unit> {
    let a = after.trim_start();
    let percent = ["%", "percent", "mol %", "mol%", "mol. %", "mole %", "mole percent", "vol %", "vol%", "vol. %", "volume percent", "volume %"];
    if percent.iter().any(|p| a.starts_with(p)) {
        return Some(O2Unit::Percent);
    }
    let fraction = ["mole fraction", "mol fraction", "mol. fraction", "volume fraction", "molar fraction"];
    // "0.21 O2 mole fraction": the gas name may sit between the number and the unit.
    let lead: String = a.chars().take(24).collect();
    if fraction.iter().any(|p| lead.contains(p)) {
        return Some(O2Unit::Fraction);
    }
    None
}

/// The first word after the number and its unit: which gas the number is a share of, when the text says.
fn gas_after(after: &str) -> Option<String> {
    let unit_words = ["percent", "mol", "mole", "vol", "volume", "by", "fraction", "molar"];
    after.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty() && !unit_words.contains(w)).map(|w| w.to_string())
}

/// Oxygen levels in percent stated by this text. Empty when it states none that can be read.
pub fn oxygen_pct(text: &str) -> Vec<Span> {
    let nums = numbers(text);
    let n = nums.len();
    let mut unit: Vec<Option<O2Unit>> = nums.iter().map(|x| oxygen_unit(&x.after)).collect();
    let gas: Vec<Option<String>> = nums.iter().map(|x| gas_after(&x.after)).collect();
    let mut oxygen: Vec<bool> = gas.iter().map(|g| matches!(g.as_deref(), Some("o2") | Some("oxygen"))).collect();
    // "18-21% O2": the unit and the gas written after the last number of a range cover the whole range.
    for k in (0..n.saturating_sub(1)).rev() {
        if nums[k].link != Link::Apart && unit[k].is_none() {
            unit[k] = unit[k + 1];
        }
        if nums[k].link == Link::Range && oxygen[k + 1] {
            oxygen[k] = true;
        }
    }
    let other_gas = ["n2", "nitrogen", "he", "helium", "co2", "ar", "argon", "n", "fuel", "diluent", "inert", "methane", "ch4", "h2", "hydrogen", "co"];
    let any_oxygen = oxygen.iter().any(|x| *x);
    let mut kept: Vec<(usize, f64)> = vec![];
    for k in 0..n {
        let Some(u) = unit[k] else { continue };
        // When the text says which numbers are oxygen, only those count ("21% O2 and 79% N2").
        if any_oxygen && !oxygen[k] {
            continue;
        }
        if !any_oxygen && gas[k].as_deref().is_some_and(|g| other_gas.contains(&g)) {
            continue;
        }
        let pct = match u {
            O2Unit::Percent => nums[k].value,
            O2Unit::Fraction if nums[k].value <= 1.0 => round(nums[k].value * 100.0, 4),
            O2Unit::Fraction => return vec![],
        };
        if !(pct > 0.0 && pct <= 100.0) {
            return vec![];
        }
        kept.push((k, pct));
    }
    spans(&kept, &nums)
}

/// kPa per unit, from the word that follows the number. Gauge pressure ("psig") is not absolute and is refused.
fn pressure_unit(after: &str) -> Option<f64> {
    let a = after.trim_start();
    let word: String = a.chars().take_while(|c| c.is_alphabetic()).collect();
    match word.as_str() {
        "kpa" => Some(1.0),
        "pa" => Some(0.001),
        "mpa" => Some(1000.0),
        "bar" => Some(100.0),
        "mbar" => Some(0.1),
        "psi" | "psia" => Some(6.894_757),
        "atm" | "atmosphere" | "atmospheres" | "atmospheric" => Some(101.325),
        "torr" | "mmhg" => Some(0.133_322_4),
        "mm" if a[word.len()..].trim_start().starts_with("hg") => Some(0.133_322_4),
        _ => None,
    }
}

/// Pressures in kPa stated by this text. Empty when it states none that can be read.
pub fn pressure_kpa(text: &str) -> Vec<Span> {
    let nums = numbers(text);
    let n = nums.len();
    let mut unit: Vec<Option<f64>> = nums.iter().map(|x| pressure_unit(&x.after)).collect();
    for k in (0..n.saturating_sub(1)).rev() {
        if nums[k].link != Link::Apart && unit[k].is_none() {
            unit[k] = unit[k + 1];
        }
    }
    let mut kept: Vec<(usize, f64)> = vec![];
    for k in 0..n {
        let Some(per) = unit[k] else { continue };
        let kpa = round(nums[k].value * per, 2);
        if !(kpa.is_finite() && (0.0..=200_000.0).contains(&kpa)) {
            return vec![];
        }
        kept.push((k, kpa));
    }
    let out = spans(&kept, &nums);
    // Zero is only meaningful as the low end of a range ("0 to 3 atm").
    if out.iter().any(|s| s.max <= 0.0) {
        return vec![];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(v: f64) -> Vec<Span> {
        vec![Span { min: v, max: v }]
    }
    fn range(a: f64, b: f64) -> Vec<Span> {
        vec![Span { min: a, max: b }]
    }

    #[test]
    fn oxygen_single_values() {
        assert_eq!(oxygen_pct("21%"), one(21.0));
        assert_eq!(oxygen_pct("17% O2"), one(17.0));
        assert_eq!(oxygen_pct("13.6%"), one(13.6));
        assert_eq!(oxygen_pct("21 percent (ambient)"), one(21.0));
        assert_eq!(oxygen_pct("22.0 mol %"), one(22.0));
        assert_eq!(oxygen_pct("Air (~21% O2)"), one(21.0));
        assert_eq!(oxygen_pct("Air (approx 21% O2)"), one(21.0));
    }

    #[test]
    fn oxygen_ranges_and_lists() {
        assert_eq!(oxygen_pct("18-21% O2"), range(18.0, 21.0));
        assert_eq!(oxygen_pct("17-83%"), range(17.0, 83.0));
        assert_eq!(oxygen_pct("14% to 18% by volume"), range(14.0, 18.0));
        assert_eq!(oxygen_pct("21% to 22% O2"), range(21.0, 22.0));
        assert_eq!(oxygen_pct("~21% to ~14%"), range(14.0, 21.0));
        assert_eq!(oxygen_pct("15%, 18% and 21%"), vec![Span { min: 15.0, max: 15.0 }, Span { min: 18.0, max: 18.0 }, Span { min: 21.0, max: 21.0 }]);
    }

    #[test]
    fn oxygen_mole_fractions_need_the_words() {
        assert_eq!(oxygen_pct("0.10 to 0.50 mole fraction"), range(10.0, 50.0));
        assert_eq!(oxygen_pct("near 0.21 O2 mole fraction"), one(21.0));
        // A bare decimal does not say what it is.
        assert!(oxygen_pct("0.17").is_empty());
        assert!(oxygen_pct("0.30").is_empty());
    }

    #[test]
    fn oxygen_ignores_other_gases() {
        assert_eq!(oxygen_pct("21% O2 and 79% N2"), one(21.0));
        assert_eq!(oxygen_pct("n-butane/oxygen premixture (50% n-C4H10-50% O2)"), one(50.0));
        assert_eq!(oxygen_pct("30% O2 in 70% He"), one(30.0));
        assert!(oxygen_pct("79% N2").is_empty());
    }

    #[test]
    fn oxygen_garbage_gives_nothing() {
        for text in ["", "air", "elevated", "varies", "Varied to determine flammability limits", "air and a mixture of oxygen and helium", "O2", "150%", "0%", "21", "about 21 or so"] {
            assert!(oxygen_pct(text).is_empty(), "{text:?} should not parse");
        }
        assert_eq!(oxygen_pct("Varied (tests conducted at levels including 16.1% and lower)"), one(16.1));
    }

    #[test]
    fn pressure_units_convert_to_kpa() {
        assert_eq!(pressure_kpa("100.0 kPa"), one(100.0));
        assert_eq!(pressure_kpa("54.1 kPa"), one(54.1));
        assert_eq!(pressure_kpa("1 atm"), one(101.33));
        assert_eq!(pressure_kpa("1.0 atm"), one(101.33));
        assert_eq!(pressure_kpa("0.70 atm"), one(70.93));
        assert_eq!(pressure_kpa("14.7 psia"), one(101.35));
        assert_eq!(pressure_kpa("10.2 psi"), one(70.33));
        assert_eq!(pressure_kpa("3.2 psia"), one(22.06));
        assert_eq!(pressure_kpa("532.2 mmHg"), one(70.95));
        assert_eq!(pressure_kpa("760 torr"), one(101.33));
        assert_eq!(pressure_kpa("760 mm Hg"), one(101.33));
        assert_eq!(pressure_kpa("0.1 MPa"), one(100.0));
    }

    #[test]
    fn pressure_ranges_and_lists() {
        assert_eq!(pressure_kpa("0.5 to 1.0 atm"), range(50.66, 101.33));
        assert_eq!(pressure_kpa("35-130 kPa"), range(35.0, 130.0));
        assert_eq!(pressure_kpa("100 to 40 kPa"), range(40.0, 100.0));
        assert_eq!(pressure_kpa("0 to 3 atm"), range(0.0, 303.98));
        assert_eq!(pressure_kpa("0.7 and 1 atmospheric pressures"), vec![Span { min: 70.93, max: 70.93 }, Span { min: 101.33, max: 101.33 }]);
    }

    #[test]
    fn pressure_garbage_gives_nothing() {
        for text in [
            "",
            "near ambient",
            "atmospheric",
            "normal atmospheric pressure",
            "Atmospheric pressure (implied by LOI context)",
            "Nominally 100-liter combustion chamber (pressure not explicitly stated as a variable parameter in text)",
            "14.7",
            "5 psig",
            "0 kPa",
            "high",
        ] {
            assert!(pressure_kpa(text).is_empty(), "{text:?} should not parse");
        }
    }

    #[test]
    fn numeric_columns() {
        assert_eq!(column(Some(21.0), Some(21.0)), Some(Span { min: 21.0, max: 21.0 }));
        assert_eq!(column(Some(30.0), Some(18.0)), Some(Span { min: 18.0, max: 30.0 }));
        assert_eq!(column(Some(70.3), None), Some(Span { min: 70.3, max: 70.3 }));
        assert_eq!(column(None, None), None);
        assert_eq!(column(Some(f64::NAN), None), None);
        assert!(Span { min: 1.0, max: 2.0 }.is_range());
    }
}
