//! Reusable validators — the building blocks a [`Field`](crate::Field)'s
//! parser is made of. effect-form composes these from an Effect `Schema`
//! (`Schema.nonEmptyString()`, `Schema.minLength(8)`, …); here they're
//! plain closure factories you hand to [`use_field`](crate::use_field).
//!
//! Each returns a `Fn(&str) -> Result<T, String>`: `Ok` carries the
//! decoded value, `Err` a human-readable message.

/// Require a non-empty value (after trimming); decodes to the trimmed
/// string.
pub fn required(label: &'static str) -> impl Fn(&str) -> Result<String, String> + Clone {
    move |s: &str| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            Err(format!("{label} is required"))
        } else {
            Ok(trimmed.to_string())
        }
    }
}

/// Accept any value, including empty — an optional free-text field.
/// Decodes to the raw string.
pub fn optional() -> impl Fn(&str) -> Result<String, String> + Clone {
    |s: &str| Ok(s.to_string())
}

/// Require a non-empty value that parses as `T` (numbers, ids, …). The
/// derive-generated form fields use this for every non-`String` payload
/// field.
pub fn parse<T>(label: &'static str) -> impl Fn(&str) -> Result<T, String> + Clone
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    move |s: &str| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(format!("{label} is required"));
        }
        trimmed.parse().map_err(|e| format!("{label}: {e}"))
    }
}

/// Require at least `n` characters.
pub fn min_length(
    n: usize,
    label: &'static str,
) -> impl Fn(&str) -> Result<String, String> + Clone {
    move |s: &str| {
        if s.chars().count() < n {
            Err(format!("{label} must be at least {n} characters"))
        } else {
            Ok(s.to_string())
        }
    }
}

/// Require at most `n` characters.
pub fn max_length(
    n: usize,
    label: &'static str,
) -> impl Fn(&str) -> Result<String, String> + Clone {
    move |s: &str| {
        if s.chars().count() > n {
            Err(format!("{label} must be at most {n} characters"))
        } else {
            Ok(s.to_string())
        }
    }
}

/// A minimal email check — exactly one `@` with non-empty sides. Decodes to
/// the trimmed string. (Deliberately loose; real email validation belongs
/// server-side.)
pub fn email(label: &'static str) -> impl Fn(&str) -> Result<String, String> + Clone {
    move |s: &str| {
        let t = s.trim();
        let parts: Vec<&str> = t.split('@').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            Ok(t.to_string())
        } else {
            Err(format!("{label} must be a valid email address"))
        }
    }
}

/// Chain two validators: run `first`, then `second` on its decoded output.
/// Lets you compose `required` with a length/format check.
///
/// ```ignore
/// let name = use_field("", and(required("Name"), max_length(80, "Name")));
/// ```
pub fn and<T, U>(
    first: impl Fn(&str) -> Result<T, String>,
    second: impl Fn(&T) -> Result<U, String>,
) -> impl Fn(&str) -> Result<U, String> {
    move |s: &str| first(s).and_then(|t| second(&t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_rejects_blank_and_trims() {
        let v = required("Name");
        assert_eq!(v("  "), Err("Name is required".into()));
        assert_eq!(v("  hi "), Ok("hi".into()));
    }

    #[test]
    fn optional_always_ok() {
        assert_eq!(optional()(""), Ok(String::new()));
    }

    #[test]
    fn length_bounds() {
        assert!(min_length(3, "X")("ab").is_err());
        assert!(min_length(3, "X")("abc").is_ok());
        assert!(max_length(3, "X")("abcd").is_err());
        assert!(max_length(3, "X")("abc").is_ok());
    }

    #[test]
    fn email_shape() {
        assert!(email("E")("a@b").is_ok());
        assert!(email("E")("nope").is_err());
        assert!(email("E")("a@@b").is_err());
    }

    #[test]
    fn and_chains_required_then_length() {
        let v = and(required("Name"), |s: &String| max_length(3, "Name")(s));
        assert_eq!(v("  "), Err("Name is required".into()));
        assert!(v("abcd").is_err());
        assert_eq!(v(" ok "), Ok("ok".into()));
    }
}
