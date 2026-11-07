//! What a node is told: a TOML file, then the environment over it.
//!
//! `architect-desktop serve --config <file>` (or `$ARCHITECT_CONFIG`)
//! reads the file; every `ARCHITECT_*` variable below then wins over the
//! file's value, so a container sets what differs per deployment and
//! mounts the rest. Secrets are never values here: the account's session
//! and the MCP bearer are **files** a secret is mounted at.
//!
//! | Variable | File key | Default | What |
//! |---|---|---|---|
//! | `ARCHITECT_NAME` | `name` | `architect` | the device's name on the account |
//! | `ARCHITECT_BIND` | `bind` | `0.0.0.0:8080` | HTTP: `/health`, Sites, `/mcp`, the HTTP face, `/vox` |
//! | `ARCHITECT_DATA_DIR` | `data_dir` | `architect-data` | the node's state: its iroh key, device id, documents, built sites |
//! | `ARCHITECT_VAULTS` | `[vaults]` | none | `id=path,…`: the vaults it holds (a relative path is under the data dir) |
//! | `ARCHITECT_SITES` | `[sites]` | none | `/mount=vault,…`: vaults published as Sites |
//! | `ARCHITECT_ISSUER` | `issuer` | none | the account server (`https://auth.fasttrackstudio.app`) |
//! | `ARCHITECT_SESSION_FILE` | `session_file` | none | a file holding the account's session token |
//! | `ARCHITECT_ORG` | `org` | none | the org (slug) it offers the account when the account shares none yet |
//! | `ARCHITECT_HOST_ORG` | `host_org` | none | an issuer org (slug) of the session's: the node serves its enrolment lane, `/org/{slug}/vox`, and its vaults are that org's |
//! | `ARCHITECT_ENROL_AT` | `enrol_at` | none | another node's HTTP origin: this node enrols itself in `enrol_org` there, once |
//! | `ARCHITECT_ENROL_ORG` | `enrol_org` | none | the issuer org (slug) to enrol in there; its vaults are that org's |
//! | `ARCHITECT_ENROL_ROLE` | `enrol_role` | `member` | the role it asks for: at most the session's own in the org |
//! | `ARCHITECT_PEERS` | `peers` | none | `endpoint@host:port,…`: nodes dialled directly (no lookup) |
//! | `ARCHITECT_IROH` | `iroh` | `n0` | `n0` (relays and lookup) or `local` (loopback, no relay: tests) |
//! | `ARCHITECT_IROH_BIND` | `iroh_bind` | any port | the iroh UDP socket (`0.0.0.0:7777`, for a fixed port) |
//! | `ARCHITECT_MCP_TOKEN_FILE` | `mcp_token_file` | none | `/mcp` is served only with a bearer, read from this file |
//! | `ARCHITECT_SWEEP_SECS` | `sweep_secs` | `30` | how often it sweeps its peers: the fallback for what a live bridge missed |
//! | `ARCHITECT_LIVE_MAX` | `live_max` | `256` | how many notes it keeps live with each peer (a `DocSync` bridge each); `0` sweeps only |
//! | `ARCHITECT_FILES_DIR` | `files_dir` | none | projects and asset libraries (`<org>/Projects/<p>`, `<org>/Assets/<kind>`): the tree held in place, or where pins go (`architect_files::sync`) |
//! | `ARCHITECT_FILES_HOLD` | `files_hold` | `false` | hold every root found in `files_dir` (the always-on node); otherwise only pinned ones have a working copy |
//! | `ARCHITECT_DRIVE` | `drive` | none | mount every root this device knows here, as a real folder (FUSE; `~/Architect` when the agent is installed, `architect-desktop agent install`); empty is off |
//! | `ARCHITECT_FILES_SECS` | `files_secs` | `2` | how often each root is scanned and brought to its heads |
//! | `ARCHITECT_BLOB_S3_BUCKET` | `blob_s3_bucket` | none | the files' bytes in an S3 bucket too (Garage, MinIO, R2, AWS, Ceph RGW): turns the backend on |
//! | `ARCHITECT_BLOB_S3_ENDPOINT` | `blob_s3_endpoint` | AWS | the S3 endpoint (`http://garage.lan:3900`) |
//! | `ARCHITECT_BLOB_S3_REGION` | `blob_s3_region` | `us-east-1` | its region (`garage`, `auto` for R2) |
//! | `ARCHITECT_BLOB_S3_PREFIX` | `blob_s3_prefix` | `architect` | keys go under it: `<prefix>/sha256/<hh>/<hex>` |
//! | `ARCHITECT_BLOB_S3_ACCESS_KEY_ID` | `blob_s3_access_key_id` | none | the bucket's access key id |
//! | `ARCHITECT_BLOB_S3_SECRET_FILE` | `blob_s3_secret_file` | none | a file holding its secret key (or `ARCHITECT_BLOB_S3_SECRET_ACCESS_KEY`, read at start and never kept in the config) |
//! | `ARCHITECT_BLOB_PLACEMENT` | `blob_placement` | `replicate` | `replicate`: every byte here and in the bucket; `primary`: the bucket holds them, this node keeps a cache |
//! | `ARCHITECT_BLOB_CACHE_MB` | `blob_cache_mb` | `10240` | with `primary`, how much of the store is kept here (working copies are never evicted) |
//! | `ARCHITECT_S3_BIND` | `s3_bind` | none | the roots over S3 (`127.0.0.1:9000`): a bucket per org, keys from `serve s3-key issue` (needs `files_dir`) |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// How the node's iroh endpoint finds peers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IrohMode {
    /// n0's relays and address lookup: dialable by bare endpoint id.
    #[default]
    N0,
    /// Loopback only, no relay, no lookup: peers come from `peers`.
    Local,
}

