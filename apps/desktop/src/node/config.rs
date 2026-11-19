//! What a node is told: its defaults, a TOML file over them, then the
//! environment over both.
//!
//! The app runs its node in the window and without it (`serve`) from the
//! same configuration; only the defaults differ ([`Config::load_with`]).
//! In the window they suit a person's machine ([`app_defaults`]): HTTP on
//! loopback, its state in the app's folder, its platform the machine's.
//! Run with `serve`, they are the server's in the table below.
//!
//! `--config <file>` (or `$ARCHITECT_CONFIG`) names the file; every
//! `ARCHITECT_*` variable below then wins over the file's value, so a
//! container sets what differs per deployment and mounts the rest.
//! Secrets are never values here: the account's session and the MCP
//! bearer are **files** a secret is mounted at.
//!
//! | Variable | File key | Default (`serve`) | What |
//! |---|---|---|---|
//! | `ARCHITECT_NAME` | `name` | `architect` | the device's name on the account |
//! | `ARCHITECT_PLATFORM` | `platform` | `server` | the device's platform on the account (in the window: the machine's, `linux`, `macos`, `windows`) |
//! | `ARCHITECT_BIND` | `bind` | `0.0.0.0:8080` | HTTP: `/health`, Sites, `/mcp`, the HTTP face, `/vox` (in the window: `127.0.0.1:7420`, or any free loopback port when that one is taken) |
//! | `ARCHITECT_DATA_DIR` | `data_dir` | `architect-data` | the node's state: its iroh key, device id, documents, built sites (in the window: `<app dir>/node`) |
//! | `ARCHITECT_VAULTS` | `[vaults]` | none | `id=path,…`: the vaults it holds (a relative path is under the data dir) |
//! | `ARCHITECT_SITES` | `[sites]` | none | `/mount=vault,…`: vaults published as Sites |
//! | `ARCHITECT_EXTENSIONS` | `extensions` | `<data dir>/extensions` | installed extensions, a package folder each (`<id>/extension.toml`): the fences they claim are drawn on share links and Sites (a relative path is under the data dir) |
//! | `ARCHITECT_EXTENSION_SECRETS` | `extension_secrets` | none | its extensions' node secrets, set by its operator: a folder with a file each, `<ext id>/<secret id>` (as sops-nix writes them); none: an org admin sets them on the org lane (`NodeActions::set_secret`), kept in `<data dir>/extension-secrets.json`. The node runs installed extensions' node actions (`runs = "node"`, #583) on its org lane with them |
//! | `ARCHITECT_ISSUER` | `issuer` | none | the account server (`https://auth.fasttrackstudio.app`) |
//! | `ARCHITECT_SESSION_FILE` | `session_file` | none | a file holding the account's session token (in the window, with neither this nor `issuer` set: the app's signed-in account) |
//! | `ARCHITECT_ORG` | `org` | none | the org (slug) it offers the account when the account shares none yet |
//! | `ARCHITECT_HOST_ORG` | `host_org` | none | an issuer org (slug) of the session's: the node serves its enrolment lane, `/org/{slug}/vox`, and its vaults are that org's |
//! | `ARCHITECT_HOST_ORGS` | `host_orgs` | none | several issuer orgs (slugs) of the session's, `a,b`, or `*` for every org the session is in: one process serves them all, each on its own lane `/org/{slug}/vox` on the one origin (#538). A vault is the org's whose slug is its id, or the one `vault_orgs` names |
//! | `ARCHITECT_VAULT_ORGS` | `[vault_orgs]` | none | `vault=org,…`: the org (slug) a vault is, when its id is not the org's slug. With one org and none named, every vault is that org's |
//! | `ARCHITECT_LANE_RECHECK_SECS` | `lane_recheck_secs` | `30` | on the org lane, how long a person's bearer is trusted before the issuer is asked again, and how often an open note's live session checks it still is: signed out, expired or removed from the org, the session ends within about twice this |
//! | `ARCHITECT_LANE_ISSUER_GRACE_SECS` | `lane_issuer_grace_secs` | `300` | on the org lane, how long an open note's live session is kept while the issuer cannot be reached, counted from the issuer's last answer for its bearer; then it ends (a refusal ends it at once, and new calls are refused while the issuer is unreachable) |
//! | `ARCHITECT_LANE_ORIGINS` | `lane_origins` | none | `https://app.example,…`: browser origins besides the node's own that may open its org lane (a WebSocket upgrade from any other `Origin` is refused; one with no `Origin`, a native client's, is let through to the bearer check) |
//! | `ARCHITECT_ENROL_AT` | `enrol_at` | none | another node's HTTP origin: this node enrols itself in `enrol_org` there, once |
//! | `ARCHITECT_ENROL_ORG` | `enrol_org` | none | the issuer org (slug) to enrol in there; its vaults are that org's |
//! | `ARCHITECT_ENROL_ORGS` | `enrol_orgs` | none | several orgs (slugs) to enrol in at `enrol_at`, `a,b`: a storage server of more than one org on a node that hosts them all |
//! | `ARCHITECT_ENROL_ROLE` | `enrol_role` | `member` | the role it asks for: at most the session's own in the org |
//! | `ARCHITECT_WEB_APP` | `web_app` | none | whether the web app is served beside the node, on its origin: `beside` (a proxy serves the same release's web bundle at `/app/`, as the chart's web front end does), or the folder of that bundle (checked at start for `app/index.html` and `app/start.js`). A share link's page (`/s/<code>/`) is the web app's page, which loads it from there; none: it says the link opens in the web app, which this server does not serve (#646) |
//! | `ARCHITECT_APPLE_APP_IDS` | `apple_app_ids` | none | `<team id>.<bundle id>,…` (`28C2G63DA7.app.fasttrackstudio.architect`): the iOS apps that open this node's share links (`/s/*`) as universal links, named in `/.well-known/apple-app-site-association`; none: that file is not served (#629) |
//! | `ARCHITECT_PEERS` | `peers` | none | `endpoint@host:port,…`: nodes dialled directly (no lookup) |
//! | `ARCHITECT_IROH` | `iroh` | `n0` | `n0` (relays and lookup) or `local` (loopback, no relay: tests) |
//! | `ARCHITECT_IROH_BIND` | `iroh_bind` | any port | the iroh UDP socket (`0.0.0.0:7777`, for a fixed port) |
//! | `ARCHITECT_MCP_TOKEN_FILE` | `mcp_token_file` | none | `/mcp` is served only with a bearer, read from this file |
//! | `ARCHITECT_SWEEP_SECS` | `sweep_secs` | `30` | how often it sweeps its peers: the fallback for what a live bridge missed |
//! | `ARCHITECT_LIVE_MAX` | `live_max` | `256` | how many notes it keeps live with each peer (a `DocSync` bridge each); `0` sweeps only |
//! | `ARCHITECT_FILES_DIR` | `files_dir` | none | projects and asset libraries (`<org>/Projects/<p>`, `<org>/Assets/<kind>`): the tree held in place, or where pins go (`architect_files::sync`) |
//! | `ARCHITECT_FILES_HOLD` | `files_hold` | `false` | hold every root found in `files_dir` (the always-on node); otherwise only pinned ones have a working copy |
//! | `ARCHITECT_FILES_HOLD_PATHS` | `files_hold_paths` | none | `glob,…` (root-relative; `*`, `?`, `**`): with any set, only the files one of them matches are fetched, held and copied to the bucket here (#904). The rest stay listed, their bytes on the holders that keep them (a storage server) |
//! | `ARCHITECT_FILES_SKIP_PATHS` | `files_skip_paths` | none | `glob,…`: files never fetched or held here, whatever `files_hold_paths` says (`**/Media/**,**/Audio Files/**`: a project's multitracks) |
//! | `ARCHITECT_FILES_HOLD_MAX` | `files_hold_max` | none | never fetch or hold a single file over this ([`Size`] in bytes: `256Mi`) |
//! | `ARCHITECT_DRIVE` | `drive` | none | mount every root this device knows here, as a real folder (FUSE; `~/Architect` when the agent is installed, `architect-desktop agent install`); empty is off |
//! | `ARCHITECT_FILES_SECS` | `files_secs` | `2` | how often each root is scanned and brought to its heads |
//! | `ARCHITECT_BLOB_S3_BUCKET` | `blob_s3_bucket` | none | the files' bytes in an S3 bucket too (Garage, MinIO, R2, AWS, Ceph RGW): turns the backend on |
//! | `ARCHITECT_BLOB_S3_ENDPOINT` | `blob_s3_endpoint` | AWS | the S3 endpoint (`http://garage.lan:3900`) |
//! | `ARCHITECT_BLOB_S3_REGION` | `blob_s3_region` | `us-east-1` | its region (`garage`, `auto` for R2) |
//! | `ARCHITECT_BLOB_S3_PREFIX` | `blob_s3_prefix` | `architect` | keys go under it: `<prefix>/sha256/<hh>/<hex>` |
//! | `ARCHITECT_BLOB_S3_ACCESS_KEY_ID` | `blob_s3_access_key_id` | none | the bucket's access key id |
//! | `ARCHITECT_BLOB_S3_SECRET_FILE` | `blob_s3_secret_file` | none | a file holding its secret key (or `ARCHITECT_BLOB_S3_SECRET_ACCESS_KEY`, read at start and never kept in the config) |
//! | `ARCHITECT_BLOB_S3_MAX` | `blob_s3_max` | none | never copy a single file over this to the bucket ([`Size`] in bytes): on a storage server that shares the bucket, the small files go to it and the large ones stay on the server |
//! | `ARCHITECT_BLOB_S3_SKIP_PATHS` | `blob_s3_skip_paths` | none | `glob,…`: files never copied to the bucket (still held here) |
//! | `ARCHITECT_BLOB_PLACEMENT` | `blob_placement` | `replicate` | `replicate`: every byte here and in the bucket; `primary`: the bucket holds them, this node keeps a cache |
//! | `ARCHITECT_BLOB_CACHE_MB` | `blob_cache_mb` | `10240` | with `primary`, how much of the store is kept here (working copies are never evicted) |
//! | `ARCHITECT_S3_BIND` | `s3_bind` | none | the roots over S3 (`127.0.0.1:9000`): a bucket per org, keys from `serve s3-key issue` (needs `files_dir`) |
//! | `ARCHITECT_FILES_SITE` | `files_site` | the name | where this holder is (`home`, `starcommand`): copies at one site are not off-site |
//! | `ARCHITECT_FILES_VERIFY_SECS` | `files_verify_secs` | `2592000` (30 days) | how long a copy's evidence (a scrub, a passed challenge) counts |
//! | `ARCHITECT_FILES_SCRUB_MIB` | `files_scrub_mib` | `32` | MiB re-hashed per pass at most (0: only on `FILES_SCRUB`) |
//! | `ARCHITECT_FILES_STORE_MAX` | `files_store_max` | a quarter of the filesystem holding the store, at most 100 GiB | the hard limit on `<data dir>/files/blobs` ([`Size`]: `20000000000`, `16Gi`, `40%`): a write past it is refused before a byte lands, a fetch waits until there is room |
//! | `ARCHITECT_FILES_KEEP_FREE` | `files_keep_free` | 15% of that filesystem, at most 64 GiB | the store never writes into this floor of free space (`statvfs`), whoever else filled the disk; on a holding node, also the floor kept on the tree's filesystem. If the free space cannot be read, writes that need the floor are refused |
//! | `ARCHITECT_FILES_HISTORY_MAX` | `files_history_max` | a tenth of the tree's filesystem, at most 500 GiB | on a holding node, how much of the replaced versions kept beside the tree (`<files_dir>/.architect/versions`) may hold; at the limit, or at the tree filesystem's floor, a replacement waits and is reported, nothing is deleted |

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
    pub platform: String,
    pub bind: String,
    pub data_dir: PathBuf,
    pub vaults: BTreeMap<String, PathBuf>,
    pub sites: BTreeMap<String, String>,
    pub extensions: Option<PathBuf>,
    pub extension_secrets: Option<PathBuf>,
    pub issuer: Option<String>,
    pub session_file: Option<PathBuf>,
    pub org: Option<String>,
    pub host_org: Option<String>,
    pub host_orgs: Vec<String>,
    pub vault_orgs: BTreeMap<String, String>,
    pub enrol_orgs: Vec<String>,
    pub enrol_at: Option<String>,
    pub enrol_org: Option<String>,
    pub enrol_role: String,
    pub peers: Vec<String>,
    pub apple_app_ids: Vec<String>,
    pub web_app: Option<String>,
    pub iroh: IrohMode,
    pub iroh_bind: Option<String>,
    pub mcp_token_file: Option<PathBuf>,
    pub sweep_secs: u64,
    pub live_max: usize,
    pub lane_recheck_secs: u64,
    pub lane_issuer_grace_secs: u64,
    pub lane_origins: Vec<String>,
    pub files_dir: Option<PathBuf>,
    pub files_hold: bool,
    pub files_hold_paths: Vec<String>,
    pub files_skip_paths: Vec<String>,
    pub files_hold_max: Option<Size>,
    pub files_secs: u64,
    pub drive: Option<PathBuf>,
    pub s3_bind: Option<String>,
    pub blob_s3_bucket: Option<String>,
    pub blob_s3_endpoint: Option<String>,
    pub blob_s3_region: String,
    pub blob_s3_prefix: String,
    pub blob_s3_access_key_id: Option<String>,
    pub blob_s3_secret_file: Option<PathBuf>,
    pub blob_s3_max: Option<Size>,
    pub blob_s3_skip_paths: Vec<String>,
    pub blob_placement: Placement,
    pub blob_cache_mb: u64,
    pub files_site: Option<String>,
    pub files_verify_secs: u64,
    pub files_scrub_mib: u64,
    pub files_store_max: Option<Size>,
    pub files_keep_free: Option<Size>,
    pub files_history_max: Option<Size>,
}

