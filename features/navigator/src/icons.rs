//! Icons for folders, tags, properties and files.
//!
//! Notebook Navigator's `src/services/icons`, `src/utils/iconizeFormat.ts`
//! and `src/utils/fileIconUtils.ts`: an [`Icon`] read from what a person
//! or a note's frontmatter wrote, written back the same way, and the icon
//! a file takes from its name or its type when it has none of its own.
//!
//! Lucide icons are drawn from architect's own table
//! (`architect_shell::lucide`); an emoji is text; a vault SVG is drawn
//! from the vault. Icon packs (Phosphor, Font Awesome, …) are read and
//! kept by name, and drawn as their pack's mark until a pack is installed.

/// An icon, as Notebook Navigator names one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Icon {
    /// A Lucide icon by its name (`folder`).
    Lucide(String),
    /// An emoji (`📁`).
    Emoji(String),
    /// An icon pack's icon (`phosphor:house`).
    Pack { pack: String, name: String },
    /// An SVG file in the vault (`Icons/rocket.svg`).
    Vault(String),
}

/// The packs Notebook Navigator knows, with the short names its
/// frontmatter writes (`SHORT_FRONTMATTER_PROVIDER_ALIASES`) and Iconize's
/// prefixes.
const PACKS: &[(&str, &str, &str)] = &[
    ("phosphor", "ph", "Ph"),
    ("bootstrap-icons", "bi", "Bi"),
    ("fontawesome-solid", "fas", "Fas"),
    ("material-icons", "mi", "Mi"),
    ("simple-icons", "si", "Si"),
    ("rpg-awesome", "ra", "Ra"),
];

/// Whether `c` is a character emoji are made of.
fn emoji_char(c: char) -> bool {
    matches!(u32::from(c),
        0x1F000..=0x1FAFF
        | 0x2600..=0x27BF
        | 0x2B00..=0x2BFF
        | 0x2190..=0x21FF
        | 0x2300..=0x23FF
        | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139
        | 0xE0020..=0xE007F)
}

/// Whether `text` is one emoji (and its joiners, selectors and skin
/// tones) and nothing else (`extractFirstEmoji`).
#[must_use]
pub fn is_emoji(text: &str) -> bool {
    let mut any = false;
    for c in text.chars() {
        match u32::from(c) {
            0x200D | 0xFE0E | 0xFE0F => {}
            // A keycap's mark: `1️⃣`.
            0x20E3 => any = true,
            _ if emoji_char(c) => any = true,
            // Keycaps: a digit, `#` or `*` before U+20E3.
            _ if c.is_ascii_digit() || c == '#' || c == '*' => {}
            _ => return false,
        }
    }
    any
}