/// Where a node keeps the files' bytes when it has a bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// Every byte on this node and in the bucket.
    #[default]
    Replicate,
    /// The bucket holds them; the node keeps `blob_cache_mb` of its store.
    Primary,
}

/// A node's configuration. See the module docs for each field.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub name: String,
    pub bind: String,
    pub data_dir: PathBuf,
    pub vaults: BTreeMap<String, PathBuf>,
    pub sites: BTreeMap<String, String>,
    pub issuer: Option<String>,
    pub session_file: Option<PathBuf>,
    pub org: Option<String>,
    pub host_org: Option<String>,
    pub enrol_at: Option<String>,
    pub enrol_org: Option<String>,
    pub enrol_role: String,
    pub peers: Vec<String>,
    pub iroh: IrohMode,
    pub iroh_bind: Option<String>,
    pub mcp_token_file: Option<PathBuf>,
    pub sweep_secs: u64,
    pub live_max: usize,
    pub files_dir: Option<PathBuf>,
    pub files_hold: bool,
    pub files_secs: u64,
    pub drive: Option<PathBuf>,
    pub s3_bind: Option<String>,
    pub blob_s3_bucket: Option<String>,
    pub blob_s3_endpoint: Option<String>,
    pub blob_s3_region: String,
    pub blob_s3_prefix: String,
    pub blob_s3_access_key_id: Option<String>,
    pub blob_s3_secret_file: Option<PathBuf>,
    pub blob_placement: Placement,
    pub blob_cache_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "architect".to_owned(),
            bind: "0.0.0.0:8080".to_owned(),
            data_dir: PathBuf::from("architect-data"),
            vaults: BTreeMap::new(),
            sites: BTreeMap::new(),
            issuer: None,
            session_file: None,
            org: None,
            host_org: None,
            enrol_at: None,
            enrol_org: None,
            enrol_role: "member".to_owned(),
            peers: Vec::new(),
            iroh: IrohMode::N0,
            iroh_bind: None,
            mcp_token_file: None,
            sweep_secs: 30,
            live_max: 256,
            files_dir: None,
            files_hold: false,
            files_secs: 2,
            drive: None,
            s3_bind: None,
            blob_s3_bucket: None,
            blob_s3_endpoint: None,
            blob_s3_region: String::new(),
            blob_s3_prefix: "architect".to_owned(),
            blob_s3_access_key_id: None,
            blob_s3_secret_file: None,
            blob_placement: Placement::Replicate,
            blob_cache_mb: 10_240,
        }
    }
}

/// Why a configuration could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("{var}: {why}")]
    Env { var: &'static str, why: String },
}

