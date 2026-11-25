//! Configuration from the environment: what `#[derive(auth_macros::Config)]`
//! reads through.

use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{var} is required")]
    Missing { var: String },
    #[error("{var}: {detail}")]
    Invalid { var: String, detail: String },
    #[error("{var}_FILE ({path}): {source}")]
    File {
        var: String,
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The value of `var`, trimmed, or `None` when unset or blank.
#[must_use]
pub fn var(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A secret: `VAR`, or the trimmed contents of the file `VAR_FILE` names
/// (a mounted file does not show up in a process listing).
pub fn secret(var_name: &str) -> Result<Option<String>, ConfigError> {
    let file_var = format!("{var_name}_FILE");
    if let Some(path) = var(&file_var) {
        let contents = std::fs::read_to_string(&path).map_err(|source| ConfigError::File {
            var: var_name.to_owned(),
            path,
            source,
        })?;
        return Ok(Some(contents.trim().to_owned()));
    }
    Ok(var(var_name))
}

/// How a field reads itself from an environment value (`None`: unset).
pub trait FromEnvValue: Sized {
    fn from_env_value(var: &str, raw: Option<String>) -> Result<Self, ConfigError>;
}

fn parse<T: FromStr>(var: &str, raw: &str) -> Result<T, ConfigError>
where
    T::Err: std::fmt::Display,
{
    raw.parse().map_err(|e: T::Err| ConfigError::Invalid {
        var: var.to_owned(),
        detail: e.to_string(),
    })
}

macro_rules! scalar {
    ($($t:ty),*) => {$(
        impl FromEnvValue for $t {
            fn from_env_value(var: &str, raw: Option<String>) -> Result<Self, ConfigError> {
                match raw {
                    Some(raw) => parse(var, &raw),
                    None => Err(ConfigError::Missing { var: var.to_owned() }),
                }
            }
        }
    )*};
}

scalar!(
    String, i8, i16, i32, i64, u8, u16, u32, u64, usize, f32, f64
);

impl FromEnvValue for bool {
    fn from_env_value(var: &str, raw: Option<String>) -> Result<Self, ConfigError> {
        match raw.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("1" | "true" | "yes" | "on") => Ok(true),
            Some("0" | "false" | "no" | "off") => Ok(false),
            Some(other) => Err(ConfigError::Invalid {
                var: var.to_owned(),
                detail: format!("expected a boolean, got {other:?}"),
            }),
            None => Err(ConfigError::Missing {
                var: var.to_owned(),
            }),
        }
    }
}

impl<T: FromEnvValue> FromEnvValue for Option<T> {
    fn from_env_value(var: &str, raw: Option<String>) -> Result<Self, ConfigError> {
        raw.map_or_else(
            || Ok(None),
            |raw| T::from_env_value(var, Some(raw)).map(Some),
        )
    }
}

impl<T: FromEnvValue> FromEnvValue for Vec<T> {
    fn from_env_value(var: &str, raw: Option<String>) -> Result<Self, ConfigError> {
        raw.map_or_else(
            || Ok(Self::new()),
            |raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| T::from_env_value(var, Some(s.to_owned())))
                    .collect()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_as_they_always_have() {
        assert!(bool::from_env_value("X", Some("Yes".into())).unwrap());
        assert!(!bool::from_env_value("X", Some("off".into())).unwrap());
        assert!(bool::from_env_value("X", Some("maybe".into())).is_err());
        assert_eq!(
            Vec::<String>::from_env_value("X", Some("a, b,,c".into())).unwrap(),
            vec!["a", "b", "c"]
        );
        assert_eq!(Option::<u64>::from_env_value("X", None).unwrap(), None);
        assert!(matches!(
            String::from_env_value("X", None),
            Err(ConfigError::Missing { .. })
        ));
    }
}
