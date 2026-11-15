//! Units, by Cooklang's own database.
//!
//! As the Cookbook's `extensions/cookbook/src/moved/units.rs` does
//! (Task's `task:features/mealplan/pantry/src/units.rs` before it): 500
//! g and 0.5 kg are one row on a grocery list, and 1 kg in the pantry
//! covers 600 g a recipe asks for. Grams and cloves stay apart: there
//! is no honest answer to adding them.

use std::sync::OnceLock;

use cooklang::Converter;
use cooklang::quantity::{Quantity, Value};

/// The bundled units, parsed once.
fn converter() -> &'static Converter {
    static CONVERTER: OnceLock<Converter> = OnceLock::new();
    CONVERTER.get_or_init(Converter::bundled)
}

/// `value` in `from` expressed in `to`; `None` when they are not the
/// same kind of thing. Two counts (no unit) are the same thing.
#[must_use]
pub fn convert(value: f64, from: &str, to: &str) -> Option<f64> {
    if from.trim().eq_ignore_ascii_case(to.trim()) {
        return Some(value);
    }
    if from.trim().is_empty() || to.trim().is_empty() {
        return None;
    }
    let mut q = Quantity::new(Value::Number(value.into()), Some(from.trim().to_owned()));
    q.convert(to.trim(), converter()).ok()?;
    match q.value() {
        Value::Number(n) => Some(n.value()),
        _ => None,
    }
}

/// An amount in the unit a person would write: 1500 g as 1.5 kg, 1200
/// ml as 1.2 l; anything else as it is.
#[must_use]
pub fn tidy(qty: f64, unit: &str) -> (f64, String) {
    let bigger = match unit.trim().to_ascii_lowercase().as_str() {
        "g" | "gram" | "grams" => Some("kg"),
        "ml" | "milliliter" | "milliliters" | "millilitre" | "millilitres" => Some("l"),
        _ => None,
    };
    if let Some(to) = bigger
        && qty >= 1000.0
        && let Some(v) = convert(qty, unit, to)
    {
        return (v, to.to_owned());
    }
    (qty, unit.trim().to_owned())
}

/// An amount as text: at most two decimals, none when whole (`600`,
/// `1.5`, `0.33`).
#[must_use]
pub fn number(qty: f64) -> String {
    let rounded = (qty * 100.0).round() / 100.0;
    let s = format!("{rounded:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    #[test]
    fn converts_within_and_across_systems() {
        assert_eq!(convert(500.0, "g", "kg"), Some(0.5));
        assert_eq!(convert(2.0, "kg", "g"), Some(2000.0));
        let oz = convert(1000.0, "g", "oz").unwrap();
        assert!((oz - 35.27).abs() < 0.1, "{oz}");
    }

    #[test]
    fn refuses_what_does_not_add_up() {
        assert_eq!(convert(3.0, "g", "clove"), None);
        assert_eq!(
            convert(3.0, "ml", "g"),
            None,
            "volume to mass needs density"
        );
        assert_eq!(convert(2.0, "", "g"), None);
        assert_eq!(convert(2.0, "", ""), Some(2.0));
    }

    #[test]
    fn tidies_and_writes_numbers() {
        assert_eq!(tidy(1500.0, "g"), (1.5, "kg".to_owned()));
        assert_eq!(tidy(600.0, "g"), (600.0, "g".to_owned()));
        assert_eq!(number(600.0), "600");
        assert_eq!(number(1.5), "1.5");
        assert_eq!(number(1.0 / 3.0), "0.33");
    }
}
