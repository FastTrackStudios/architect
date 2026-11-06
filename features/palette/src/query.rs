//! What a typed query asks for, by its first character.

/// What a query asks for, by its first character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `>` or nothing: commands.
    Commands,
    /// `@`: this page's targets (a track, a section).
    Targets,
    /// `#`: settings.
    Settings,
    /// `?`: what a key does.
    Help,
}

/// A typed query: its scope and the text to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub scope: Scope,
    pub text: String,
}

impl Query {
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim_start();
        let (scope, rest) = match trimmed.chars().next() {
            Some('>') => (Scope::Commands, trimmed.get(1..).unwrap_or("")),
            Some('@') => (Scope::Targets, trimmed.get(1..).unwrap_or("")),
            Some('#') => (Scope::Settings, trimmed.get(1..).unwrap_or("")),
            Some('?') => (Scope::Help, trimmed.get(1..).unwrap_or("")),
            _ => (Scope::Commands, trimmed),
        };
        Self {
            scope,
            text: rest.trim().to_owned(),
        }
    }
}