impl Config {
    /// The file at `file` (if any), then the environment `env` over it.
    ///
    /// # Errors
    ///
    /// The file is unreadable or not this shape, or a variable is malformed.
    pub fn load(
        file: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let mut config = match file {
            Some(path) => Self::from_file(path)?,
            None => Self::default(),
        };
        config.apply_env(env)?;
        Ok(config)
    }

    /// The TOML file at `path`, over the defaults.
    ///
    /// # Errors
    ///
    /// Unreadable, or not this shape.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source: Box::new(source),
        })
    }

    fn apply_env(&mut self, env: impl Fn(&str) -> Option<String>) -> Result<(), ConfigError> {
        let get = |var: &str| env(var).filter(|v| !v.trim().is_empty());
        if let Some(v) = get("ARCHITECT_NAME") {
            self.name = v;
        }
        if let Some(v) = get("ARCHITECT_BIND") {
            self.bind = v;
        }
        if let Some(v) = get("ARCHITECT_DATA_DIR") {
            self.data_dir = PathBuf::from(v);
        }
        if let Some(v) = get("ARCHITECT_VAULTS") {
            self.vaults = pairs("ARCHITECT_VAULTS", &v)?
                .into_iter()
                .map(|(k, p)| (k, PathBuf::from(p)))
                .collect();
        }
        if let Some(v) = get("ARCHITECT_SITES") {
            self.sites = pairs("ARCHITECT_SITES", &v)?.into_iter().collect();
        }
        if let Some(v) = get("ARCHITECT_ISSUER") {
            self.issuer = Some(v);
        }
        if let Some(v) = get("ARCHITECT_SESSION_FILE") {
            self.session_file = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_ORG") {
            self.org = Some(v);
        }
        if let Some(v) = get("ARCHITECT_HOST_ORG") {
            self.host_org = Some(v);
        }
        if let Some(v) = get("ARCHITECT_ENROL_AT") {
            self.enrol_at = Some(v);
        }
        if let Some(v) = get("ARCHITECT_ENROL_ORG") {
            self.enrol_org = Some(v);
        }
        if let Some(v) = get("ARCHITECT_ENROL_ROLE") {
            self.enrol_role = v.trim().to_ascii_lowercase();
        }
        if let Some(v) = get("ARCHITECT_PEERS") {
            self.peers = list(&v);
        }
        if let Some(v) = get("ARCHITECT_IROH") {
            self.iroh = match v.trim() {
                "n0" => IrohMode::N0,
                "local" => IrohMode::Local,
                other => {
                    return Err(ConfigError::Env {
                        var: "ARCHITECT_IROH",
                        why: format!("`{other}` is not `n0` or `local`"),
                    });
                }
            };
        }
        if let Some(v) = get("ARCHITECT_IROH_BIND") {
            self.iroh_bind = Some(v);
        }
        if let Some(v) = get("ARCHITECT_MCP_TOKEN_FILE") {
            self.mcp_token_file = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_SWEEP_SECS") {
            self.sweep_secs = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_SWEEP_SECS",
                why: format!("{e}"),
            })?;
        }
        if let Some(v) = get("ARCHITECT_LIVE_MAX") {
            self.live_max = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_LIVE_MAX",
                why: format!("{e}"),
            })?;
        }
        if let Some(v) = get("ARCHITECT_FILES_DIR") {
            self.files_dir = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_FILES_HOLD") {
            self.files_hold = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = get("ARCHITECT_DRIVE") {
            self.drive = (!v.trim().is_empty()).then(|| PathBuf::from(v.trim()));
        }
        if let Some(v) = get("ARCHITECT_FILES_SECS") {
            self.files_secs = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_FILES_SECS",
                why: format!("{e}"),
            })?;
        }
        if let Some(v) = get("ARCHITECT_S3_BIND") {
            self.s3_bind = Some(v);
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_BUCKET") {
            self.blob_s3_bucket = Some(v);
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_ENDPOINT") {
            self.blob_s3_endpoint = Some(v);
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_REGION") {
            self.blob_s3_region = v;
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_PREFIX") {
            self.blob_s3_prefix = v;
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_ACCESS_KEY_ID") {
            self.blob_s3_access_key_id = Some(v);
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_SECRET_FILE") {
            self.blob_s3_secret_file = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_BLOB_PLACEMENT") {
            self.blob_placement = match v.trim() {
                "replicate" => Placement::Replicate,
                "primary" => Placement::Primary,
                other => {
                    return Err(ConfigError::Env {
                        var: "ARCHITECT_BLOB_PLACEMENT",
                        why: format!("`{other}` is not `replicate` or `primary`"),
                    });
                }
            };
        }
        if let Some(v) = get("ARCHITECT_BLOB_CACHE_MB") {
            self.blob_cache_mb = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_BLOB_CACHE_MB",
                why: format!("{e}"),
            })?;
        }
        Ok(())
    }

    /// The issuer org (slug) the node's vaults are: the one it hosts,
    /// else the one it enrols in.
    #[must_use]
    pub fn vault_org(&self) -> Option<&str> {
        self.host_org.as_deref().or(self.enrol_org.as_deref())
    }

    /// Where vault `path` lives: under the data dir when relative.
    #[must_use]
    pub fn vault_root(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.data_dir.join(path)
        }
    }
}

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn pairs(var: &'static str, v: &str) -> Result<Vec<(String, String)>, ConfigError> {
    list(v)
        .into_iter()
        .map(|item| {
            item.split_once('=')
                .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .ok_or_else(|| ConfigError::Env {
                    var,
                    why: format!("`{item}` is not `key=value`"),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_wins_over_the_file() {
        let dir = std::env::temp_dir().join(format!("node-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let file = dir.join("architect.toml");
        std::fs::write(
            &file,
            "name = \"from-file\"\nbind = \"127.0.0.1:1\"\n[vaults]\nnotes = \"vaults/notes\"\n",
        )
        .ok();
        let env = |var: &str| match var {
            "ARCHITECT_BIND" => Some("0.0.0.0:9000".to_owned()),
            "ARCHITECT_SITES" => Some("/=notes, /docs=docs".to_owned()),
            "ARCHITECT_IROH" => Some("local".to_owned()),
            _ => None,
        };
        let config = Config::load(Some(&file), env).ok();
        let config = config.as_ref();
        assert_eq!(config.map(|c| c.name.as_str()), Some("from-file"));
        assert_eq!(config.map(|c| c.bind.as_str()), Some("0.0.0.0:9000"));
        assert_eq!(config.map(|c| c.iroh), Some(IrohMode::Local));
        assert_eq!(
            config
                .and_then(|c| c.sites.get("/docs"))
                .map(String::as_str),
            Some("docs")
        );
        assert_eq!(
            config.and_then(|c| c.vaults.get("notes").map(|p| c.vault_root(p))),
            Some(PathBuf::from("architect-data/vaults/notes"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bucket_and_its_placement_come_from_the_environment() {
        let env = |var: &str| match var {
            "ARCHITECT_BLOB_S3_BUCKET" => Some("architect-blobs".to_owned()),
            "ARCHITECT_BLOB_PLACEMENT" => Some("primary".to_owned()),
            "ARCHITECT_BLOB_CACHE_MB" => Some("512".to_owned()),
            "ARCHITECT_S3_BIND" => Some("127.0.0.1:9000".to_owned()),
            _ => None,
        };
        let config = Config::load(None, env).ok();
        let config = config.as_ref();
        assert_eq!(
            config.and_then(|c| c.blob_s3_bucket.as_deref()),
            Some("architect-blobs")
        );
        assert_eq!(config.map(|c| c.blob_placement), Some(Placement::Primary));
        assert_eq!(config.map(|c| c.blob_cache_mb), Some(512));
        assert_eq!(
            config.and_then(|c| c.s3_bind.as_deref()),
            Some("127.0.0.1:9000")
        );
        let bad = |var: &str| (var == "ARCHITECT_BLOB_PLACEMENT").then(|| "both".to_owned());
        assert!(Config::load(None, bad).is_err());
    }

    #[test]
    fn a_malformed_variable_is_refused() {
        let env = |var: &str| (var == "ARCHITECT_VAULTS").then(|| "notes".to_owned());
        assert!(Config::load(None, env).is_err());
    }
}