/// `CamelCase` cut into `kebab-case`: Iconize's `LiFolderOpen` is
/// `folder-open`, `LiArrowDown01` `arrow-down-0-1` (each digit a part, as
/// Lucide names them).
fn kebab(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if i > 0 && (c.is_ascii_uppercase() || c.is_ascii_digit()) {
            out.push('-');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

impl Icon {
    /// What a person or a note wrote, read (`deserializeIconFromFrontmatter`
    /// and the canonical ids): `folder` and `lucide-folder` (Lucide),
    /// `li:folder`, `ph:house` and `phosphor:house` (packs, short or
    /// long), `emoji:📁` or `📁`, `Icons/rocket.svg` (a vault SVG), and
    /// Iconize's `LiFolderOpen`, `PhHouse`. `None` for nothing.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim().trim_matches('"').trim();
        if t.is_empty() {
            return None;
        }
        if let Some(e) = t.strip_prefix("emoji:") {
            return is_emoji(e.trim()).then(|| Self::Emoji(e.trim().to_owned()));
        }
        if is_emoji(t) {
            return Some(Self::Emoji(t.to_owned()));
        }
        if t.to_ascii_lowercase().ends_with(".svg") {
            return Some(Self::Vault(
                t.strip_prefix("vault:")
                    .unwrap_or(t)
                    .trim_matches('/')
                    .to_owned(),
            ));
        }
        if let Some((pack, name)) = t.split_once(':') {
            let (pack, name) = (pack.trim().to_ascii_lowercase(), name.trim());
            if name.is_empty() {
                return None;
            }
            if pack == "li" || pack == "lucide" {
                return Some(Self::Lucide(name.to_ascii_lowercase()));
            }
            let long = PACKS
                .iter()
                .find(|(long, short, _)| *long == pack || *short == pack)
                .map_or(pack.as_str(), |(long, _, _)| *long);
            return Some(Self::Pack {
                pack: long.to_owned(),
                name: name.to_ascii_lowercase(),
            });
        }
        // Iconize: a pack's prefix, then the name in camel case.
        let iconize = |prefix: &str| {
            t.strip_prefix(prefix)
                .filter(|rest| {
                    rest.starts_with(|c: char| c.is_ascii_uppercase() || c.is_ascii_digit())
                })
                .map(kebab)
        };
        if let Some(name) = iconize("Li") {
            return Some(Self::Lucide(name));
        }
        for (long, _, prefix) in PACKS {
            if let Some(name) = iconize(prefix) {
                return Some(Self::Pack {
                    pack: (*long).to_owned(),
                    name,
                });
            }
        }
        let name = t.strip_prefix("lucide-").unwrap_or(t);
        name.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
            .then(|| Self::Lucide(name.to_ascii_lowercase()))
    }

    /// Its id as the navigator keeps it (`folder`, `emoji:📁`,
    /// `phosphor:house`, `vault:Icons/rocket.svg`).
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            Self::Lucide(n) => n.clone(),
            Self::Emoji(e) => format!("emoji:{e}"),
            Self::Pack { pack, name } => format!("{pack}:{name}"),
            Self::Vault(p) => format!("vault:{p}"),
        }
    }

    /// It as a note's frontmatter writes it (`serializeIconForFrontmatter`):
    /// a Lucide name bare, a pack by its short name (`ph:house`), an emoji
    /// as itself, a vault SVG by its path.
    #[must_use]
    pub fn frontmatter(&self) -> String {
        match self {
            Self::Lucide(n) => n.clone(),
            Self::Emoji(e) => e.clone(),
            Self::Pack { pack, name } => {
                let short = PACKS
                    .iter()
                    .find(|(long, _, _)| long == pack)
                    .map_or(pack.as_str(), |(_, short, _)| *short);
                format!("{short}:{name}")
            }
            Self::Vault(p) => p.clone(),
        }
    }
}

/// A file's type for its icon (`resolveFileTypeIconKey`): its extension,
/// lower case, `excalidraw.md` for a drawing.
#[must_use]
pub fn type_key(path: &str) -> String {
    let lower = path.to_lowercase();
    if lower.ends_with(".excalidraw.md") || lower.ends_with(".excalidraw") {
        return "excalidraw.md".to_owned();
    }
    lower
        .rsplit_once('.')
        .map_or_else(String::new, |(_, e)| e.to_owned())
}

/// The built-in type icons (`BUILT_IN_FILE_TYPE_ICON_MAP`), raster images
/// aside.
const BUILT_IN: &[(&str, &str)] = &[
    ("md", "file-text"),
    ("excalidraw.md", "pencil"),
    ("canvas", "layout-grid"),
    ("base", "database"),
    ("pdf", "file-text"),
];

const RASTER: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "avif",
];

/// `key=icon` pairs from a comma list (`meeting=users, todo=list-todo`),
/// keys as written (a name's needle keeps its spaces).
#[must_use]
pub fn icon_map(text: &str) -> Vec<(String, Icon)> {
    text.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k = k.trim_start_matches([' ', '\n']).trim_end_matches('\n');
            let icon = Icon::parse(v)?;
            (!k.trim().is_empty()).then(|| (k.to_owned(), icon))
        })
        .collect()
}