/// A size given as bytes (`20000000000`, `16Gi`, `500M`) or as a share
/// of the filesystem it is about (`40%`). `K`/`M`/`G`/`T` are powers of
/// 1000, `Ki`/`Mi`/`Gi`/`Ti` powers of 1024; a trailing `B` is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "SizeRaw")]
pub enum Size {
    Bytes(u64),
    Percent(u8),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SizeRaw {
    Number(u64),
    Text(String),
}

impl TryFrom<SizeRaw> for Size {
    type Error = String;

    fn try_from(raw: SizeRaw) -> Result<Self, String> {
        match raw {
            SizeRaw::Number(n) => Ok(Self::Bytes(n)),
            SizeRaw::Text(t) => t.parse(),
        }
    }
}

impl std::str::FromStr for Size {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if let Some(p) = s.strip_suffix('%') {
            return match p.trim().parse::<u8>() {
                Ok(p) if p <= 100 => Ok(Self::Percent(p)),
                _ => Err(format!("`{s}` is not a percentage from 0 to 100")),
            };
        }
        let end = s
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(s.len());
        let (number, unit) = s.split_at(end);
        let bad = || format!("`{s}` is not a size (bytes, `16Gi`, `1.5G`, `40%`)");
        let (whole, frac) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty() || frac.contains('.') {
            return Err(bad());
        }
        let whole: u64 = whole.parse().map_err(|_| bad())?;
        let unit = unit.trim().to_ascii_lowercase();
        let unit = unit.strip_suffix('b').unwrap_or(&unit);
        let scale: u64 = match unit {
            "" => 1,
            "k" => 1_000,
            "m" => 1_000_000,
            "g" => 1_000_000_000,
            "t" => 1_000_000_000_000,
            "ki" => 1 << 10,
            "mi" => 1 << 20,
            "gi" => 1 << 30,
            "ti" => 1 << 40,
            _ => return Err(format!("`{s}`: unknown unit `{unit}`")),
        };
        // A fraction of the unit, to the byte (`1.5Gi`).
        let frac_bytes = if frac.is_empty() {
            0
        } else {
            let digits = u32::try_from(frac.len()).map_err(|_| bad())?;
            let den = 10u64.checked_pow(digits).ok_or_else(bad)?;
            let num: u64 = frac.parse().map_err(|_| bad())?;
            u64::try_from(
                u128::from(num)
                    .saturating_mul(u128::from(scale))
                    .checked_div(u128::from(den))
                    .unwrap_or(0),
            )
            .map_err(|_| bad())?
        };
        whole
            .checked_mul(scale)
            .and_then(|n| n.checked_add(frac_bytes))
            .map(Self::Bytes)
            .ok_or_else(|| format!("`{s}` is too large"))
    }
}

