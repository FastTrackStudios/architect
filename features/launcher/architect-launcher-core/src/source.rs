//! A launcher [`Provider`] as one of the palette engine's
//! [`Source`](architect_palette::Source)s: the apps, the calculator, a
//! pack's workflows ranked by the same [`Engine`](architect_palette::Engine)
//! the ⌘K Command Palette ranks with — what the OS-wide launcher asks.

use architect_palette::{Item as Row, Ranking, Source};

use crate::provider::{Item, Provider};

/// A provider as an engine source; its id is the provider's name
/// (`apps`: `apps:` narrows to it).
pub struct ProviderSource {
    provider: Box<dyn Provider>,
    title: String,
}

impl ProviderSource {
    pub fn new(provider: Box<dyn Provider>) -> Self {
        let title = provider.config().name.clone();
        Self { provider, title }
    }

    /// The provider it asks.
    #[must_use]
    pub fn provider(&self) -> &dyn Provider {
        &*self.provider
    }
}

/// A launcher item as an engine row: the provider its source, its
/// secondary text its detail, its later search fields searched as the
/// detail too.
#[must_use]
pub fn row(item: &Item) -> Row {
    let detail = std::iter::once(item.sub.as_str())
        .chain(item.search_fields.iter().skip(1).map(String::as_str))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Row::new(item.id.clone(), item.label.clone())
        .source(item.provider.clone())
        .detail(detail)
}

impl Source for ProviderSource {
    fn id(&self) -> &str {
        self.provider.name()
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn ranking(&self) -> Ranking {
        Ranking::Open
    }
    fn items(&self, text: &str) -> Vec<Row> {
        if !self.provider.available() {
            return Vec::new();
        }
        self.provider
            .query(text, false)
            .map(|items| items.iter().map(row).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderConfig;
    use architect_palette::{Engine, Recents};

    struct Apps(ProviderConfig);

    impl Provider for Apps {
        fn name(&self) -> &str {
            "apps"
        }
        fn config(&self) -> &ProviderConfig {
            &self.0
        }
        fn config_mut(&mut self) -> &mut ProviderConfig {
            &mut self.0
        }
        fn query(
            &self,
            _: &str,
            _: bool,
        ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(vec![
                Item::new("firefox", "Firefox", "apps").with_sub("Web browser"),
                Item::new("files", "Files", "apps"),
            ])
        }
    }

    #[test]
    fn a_provider_is_ranked_by_the_palette_engine() {
        let mut engine = Engine::new();
        let config = ProviderConfig {
            name: "Applications".to_owned(),
            ..ProviderConfig::default()
        };
        engine.register(ProviderSource::new(Box::new(Apps(config))));
        assert_eq!(engine.title("apps"), Some("Applications"));
        let rows = engine.search("apps: browser", &Recents::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].item.id, "firefox");
        assert_eq!(rows[0].item.source, "apps");
        let rows = engine.search("fil", &Recents::default());
        assert_eq!(rows.first().map(|r| r.item.id.as_str()), Some("files"));
    }
}
