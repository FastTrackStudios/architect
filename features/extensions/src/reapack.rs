//! `ReaPack` repositories as a registry source.
//!
//! A repository's `index.xml` is read into [`Listing`]s, so its packages
//! (JSFX, scripts, presets) are
//! searched and installed beside extensions, as the app's package kinds.
//!
//! What is read: each `<category>`'s `<reapack>` packages (name, `type`,
//! `desc`), each `<version>` of one (`ReaPack`'s version names, read as
//! semver: `1.2` is `1.2.0`, `2.0beta1` is `2.0.0-beta1`), and each version's
//! `<source>` files for this platform (`platform="all"`, this OS's, this
//! OS and architecture's) with where each goes (`file`, else the
//! package's name). Pinning, dependencies and offline bundles are later.

use semver::Version;

use crate::index::{Index, Listing, Location};

/// Why an `index.xml` could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaPackError(pub String);

impl std::fmt::Display for ReaPackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the ReaPack index cannot be read: {}", self.0)
    }
}

impl std::error::Error for ReaPackError {}

/// Reads a `ReaPack` `index.xml` for this platform: one listing a package
/// and version that has files here (`reapack.<type>` its kind, its
/// category its category, `<category>/<name>` its id).
///
/// # Errors
///
/// It is not XML, or not a `ReaPack` index.
pub fn parse(xml: &str) -> Result<Index, ReaPackError> {
    parse_for(xml, std::env::consts::OS, std::env::consts::ARCH)
}

/// [`parse`] for platform `os` (`linux`, `macos`, `windows`) on `arch`
/// (`x86_64`, `aarch64`, …).
///
/// # Errors
///
/// As [`parse`].
pub fn parse_for(xml: &str, os: &str, arch: &str) -> Result<Index, ReaPackError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| ReaPackError(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "index" {
        return Err(ReaPackError(format!(
            "its root is <{}>, not <index>",
            root.tag_name().name()
        )));
    }
    let mut listings = Vec::new();
    for category in root.children().filter(|n| n.has_tag_name("category")) {
        let category_name = category.attribute("name").unwrap_or_default();
        for package in category.children().filter(|n| n.has_tag_name("reapack")) {
            let Some(name) = package.attribute("name") else {
                continue;
            };
            let kind = package.attribute("type").unwrap_or("script");
            let description = package.attribute("desc").unwrap_or(name);
            for version in package.children().filter(|n| n.has_tag_name("version")) {
                let Some(parsed) = version.attribute("name").and_then(lenient) else {
                    continue;
                };
                let files: Vec<(String, String)> = version
                    .children()
                    .filter(|n| n.has_tag_name("source"))
                    .filter(|s| platform_ok(s.attribute("platform").unwrap_or("all"), os, arch))
                    .filter_map(|s| {
                        let url = s.text()?.trim().to_owned();
                        let file = s.attribute("file").unwrap_or(name).to_owned();
                        (!url.is_empty()).then_some((url, file))
                    })
                    .collect();
                if files.is_empty() {
                    continue;
                }
                listings.push(Listing {
                    id: format!("{category_name}/{name}"),
                    name: name.to_owned(),
                    version: parsed,
                    kind: format!("reapack.{kind}"),
                    description: description.to_owned(),
                    category: category_name.to_owned(),
                    location: Location::Files(files),
                    platforms: Vec::new(),
                    changelog: String::new(),
                });
            }
        }
    }
    Ok(Index {
        name: root.attribute("name").unwrap_or_default().to_owned(),
        listings,
    })
}

/// A `ReaPack` version name as semver: up to three numbers (missing ones
/// 0), what follows them a pre-release (`2.0beta1` → `2.0.0-beta1`).
#[must_use]
pub fn lenient(name: &str) -> Option<Version> {
    let name = name.trim().trim_start_matches(['v', 'V']);
    let split = name
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.'))
        .map_or(name.len(), |(i, _)| i);
    let (numbers, rest) = name.split_at(split);
    let mut parts = numbers
        .split('.')
        .filter(|p| !p.is_empty())
        .map(str::parse::<u64>);
    let major = parts.next()?.ok()?;
    let minor = parts.next().transpose().ok()?.unwrap_or(0);
    let patch = parts.next().transpose().ok()?.unwrap_or(0);
    let mut version = Version::new(major, minor, patch);
    let pre: String = rest
        .trim_start_matches(['-', '.', '_', '+'])
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '.' })
        .collect();
    let pre = pre.trim_matches('.');
    if !pre.is_empty() {
        version.pre = semver::Prerelease::new(pre).ok()?;
    }
    Some(version)
}