impl Size {
    /// In bytes, on a filesystem of `total` bytes (needed for a share).
    #[must_use]
    pub fn of(self, total: Option<u64>) -> Option<u64> {
        match self {
            Self::Bytes(n) => Some(n),
            Self::Percent(p) => total.map(|t| (t / 100).saturating_mul(u64::from(p))),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "architect".to_owned(),
            platform: "server".to_owned(),
            bind: "0.0.0.0:8080".to_owned(),
            data_dir: PathBuf::from("architect-data"),
            vaults: BTreeMap::new(),
            sites: BTreeMap::new(),
            extensions: None,
            extension_secrets: None,
            issuer: None,
            session_file: None,
            org: None,
            host_org: None,
            host_orgs: Vec::new(),
            vault_orgs: BTreeMap::new(),
            enrol_orgs: Vec::new(),
            enrol_at: None,
            enrol_org: None,
            enrol_role: "member".to_owned(),
            peers: Vec::new(),
            apple_app_ids: Vec::new(),
            web_app: None,
            iroh: IrohMode::N0,
            iroh_bind: None,
            mcp_token_file: None,
            sweep_secs: 30,
            live_max: 256,
            lane_recheck_secs: 30,
            lane_issuer_grace_secs: 300,
            lane_origins: Vec::new(),
            files_dir: None,
            files_hold: false,
            files_hold_paths: Vec::new(),
            files_skip_paths: Vec::new(),
            files_hold_max: None,
            files_secs: 2,
            drive: None,
            s3_bind: None,
            blob_s3_bucket: None,
            blob_s3_endpoint: None,
            blob_s3_region: String::new(),
            blob_s3_prefix: "architect".to_owned(),
            blob_s3_access_key_id: None,
            blob_s3_secret_file: None,
            blob_s3_max: None,
            blob_s3_skip_paths: Vec::new(),
            blob_placement: Placement::Replicate,
            blob_cache_mb: 10_240,
            files_site: None,
            files_verify_secs: 30 * 24 * 3600,
            files_scrub_mib: 32,
            files_store_max: None,
            files_keep_free: None,
            files_history_max: None,
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
    #[error("the defaults: {0}")]
    Defaults(String),
}

/// Where the window's node listens unless told otherwise: loopback, so
/// nothing off the machine reaches it until the person says so.
pub const APP_BIND: &str = "127.0.0.1:7420";

/// The window's defaults, under [`Config::load_with`].
///
/// The device named `name` with the machine's platform, HTTP on loopback
/// ([`APP_BIND`]), its state in `<app_dir>/node`. Everything public (Sites, share links'
/// web app, a bind off loopback, an org it hosts) stays off until the
/// configuration turns it on, as on a server.
#[must_use]
pub fn app_defaults(app_dir: &Path, name: &str) -> toml::Table {
    let mut table = toml::Table::new();
    table.insert("name".to_owned(), name.into());
    table.insert("platform".to_owned(), std::env::consts::OS.into());
    table.insert("bind".to_owned(), APP_BIND.into());
    table.insert(
        "data_dir".to_owned(),
        app_dir.join("node").display().to_string().into(),
    );
    table
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
        Self::load_with(toml::Table::new(), file, env)
    }

    /// [`load`](Self::load) over `defaults` instead of the server's: each
    /// key `defaults` sets (a file key) holds unless the file or the
    /// environment sets it ([`app_defaults`]: the window's).
    ///
    /// # Errors
    ///
    /// As [`load`](Self::load); `defaults` not this shape is
    /// [`ConfigError::Defaults`].
    pub fn load_with(
        defaults: toml::Table,
        file: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let mut table = defaults;
        if let Some(path) = file {
            table.extend(Self::table_of(path)?);
        }
        let mut config: Self =
            toml::Value::Table(table)
                .try_into()
                .map_err(|source| match file {
                    Some(path) => ConfigError::Parse {
                        path: path.to_owned(),
                        source: Box::new(source),
                    },
                    None => ConfigError::Defaults(source.to_string()),
                })?;
        config.apply_env(env)?;
        config.check_store()?;
        Ok(config)
    }

    fn table_of(path: &Path) -> Result<toml::Table, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source: Box::new(source),
        })
    }

    /// Refuse a store budget that would refuse every write: a limit of
    /// nothing, or a floor of the whole filesystem.
    fn check_store(&self) -> Result<(), ConfigError> {
        let zero = |s: Option<Size>| matches!(s, Some(Size::Bytes(0) | Size::Percent(0)));
        if zero(self.files_store_max) {
            return Err(ConfigError::Env {
                var: "ARCHITECT_FILES_STORE_MAX",
                why: "0 refuses every write into the store; leave it unset for the default"
                    .to_owned(),
            });
        }
        if zero(self.files_history_max) {
            return Err(ConfigError::Env {
                var: "ARCHITECT_FILES_HISTORY_MAX",
                why: "0 keeps no replaced version, so no replacement can happen; leave it unset to keep them all".to_owned(),
            });
        }
        for (var, size) in [
            ("ARCHITECT_FILES_HOLD_MAX", self.files_hold_max),
            ("ARCHITECT_BLOB_S3_MAX", self.blob_s3_max),
        ] {
            if matches!(size, Some(Size::Percent(_))) {
                return Err(ConfigError::Env {
                    var,
                    why: "a file's size in bytes (`256Mi`), not a share of a filesystem".to_owned(),
                });
            }
        }
        if matches!(self.files_keep_free, Some(Size::Percent(100))) {
            return Err(ConfigError::Env {
                var: "ARCHITECT_FILES_KEEP_FREE",
                why: "100% keeps the whole filesystem free: nothing could be written".to_owned(),
            });
        }
        Ok(())
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
        if let Some(v) = get("ARCHITECT_PLATFORM") {
            self.platform = v;
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
        if let Some(v) = get("ARCHITECT_HOST_ORGS") {
            self.host_orgs = list(&v);
        }
        if let Some(v) = get("ARCHITECT_VAULT_ORGS") {
            self.vault_orgs = pairs("ARCHITECT_VAULT_ORGS", &v)?.into_iter().collect();
        }
        if let Some(v) = get("ARCHITECT_ENROL_ORGS") {
            self.enrol_orgs = list(&v);
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
        if let Some(v) = get("ARCHITECT_APPLE_APP_IDS") {
            self.apple_app_ids = list(&v);
        }
        if let Some(v) = get("ARCHITECT_WEB_APP") {
            self.web_app = Some(v.trim().to_owned()).filter(|v| !v.is_empty());
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
        if let Some(v) = get("ARCHITECT_LANE_RECHECK_SECS") {
            self.lane_recheck_secs = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_LANE_RECHECK_SECS",
                why: format!("{e}"),
            })?;
        }
        if let Some(v) = get("ARCHITECT_LANE_ISSUER_GRACE_SECS") {
            self.lane_issuer_grace_secs = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_LANE_ISSUER_GRACE_SECS",
                why: format!("{e}"),
            })?;
        }
        if let Some(v) = get("ARCHITECT_LANE_ORIGINS") {
            self.lane_origins = v
                .split(',')
                .map(|o| o.trim().trim_end_matches('/').to_owned())
                .filter(|o| !o.is_empty())
                .collect();
        }
        if let Some(v) = get("ARCHITECT_LIVE_MAX") {
            self.live_max = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_LIVE_MAX",
                why: format!("{e}"),
            })?;
        }
        if let Some(v) = get("ARCHITECT_EXTENSIONS") {
            self.extensions = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_EXTENSION_SECRETS") {
            self.extension_secrets = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_FILES_DIR") {
            self.files_dir = Some(PathBuf::from(v));
        }
        if let Some(v) = get("ARCHITECT_FILES_HOLD") {
            self.files_hold = matches!(v.trim(), "1" | "true" | "yes");
        }
        if let Some(v) = get("ARCHITECT_FILES_HOLD_PATHS") {
            self.files_hold_paths = list(&v);
        }
        if let Some(v) = get("ARCHITECT_FILES_SKIP_PATHS") {
            self.files_skip_paths = list(&v);
        }
        if let Some(v) = get("ARCHITECT_BLOB_S3_SKIP_PATHS") {
            self.blob_s3_skip_paths = list(&v);
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
        if let Some(v) = get("ARCHITECT_FILES_SITE") {
            self.files_site = Some(v.trim().to_owned());
        }
        if let Some(v) = get("ARCHITECT_FILES_VERIFY_SECS") {
            self.files_verify_secs = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_FILES_VERIFY_SECS",
                why: format!("{e}"),
            })?;
        }
        for (var, field) in [
            ("ARCHITECT_FILES_STORE_MAX", &mut self.files_store_max),
            ("ARCHITECT_FILES_KEEP_FREE", &mut self.files_keep_free),
            ("ARCHITECT_FILES_HISTORY_MAX", &mut self.files_history_max),
            ("ARCHITECT_FILES_HOLD_MAX", &mut self.files_hold_max),
            ("ARCHITECT_BLOB_S3_MAX", &mut self.blob_s3_max),
        ] {
            if let Some(v) = get(var).filter(|v| !v.trim().is_empty()) {
                *field = Some(v.parse().map_err(|why| ConfigError::Env { var, why })?);
            }
        }
        if let Some(v) = get("ARCHITECT_FILES_SCRUB_MIB") {
            self.files_scrub_mib = v.trim().parse().map_err(|e| ConfigError::Env {
                var: "ARCHITECT_FILES_SCRUB_MIB",
                why: format!("{e}"),
            })?;
        }
        Ok(())
    }

    /// The issuer org (slug) the node's vaults are: the one it hosts,
    /// else the one it enrols in. With several ([`Self::hosted`],
    /// [`Self::enrolling`]) the first; which vault is whose is
    /// [`Self::org_of_vault`].
    #[must_use]
    pub fn vault_org(&self) -> Option<&str> {
        self.host_org
            .as_deref()
            .or_else(|| {
                self.host_orgs
                    .iter()
                    .map(String::as_str)
                    .find(|o| *o != ALL_ORGS)
            })
            .or(self.enrol_org.as_deref())
            .or_else(|| self.enrol_orgs.first().map(String::as_str))
    }

    /// Which files of its roots it fetches and holds (#904):
    /// [`Self::files_hold_paths`], [`Self::files_skip_paths`],
    /// [`Self::files_hold_max`]. With none set, everything.
    #[must_use]
    pub fn hold_rule(&self) -> architect_files::HoldRule {
        architect_files::HoldRule {
            paths: self.files_hold_paths.clone(),
            skip: self.files_skip_paths.clone(),
            max: self.files_hold_max.and_then(|size| size.of(None)),
        }
    }

    /// Which of the files it holds it copies to its bucket:
    /// [`Self::blob_s3_max`], [`Self::blob_s3_skip_paths`], within
    /// [`Self::hold_rule`].
    #[must_use]
    pub fn bucket_rule(&self) -> architect_files::HoldRule {
        architect_files::HoldRule {
            paths: Vec::new(),
            skip: self.blob_s3_skip_paths.clone(),
            max: self.blob_s3_max.and_then(|size| size.of(None)),
        }
    }

    /// The orgs it hosts: [`Self::host_org`] and [`Self::host_orgs`].
    #[must_use]
    pub fn hosted(&self) -> Hosted {
        if self.host_orgs.iter().any(|o| o == ALL_ORGS) {
            return Hosted::All;
        }
        Hosted::Named(distinct(
            self.host_org.iter().chain(&self.host_orgs).cloned(),
        ))
    }

    /// The orgs it enrols itself in at [`Self::enrol_at`]:
    /// [`Self::enrol_org`] and [`Self::enrol_orgs`].
    #[must_use]
    pub fn enrolling(&self) -> Vec<String> {
        distinct(self.enrol_org.iter().chain(&self.enrol_orgs).cloned())
    }

    /// The org (slug), of `orgs` (the ones this node is in), that vault
    /// `vault` is: the one [`Self::vault_orgs`] names, else the one whose
    /// slug is the vault's id, else the only one.
    #[must_use]
    pub fn org_of_vault<'a>(&'a self, vault: &str, orgs: &'a [String]) -> Option<&'a str> {
        if let Some(named) = self.vault_orgs.get(vault) {
            return orgs.iter().map(String::as_str).find(|o| *o == named);
        }
        if let Some(same) = orgs.iter().find(|o| o.as_str() == vault) {
            return Some(same);
        }
        match orgs {
            [only] => Some(only),
            _ => None,
        }
    }

    /// Where the extensions it draws blocks with are installed.
    #[must_use]
    pub fn extensions_dir(&self) -> PathBuf {
        self.extensions.as_deref().map_or_else(
            || self.data_dir.join("extensions"),
            |path| self.vault_root(path),
        )
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

/// [`Config::host_orgs`]' entry for every org the session is in.
pub const ALL_ORGS: &str = "*";

/// Which orgs a node hosts ([`Config::hosted`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hosted {
    /// Every org its session is in, asked of the issuer at start.
    All,
    /// These (slugs); none: it hosts no org.
    Named(Vec<String>),
}