/// The icon a file's name gives it (`resolveFileNameMatchIconIdFromNeedles`):
/// the longest needle its name holds, any case.
#[must_use]
pub fn name_icon(name: &str, map: &[(String, Icon)]) -> Option<Icon> {
    let name = name.to_lowercase();
    let mut needles: Vec<(String, &Icon)> =
        map.iter().map(|(k, i)| (k.to_lowercase(), i)).collect();
    needles.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    needles
        .into_iter()
        .find(|(n, _)| name.contains(n.as_str()))
        .map(|(_, i)| i.clone())
}

/// The icon a file's type gives it (`resolveFileTypeIconId`): the
/// person's map, the built-in one, `image` for raster images.
#[must_use]
pub fn type_icon(path: &str, map: &[(String, Icon)]) -> Option<Icon> {
    let key = type_key(path);
    if key.is_empty() {
        return None;
    }
    map.iter()
        .find(|(k, _)| k.trim().trim_start_matches('.').eq_ignore_ascii_case(&key))
        .map(|(_, i)| i.clone())
        .or_else(|| {
            BUILT_IN
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, n)| Icon::Lucide((*n).to_owned()))
        })
        .or_else(|| {
            RASTER
                .contains(&key.as_str())
                .then(|| Icon::Lucide("image".to_owned()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_read_every_way_they_are_written_and_write_back() {
        let lucide = Some(Icon::Lucide("folder-open".to_owned()));
        for t in [
            "folder-open",
            "lucide-folder-open",
            "li:folder-open",
            "LiFolderOpen",
        ] {
            assert_eq!(Icon::parse(t), lucide, "{t}");
        }
        let house = Some(Icon::Pack {
            pack: "phosphor".to_owned(),
            name: "house".to_owned(),
        });
        assert_eq!(Icon::parse("ph:house"), house);
        assert_eq!(Icon::parse("phosphor:house"), house);
        assert_eq!(Icon::parse("PhHouse"), house);
        assert_eq!(
            house.as_ref().map(Icon::frontmatter).as_deref(),
            Some("ph:house")
        );
        assert_eq!(
            house.as_ref().map(Icon::id).as_deref(),
            Some("phosphor:house")
        );
        assert_eq!(Icon::parse("📁"), Some(Icon::Emoji("📁".to_owned())));
        assert_eq!(Icon::parse("emoji:👩‍💻"), Some(Icon::Emoji("👩‍💻".to_owned())));
        assert_eq!(
            Icon::parse("Icons/rocket.svg"),
            Some(Icon::Vault("Icons/rocket.svg".to_owned()))
        );
        assert_eq!(
            Icon::parse("LiArrowDown01"),
            Some(Icon::Lucide("arrow-down-0-1".to_owned()))
        );
        assert_eq!(Icon::parse(""), None);
        assert_eq!(Icon::parse("not an icon!"), None);
        assert!(!is_emoji("a"));
        assert!(is_emoji("1️⃣"));
    }

    #[test]
    fn files_take_icons_from_their_names_and_types() {
        let names = icon_map("meeting=users, todo = list-todo, ai =bot");
        assert_eq!(
            name_icon("Team Meeting notes", &names),
            Some(Icon::Lucide("users".to_owned()))
        );
        assert_eq!(name_icon("Plain", &names), None);
        let types = icon_map("pdf=book, csv=sheet");
        assert_eq!(
            type_icon("a/b.PDF", &types),
            Some(Icon::Lucide("book".to_owned()))
        );
        assert_eq!(
            type_icon("a/b.md", &types),
            Some(Icon::Lucide("file-text".to_owned()))
        );
        assert_eq!(
            type_icon("a/b.excalidraw.md", &types),
            Some(Icon::Lucide("pencil".to_owned()))
        );
        assert_eq!(
            type_icon("a/b.jpeg", &types),
            Some(Icon::Lucide("image".to_owned()))
        );
        assert_eq!(type_icon("a/b.zip", &types), None);
    }
}