/// Whether a `<source platform="…">` is for `os` on `arch`.
fn platform_ok(platform: &str, os: &str, arch: &str) -> bool {
    let os = match os {
        "macos" => "darwin",
        "windows" => "windows",
        other => other,
    };
    let bits64 = matches!(arch, "x86_64" | "aarch64");
    match platform {
        "all" => true,
        "windows" => os == "windows",
        "win32" => os == "windows" && arch == "x86",
        "win64" => os == "windows" && arch == "x86_64",
        "windows-arm64ec" => os == "windows" && arch == "aarch64",
        "darwin" => os == "darwin",
        "darwin32" => os == "darwin" && !bits64,
        "darwin64" => os == "darwin" && arch == "x86_64",
        "darwin-arm64" => os == "darwin" && arch == "aarch64",
        "linux" => os == "linux",
        "linux32" => os == "linux" && arch == "x86",
        "linux64" => os == "linux" && arch == "x86_64",
        "linux-armv7l" => os == "linux" && arch == "arm",
        "linux-aarch64" => os == "linux" && arch == "aarch64",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{lenient, parse_for};
    use crate::index::Location;

    const INDEX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<index version="1" name="DSP-Ports">
  <category name="Dynamics">
    <reapack name="compressor.jsfx" type="effect" desc="A compressor">
      <version name="1.0" author="chmaha">
        <source>https://example.com/compressor-1.0.jsfx</source>
      </version>
      <version name="1.1beta2" author="chmaha">
        <source>https://example.com/compressor-1.1.jsfx</source>
        <source file="compressor.png">https://example.com/skin.png</source>
      </version>
    </reapack>
  </category>
  <category name="Tools">
    <reapack name="helper.ext" type="extension" desc="Native helper">
      <version name="2">
        <source platform="win64">https://example.com/helper.dll</source>
        <source platform="linux64" file="helper.so">https://example.com/helper.so</source>
      </version>
    </reapack>
  </category>
</index>"#;

    #[test]
    fn an_index_xml_reads_as_listings_for_this_platform() {
        let index = parse_for(INDEX, "linux", "x86_64").unwrap();
        assert_eq!(index.name, "DSP-Ports");
        assert_eq!(index.listings.len(), 3);
        let newest = index.search("compressor");
        let c = &newest[0].listing;
        assert_eq!(c.id, "Dynamics/compressor.jsfx");
        assert_eq!(c.kind, "reapack.effect");
        assert_eq!(c.version.to_string(), "1.1.0-beta2");
        assert!(
            matches!(&c.location, Location::Files(f) if f.len() == 2 && f[1].1 == "compressor.png")
        );
        let helper = index
            .listings
            .iter()
            .find(|l| l.name == "helper.ext")
            .unwrap();
        assert_eq!(
            helper.location,
            Location::Files(vec![(
                "https://example.com/helper.so".into(),
                "helper.so".into()
            )])
        );

        let mac = parse_for(INDEX, "macos", "aarch64").unwrap();
        assert_eq!(mac.listings.len(), 2, "no helper build for a Mac");
        assert!(parse_for("<feed/>", "linux", "x86_64").is_err());
    }

    #[test]
    fn reapack_versions_read_as_semver() {
        assert_eq!(lenient("1.2").unwrap().to_string(), "1.2.0");
        assert_eq!(lenient("v3").unwrap().to_string(), "3.0.0");
        assert_eq!(lenient("2.0beta1").unwrap().to_string(), "2.0.0-beta1");
        assert_eq!(lenient("1.0.3-rc 2").unwrap().to_string(), "1.0.3-rc.2");
        assert!(lenient("beta").is_none());
    }
}