impl Hosted {
    /// Whether it hosts `slug`.
    #[must_use]
    pub fn has(&self, slug: &str) -> bool {
        match self {
            Self::All => true,
            Self::Named(orgs) => orgs.iter().any(|o| o == slug),
        }
    }

    /// Whether it hosts any org.
    #[must_use]
    pub fn any(&self) -> bool {
        !matches!(self, Self::Named(orgs) if orgs.is_empty())
    }
}

fn distinct(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if !item.trim().is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out
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
    fn the_windows_defaults_hold_unless_the_file_or_environment_says_otherwise() {
        let app = Path::new("/home/someone/.config/architect");
        let none = |_: &str| None;
        let config = Config::load_with(app_defaults(app, "laptop"), None, none).ok();
        let config = config.as_ref();
        assert_eq!(config.map(|c| c.bind.as_str()), Some(APP_BIND));
        assert_eq!(config.map(|c| c.name.as_str()), Some("laptop"));
        assert_eq!(
            config.map(|c| c.platform.as_str()),
            Some(std::env::consts::OS)
        );
        assert_eq!(config.map(|c| c.data_dir.clone()), Some(app.join("node")));
        assert!(config.is_some_and(|c| c.sites.is_empty() && c.host_org.is_none()));

        let dir = std::env::temp_dir().join(format!("node-app-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let file = dir.join("architect.toml");
        std::fs::write(&file, "name = \"from-file\"\n").ok();
        let env = |var: &str| (var == "ARCHITECT_PLATFORM").then(|| "server".to_owned());
        let config = Config::load_with(app_defaults(app, "laptop"), Some(&file), env).ok();
        let config = config.as_ref();
        assert_eq!(config.map(|c| c.name.as_str()), Some("from-file"));
        assert_eq!(config.map(|c| c.platform.as_str()), Some("server"));
        assert_eq!(config.map(|c| c.bind.as_str()), Some(APP_BIND));
        std::fs::remove_dir_all(&dir).ok();

        let served = Config::load(None, none).ok();
        assert_eq!(served.map(|c| c.platform), Some("server".to_owned()));
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
    fn the_store_budget_takes_bytes_units_and_shares() {
        let env = |var: &str| match var {
            "ARCHITECT_FILES_STORE_MAX" => Some("16Gi".to_owned()),
            "ARCHITECT_FILES_KEEP_FREE" => Some("15%".to_owned()),
            _ => None,
        };
        let config = Config::load(None, env).ok();
        let config = config.as_ref();
        assert_eq!(
            config.and_then(|c| c.files_store_max),
            Some(Size::Bytes(16 << 30))
        );
        assert_eq!(
            config.and_then(|c| c.files_keep_free),
            Some(Size::Percent(15))
        );
        assert_eq!(Size::Percent(15).of(Some(1_000)), Some(150));
        assert_eq!(Size::Percent(15).of(None), None);
        assert_eq!("500MB".parse(), Ok(Size::Bytes(500_000_000)));
        assert_eq!("1234".parse(), Ok(Size::Bytes(1234)));
        assert!("12 parsecs".parse::<Size>().is_err());
        assert!("150%".parse::<Size>().is_err());
        assert_eq!("1.5Gi".parse(), Ok(Size::Bytes(3 << 29)));
        assert_eq!("16gib".parse(), Ok(Size::Bytes(16 << 30)));
        assert_eq!("2.5k".parse(), Ok(Size::Bytes(2_500)));
        assert!("1..5G".parse::<Size>().is_err());
        for (var, value) in [
            ("ARCHITECT_FILES_STORE_MAX", "lots"),
            ("ARCHITECT_FILES_STORE_MAX", "0"),
            ("ARCHITECT_FILES_STORE_MAX", "0%"),
            ("ARCHITECT_FILES_KEEP_FREE", "100%"),
            ("ARCHITECT_FILES_HISTORY_MAX", "0"),
        ] {
            let bad = |v: &str| (v == var).then(|| value.to_owned());
            assert!(Config::load(None, bad).is_err(), "{var}={value}");
        }
    }

    /// #904: the hosted node's rule, and a storage server's for the
    /// bucket it shares.
    #[test]
    fn a_holding_rule_and_a_bucket_rule_come_from_the_environment() {
        let env = |var: &str| match var {
            "ARCHITECT_FILES_SKIP_PATHS" => Some("**/Media/**, **/Audio Files/**".to_owned()),
            "ARCHITECT_FILES_HOLD_MAX" => Some("256Mi".to_owned()),
            "ARCHITECT_BLOB_S3_MAX" => Some("64Mi".to_owned()),
            _ => None,
        };
        let config = Config::load(None, env).unwrap_or_default();
        let hold = config.hold_rule();
        assert!(hold.keeps("songs/joy/chart.kf", 4_000));
        assert!(!hold.keeps("songs/joy/Audio Files/kick.wav", 4_000));
        assert!(!hold.keeps("songs/joy/film.mov", (256 << 20) + 1));
        let bucket = config.bucket_rule();
        assert!(bucket.keeps("songs/joy/reference.mp3", 64 << 20));
        assert!(!bucket.keeps("songs/joy/reference.wav", (64 << 20) + 1));
        assert!(Config::default().hold_rule().is_all());
        assert!(Config::default().bucket_rule().is_all());
        let share = |var: &str| (var == "ARCHITECT_FILES_HOLD_MAX").then(|| "10%".to_owned());
        assert!(Config::load(None, share).is_err(), "a size, not a share");
    }

    #[test]
    fn one_node_hosts_several_orgs_and_knows_whose_each_vault_is() {
        let env = |var: &str| match var {
            "ARCHITECT_HOST_ORG" => Some("fasttrackstudio".to_owned()),
            "ARCHITECT_HOST_ORGS" => Some("fasttrackaudio, tombrooksmusic".to_owned()),
            "ARCHITECT_VAULT_ORGS" => Some("notes=fasttrackstudio".to_owned()),
            _ => None,
        };
        let config = Config::load(None, env).unwrap_or_default();
        let orgs: Vec<String> = ["fasttrackstudio", "fasttrackaudio", "tombrooksmusic"]
            .map(str::to_owned)
            .into();
        assert_eq!(config.hosted(), Hosted::Named(orgs.clone()));
        assert_eq!(config.vault_org(), Some("fasttrackstudio"));
        // By its id, by the map, and none when neither says.
        assert_eq!(
            config.org_of_vault("tombrooksmusic", &orgs),
            Some("tombrooksmusic")
        );
        assert_eq!(config.org_of_vault("notes", &orgs), Some("fasttrackstudio"));
        assert_eq!(config.org_of_vault("scratch", &orgs), None);
        // One org: every vault is its, as before.
        let one = ["acme".to_owned()];
        assert_eq!(
            Config::default().org_of_vault("anything", &one),
            Some("acme")
        );

        let all = |var: &str| (var == "ARCHITECT_HOST_ORGS").then(|| "*".to_owned());
        let config = Config::load(None, all).unwrap_or_default();
        assert_eq!(config.hosted(), Hosted::All);
        assert!(config.hosted().has("whichever"));
        assert_eq!(config.vault_org(), None);
        assert!(!Config::default().hosted().any());
    }

    #[test]
    fn a_malformed_variable_is_refused() {
        let env = |var: &str| (var == "ARCHITECT_VAULTS").then(|| "notes".to_owned());
        assert!(Config::load(None, env).is_err());
    }
}
