//! Share links on the headless node, end to end (#374): the issuer in
//! process (`apps/auth-server`, as `tests/node.rs` runs it), a node that
//! hosts the org `indie` (Grace its owner, Alan a member; Ada is in
//! another org), its vault holding a shared folder and private notes the
//! shared ones link to.
//!
//! - A folder's share serves that folder, and nothing outside it: a link
//!   to a private note is plain text, its embed and attachment are not
//!   there, and asking for them is `404`.
//! - Each audience lets in its people and no one else: public and link
//!   for anyone; org for the org's members (bearer or cookie), signing in
//!   for nobody, refused for others; invited for the people named.
//! - An expired share is `404`; a revoke is `404` on the very next
//!   request.
//! - A live edit (a signed-in web client's `DocSync` on the org lane)
//!   is on the shared page within seconds, and its `_live` stream fires.
//! - A share that takes comments takes them; a read-only one refuses.
//! - A share with a domain of its own is served at that host's root.
//! - A share with two audiences (#510) serves each only its notes and
//!   files: the other audience's page and file are `404` on its link,
//!   nothing of them is listed there, a note that leaves an audience's
//!   filter is `404` there, and an audience is removed alone.
//! - A link's files page switches between the views (in order, by note,
//!   by a property), each with the audience's filter applied; its
//!   "Download all" zip, streamed by the node, holds exactly the
//!   audience's files, in list order, in folders by the view; one group
//!   downloads alone. `SHARE_AUDIENCE_ADD` / `SHARE_AUDIENCES` /
//!   `SHARE_ARCHIVE` / `SHARE_AUDIENCE_REMOVE` do the same from the app.
//! - The player (#511): a song's audio and video are native players
//!   streamed with ranges (`206`, `Content-Range`, `416` past the end),
//!   its `.lrc` attachment and an `lrc` fence are timed lines, and each
//!   page carries its link's queue in list order. An audience's link
//!   streams its own media and is `404` for the other's, ranged or not.
//! - From the app (`architect-accounts`' Action List, signed in as Grace):
//!   `SHARE_CREATE` makes a link that opens, `SHARE_LIST` lists it,
//!   `PUBLISH_DOMAIN` gives it a domain, `SHARE_REVOKE` takes it down.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::future_not_send
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use architect_accounts::Issuer;
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use architect_desktop::node::{self, Config, IrohMode, Running};
use architect_spaces::{
    Audience, Listen, NewAudience, NewShare, Share, ShareAccess, ShareChange, ShareLinksClient,
    ShareScope,
};
use auth_server::{ServerConfig, server};
use reqwest::StatusCode;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

async fn issuer() -> String {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    };
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    assert!(
        auth_server::dev::seed(&auth, &config.database_url)
            .await
            .unwrap()
    );
    let app = server::app_router_with_db(&config, auth, db).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await.unwrap() });
    origin
}

/// A session of `email`'s, written to `dir/session`; its token.
async fn session_of(origin: &str, dir: &Path, email: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let signed_in = Issuer::new(origin)
        .sign_in_password(email, auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    std::fs::write(dir.join("session"), &signed_in.session.token).unwrap();
    signed_in.session.token
}

fn touch(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// An extension installed on the node (`<data dir>/extensions`) whose
/// embed page draws a `chart` fence as an SVG that follows the playhead:
/// a transparent rect per bar with its time, as keyflow#59 engraves. It
/// also tries what must never reach a reader: a handler, a script, a
/// `<foreignObject>`, a `javascript:` link.
fn install_chart(pkg: &Path) {
    touch(
        pkg,
        "extension.toml",
        "id = \"demo.chart\"\nname = \"Chart\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
         [[embeds]]\nid = \"chart\"\npage = \"pages/chart.tsx\"\nclaims = { fence = [\"chart\"] }\n",
    );
    touch(
        pkg,
        "pages/chart.tsx",
        r#"import { h, useEmbed } from "architect";
export default function Chart() {
  const embed = useEmbed();
  const bars = (embed?.source ?? "").split("|").map((b) => b.trim()).filter((b) => b !== "");
  const w = 60;
  return (
    <svg class="demo-chart" viewBox={`0 0 ${bars.length * w} 40`} width={`${bars.length * w}`} height="40" data-follow-playhead="" onClick={() => {}}>
      {bars.map((b, i) => (
        <g key={`${i}`}>
          <rect class="demo-bar" data-at={`${i * 2}`} x={`${i * w}`} y="0" width={`${w}`} height="40" fill="transparent" />
          <text x={`${i * w + 8}`} y="26">{b}</text>
        </g>
      ))}
      <script>{"alert(1)"}</script>
      <foreignObject><div>html inside</div></foreignObject>
      <a href="javascript:alert(1)"><text>bad link</text></a>
    </svg>
  );
}
"#,
    );
}

struct World {
    host: Running,
    issuer: String,
    grace: String,
    alan: String,
    ada: String,
    tmp: tempfile::TempDir,
}

/// Grace's node hosting `indie`, its vault seeded; Alan's and Ada's sessions.
async fn world() -> World {
    world_with(architect_sites::Blocks::new()).await
}

/// [`world`], its node started by an embedder that hands it `blocks`.
async fn world_with(blocks: architect_sites::Blocks) -> World {
    world_full(blocks, Vec::new(), |_| {}).await
}

/// [`world`], its node drawing `islands` (#669), `seed` writing more
/// into its vault (the vault's folder) before it starts.
async fn world_full(
    blocks: architect_sites::Blocks,
    islands: Vec<architect_sites::SiteIsland>,
    seed: impl FnOnce(&Path),
) -> World {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("host");
    let vault = dir.join("data/vaults/notes");
    touch(
        &vault,
        "Shared/index.md",
        "---\ntitle: The album\n---\nRead [[Private/plan|the plan]] and [[track]].\n\n![[cover.png]]\n\n![[art.png]]\n",
    );
    touch(&vault, "Shared/track.md", "# Track\n\nMix notes.\n");
    touch(&vault, "Shared/art.png", "shared png");
    touch(&vault, "Private/plan.md", "# Plan\n\nThe secret plan.\n");
    touch(&vault, "Private/cover.png", "private png");
    // A setlist (#510): a chart per part, each attaching its file.
    for (n, song, part) in [
        (1, "Joy to the World", "violin"),
        (2, "Joy to the World", "cello"),
        (3, "Silent Night", "violin"),
        (4, "Noel", "cello"),
    ] {
        let slug = song.split(' ').next().unwrap().to_lowercase();
        touch(
            &vault,
            &format!("Set/0{n}-{slug}-{part}.md"),
            &format!(
                "---\ntitle: {song} ({part})\nsong: {song}\ninstrument: {part}\n---\nThe {part} part of {song}.\n\n![[charts/{slug}-{part}.pdf]]\n"
            ),
        );
        touch(
            &vault,
            &format!("Set/charts/{slug}-{part}.pdf"),
            &format!("%PDF {song} for {part}"),
        );
    }
    // Charts (#488): a ```chart fence in each, claimed by an extension
    // installed on the node.
    touch(
        &vault,
        "Charts/joy.md",
        "---\ntitle: Joy chart\ninstrument: violin\n---\n```chart\n| C | G | Am |\n```\n",
    );
    touch(
        &vault,
        "Charts/noel.md",
        "---\ntitle: Noel chart\ninstrument: cello\n---\n```chart\n| F | Bb | Dm7 |\n```\n",
    );
    install_chart(&dir.join("data/extensions/demo.chart"));
    // A fence only an embedder's own renderer claims (Studio's `kf`).
    touch(
        &vault,
        "Embedded/song.md",
        "---\ntitle: Song\n---\n```kf\n| D | A |\n```\n\n```chart\n| C |\n```\n",
    );
    seed(&vault);
    let grace = session_of(&origin, &dir, "grace@local.test").await;
    let config = Config {
        name: "indie-host".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        vaults: [("notes".to_owned(), PathBuf::from("vaults/notes"))].into(),
        issuer: Some(origin.clone()),
        session_file: Some(dir.join("session")),
        host_org: Some("indie".to_owned()),
        iroh: IrohMode::Local,
        sweep_secs: 3600,
        // The web app beside it, as production's chart serves it.
        web_app: Some("beside".to_owned()),
        ..Config::default()
    };
    let host = node::start_with_islands(config, blocks, Vec::new(), islands)
        .await
        .unwrap();
    let alan = session_of(&origin, &tmp.path().join("alan"), "alan@local.test").await;
    let ada = session_of(&origin, &tmp.path().join("ada"), "ada@local.test").await;
    World {
        host,
        issuer: origin,
        grace,
        alan,
        ada,
        tmp,
    }
}

impl World {
    async fn lane<C: vox::FromVoxLane>(&self, token: &str) -> C {
        let at =
            architect::connect::Endpoint::url(format!("ws://{}/org/indie/vox", self.host.http))
                .with_bearer(Some(token.to_owned()));
        architect::connect::Pool::new().client(&at).await.unwrap()
    }

    async fn share(&self, scope: ShareScope, audience: Audience, access: ShareAccess) -> Share {
        let links: ShareLinksClient = self.lane(&self.grace).await;
        links
            .create(NewShare {
                vault: "notes".to_owned(),
                scope,
                title: None,
                audience,
                access,
                expires_at: None,
                domain: None,
                listen: Listen::default(),
            })
            .await
            .unwrap()
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.host.http)
    }
}

/// GET `url`, with `auth` as the bearer (`Bearer …`) or a cookie
/// (`cookie:<token>`); status and body.
async fn get(url: &str, auth: Option<&str>) -> (StatusCode, reqwest::header::HeaderMap, String) {
    let mut req = reqwest::Client::new().get(url);
    match auth {
        Some(a) if a.starts_with("cookie:") => {
            let token = a.trim_start_matches("cookie:");
            req = req.header("cookie", format!("architect_session={token}"));
        }
        Some(token) => req = req.bearer_auth(token),
        None => {}
    }
    let res = req.send().await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.text().await.unwrap())
}

/// What a link's guest reads at page `url` (#646: the page itself is the
/// web app's): the note at that page through the link's guest service,
/// as its title, then each block's words, list items and media `src`, a
/// line each. None when no note of the link is at `url`, or the call is
/// refused.
async fn words_at(url: &str, auth: Option<&str>) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let origin = parsed.origin().ascii_serialization();
    let path = parsed.path().to_owned();
    let code = path.strip_prefix("/s/")?.split('/').next()?.to_owned();
    let token = auth
        .map(|a| a.trim_start_matches("cookie:").to_owned())
        .unwrap_or_default();
    let c = architect_spaces::guest::ShareGuestHttpClient::at(origin);
    let link = c.link(code.clone(), token.clone()).await.ok()?;
    let want = reqwest::Url::parse(url)
        .ok()
        .map(|u| percent_decoded(u.path()))?;
    // The link's root opens its root note (the app's `guest_view::open`):
    // the one at the root, else the first with a page.
    let at = |n: &&architect_spaces::ListedNote| {
        n.page
            .as_deref()
            .is_some_and(|p| p.trim_end_matches('/') == want.trim_end_matches('/'))
    };
    let root = want.trim_end_matches('/') == link.link.trim_end_matches('/');
    let note = link.notes.iter().find(at).or_else(|| {
        root.then(|| link.notes.iter().find(|n| n.page.is_some()))
            .flatten()
    })?;
    let read = c.read(code, token, note.path.clone()).await.ok()?;
    let mut out = format!("{}\n", read.title);
    for b in &read.blocks {
        out.push_str(&b.text);
        out.push('\n');
        for item in &b.items {
            out.push_str(item);
            out.push('\n');
        }
        if let Some(src) = &b.src {
            out.push_str(src);
            out.push('\n');
        }
        if let Some(info) = &b.info {
            out.push_str(info);
            out.push('\n');
        }
    }
    Some(out)
}

fn percent_decoded(path: &str) -> String {
    let mut out = Vec::new();
    let mut rest = path.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = tail
            .get(..2)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) if b == b'%' => {
                out.push(byte);
                rest = tail.get(2..).unwrap_or_default();
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page the share's build wrote at `page` (below the link, `joy/`)
/// for link `code` (the share's own, or `audience`'s), once it has
/// `want`: never served on the link (#646), the source of its guests'
/// blocks.
async fn built_page(
    w: &World,
    code: &str,
    audience: Option<&str>,
    page: &str,
    want: &str,
) -> String {
    let shares = w.tmp.path().join("host/data/shares").join(code);
    let site = audience.map_or_else(
        || shares.join("site"),
        |a| shares.join("audiences").join(a).join("site"),
    );
    let file = site.join(page).join("index.html");
    let mut last = String::new();
    for _ in 0..200 {
        if let Ok(html) = std::fs::read_to_string(&file) {
            if html.contains(want) {
                return html;
            }
            last = html;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{} never had `{want}`: {last}", file.display());
}

/// Whether a link's answer is the web app's page, as the node hands it
/// out for every page path (#646).
fn is_app_page(body: &str) -> bool {
    body.contains("<script type=\"module\" src=\"/app/start.js\"></script>")
}

/// Wait until page `url` answers 200 with the web app's page, and its
/// note, as the link's guest reads it ([`words_at`]), has `want` in it;
/// those words.
async fn page_with(url: &str, auth: Option<&str>, want: &str) -> String {
    let mut last = String::new();
    // A file of the link, not a page: its bytes.
    let file = !url.split(['?', '#']).next().unwrap_or(url).ends_with('/');
    for _ in 0..200 {
        let (status, _, body) = get(url, auth).await;
        if file && status == StatusCode::OK && body.contains(want) {
            return body;
        }
        if status == StatusCode::OK && is_app_page(&body) {
            // Only that the link answers the app's page.
            if want == "<!DOCTYPE html>" {
                return body;
            }
            if let Some(words) = words_at(url, auth).await {
                if words.contains(want) {
                    return words;
                }
                last = format!("{status}: words {words}");
            } else {
                last = format!("{status}: no note at the page");
            }
        } else {
            last = format!("{status}: {body}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{url} never had `{want}`; last {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_share_serves_the_folder_live_and_a_revoke_takes_it_down() {
    use crdt::sync::{DocSyncClient, SyncedDoc};
    let w = world().await;
    let share = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let link = format!("/s/{}/", share.code);

    // The folder, rooted at the link: its home note and its other note.
    let home = page_with(&w.url(&link), None, "The album").await;
    // Links are under `/s/` only (#431): the code at the root is not a
    // share, `/s/<code>` without its slash is sent to the link, and an
    // unknown code under `/s/` is 404 from the share layer.
    let (status, _, body) = get(&w.url(&format!("/{}/", share.code)), None).await;
    assert!(
        !body.contains("The album") && !is_app_page(&body),
        "{status}: {body}"
    );
    let bare = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(w.url(&format!("/s/{}", share.code)))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(bare.headers()["location"], link.as_str());
    let (status, _, _) = get(&w.url("/s/abcdefghjkmnpqrs/"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let track = page_with(&w.url(&format!("{link}track/")), None, "Mix notes.").await;
    assert!(home.contains("track"), "{home}");
    assert!(track.contains("Track"));

    // Out of scope: the private note's link is plain words, its embed and
    // its attachment are gone, nothing names it, and asking is 404.
    assert!(home.contains("the plan"), "{home}");
    for leak in ["secret", "Private", "cover.png", "plan/"] {
        assert!(!home.contains(leak), "`{leak}` in the shared note: {home}");
    }
    // The page is the web app's, and says nothing of the note but its
    // title: no note is drawn by the node.
    let (_, _, shell) = get(&w.url(&link), None).await;
    assert!(is_app_page(&shell), "{shell}");
    for word in ["Read", "Mix notes", "site-outside", "art.png"] {
        assert!(!shell.contains(word), "`{word}` in the app's page: {shell}");
    }
    assert!(shell.contains("og:title"), "{shell}");
    for path in [
        "Private/plan/",
        "Private/cover.png",
        "plan/",
        "_architect/shares/",
        "%2e%2e/%2e%2e/vaults/notes/Private/plan.md",
    ] {
        let (status, _, body) = get(&w.url(&format!("{link}{path}")), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert!(!body.contains("secret plan"), "{path}");
    }
    let (status, _, art) = get(&w.url(&format!("{link}Shared/art.png")), None).await;
    assert_eq!((status, art.as_str()), (StatusCode::OK, "shared png"));
    // The link's answers keep the code to themselves.
    let (_, headers, body) = get(&w.url(&link), None).await;
    assert_eq!(headers["referrer-policy"], "no-referrer");
    // The web app's own Content-Security-Policy on its page (#646): no
    // inline script, nothing loaded or framed off the node, never framed.
    let policy = node::share_shell::policy(Some(w.issuer.trim_end_matches('/')));
    assert_eq!(headers["content-security-policy"], policy.as_str());
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(body.matches("<script").count(), 1, "{body}");
    assert!(body.contains("<script type=\"module\" src=\"/app/start.js\"></script>"));
    let (_, headers, files) = get(&w.url(&format!("{link}_files/")), None).await;
    assert!(is_app_page(&files), "{files}");
    assert_eq!(headers["content-security-policy"], policy.as_str());
    // A file and a 404 keep the node's own policy.
    let share_policy = format!(
        "default-src 'self'; script-src 'self' {}; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'self'; img-src 'self' data: https:; media-src 'self'; style-src 'self' 'unsafe-inline'",
        architect_sites::ui::THEME_BOOT_HASH
    );
    for path in [
        "/s/abcdefghjkmnpqrs/".to_owned(),
        format!("{link}Shared/art.png"),
    ] {
        let (_, headers, _) = get(&w.url(&path), None).await;
        assert_eq!(
            headers["content-security-policy"],
            share_policy.as_str(),
            "{path}"
        );
    }
    // No page the build wrote is ever served: not by its name, not as a
    // folder without its slash.
    for path in [
        "index.html",
        "track/index.html",
        "404.html",
        "track/INDEX.HTML",
    ] {
        let (status, _, body) = get(&w.url(&format!("{link}{path}")), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert!(!body.contains("Mix notes"), "{path}");
    }
    let (status, _, body) = get(&w.url(&format!("{link}track")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(is_app_page(&body), "{body}");
    assert!(
        headers["x-robots-tag"]
            .to_str()
            .unwrap()
            .contains("noindex")
    );

    // The share is a note of the vault, hidden from every Site.
    let note = w
        .host
        .state
        .vault_root("notes")
        .unwrap()
        .join(format!("_architect/shares/{}.md", share.code));
    assert!(
        std::fs::read_to_string(&note)
            .unwrap()
            .contains("publish: false")
    );

    // A live edit, from a signed-in web client on the org lane: on the
    // shared page in seconds, and the page's event stream fires.
    let mut events = reqwest::Client::new()
        .get(w.url(&format!("{link}_live")))
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let sync: DocSyncClient = w.lane(&w.alan).await;
    let doc_id = vault_proto::collab_doc_id("notes", "Shared/track.md");
    let replica = crdt::CrdtDoc::ephemeral();
    let mut synced = SyncedDoc::new(doc_id, replica.clone());
    let session = tokio::spawn(async move {
        let _ = synced.run(&sync).await;
    });
    for _ in 0..200 {
        let text = replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
        if text.to_string().contains("Mix notes.") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let text = replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
    text.insert(text.len_unicode(), "\nTyped live in the browser.\n")
        .unwrap();
    replica.loro().commit();
    let started = std::time::Instant::now();
    page_with(
        &w.url(&format!("{link}track/")),
        None,
        "Typed live in the browser.",
    )
    .await;
    let took = started.elapsed();
    assert!(took < Duration::from_secs(8), "took {took:?}");
    let fired = tokio::time::timeout(Duration::from_secs(5), async {
        let mut seen = String::new();
        while let Ok(Some(chunk)) = events.chunk().await {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("data: rebuilt") {
                return seen;
            }
        }
        seen
    })
    .await
    .unwrap();
    assert!(fired.contains("data: rebuilt"), "{fired}");
    session.abort();

    // Revoked: 404 on the next request, and listed as revoked.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let revoked = links.revoke(share.code.clone()).await.unwrap();
    assert!(revoked.revoked_at.is_some());
    let (status, _, body) = get(&w.url(&link), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!body.contains("The album"));
    let listed = links.shares().await.unwrap();
    assert!(
        listed
            .iter()
            .any(|s| s.code == share.code && s.revoked_at.is_some())
    );
    // A revoke is final.
    assert!(
        links
            .update(share.code.clone(), ShareChange::Expires(None))
            .await
            .is_err()
    );

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_audience_lets_in_its_people_and_no_one_else() {
    let w = world().await;
    let scope = || ShareScope::Notes(vec!["Shared/track.md".to_owned()]);

    // The org's members, signed in: bearer or cookie. Nobody: sign in.
    // Someone in another org: refused.
    let org = w.share(scope(), Audience::Org, ShareAccess::Read).await;
    let link = w.url(&format!("/s/{}/", org.code));
    let alan_cookie = format!("cookie:{}", w.alan);
    page_with(&link, Some(&w.alan), "Mix notes.").await;
    page_with(&link, Some(&alan_cookie), "Mix notes.").await;
    page_with(&link, Some(&w.grace), "Mix notes.").await;
    // Nobody: the app's page (it offers the sign-in), naming nothing of
    // the link, not even its title.
    let (status, headers, body) = get(&link, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(is_app_page(&body), "{body}");
    assert!(
        body.contains("<title>Shared link · Architect</title>"),
        "{body}"
    );
    assert!(!body.contains("Mix notes."));
    assert_eq!(headers["cache-control"], "private, no-store");
    // The guest service takes the node's sign-in cookie in place of a
    // token, from a page on this origin (the app after `/s/_signin`).
    let call = |cookie: Option<String>| {
        let mut req = reqwest::Client::new()
            .post(w.url(&format!("/s/{}/_guest/link", org.code)))
            .json(&serde_json::json!({ "code": org.code, "token": "" }));
        if let Some(c) = cookie {
            req = req.header("cookie", format!("architect_session={c}"));
        }
        req.send()
    };
    let res = call(Some(w.alan.clone())).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.text().await.unwrap().contains("Track"));
    assert_eq!(call(None).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        call(Some(w.ada.clone())).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let (status, _, body) = get(&link, Some(&w.ada)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.contains("Mix notes."));
    let (status, _, _) = get(&link, Some("not-a-session")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A signed-in share's pages are cached by nobody.
    let (_, headers, _) = get(&link, Some(&w.alan)).await;
    assert_eq!(headers["cache-control"], "private, no-store");

    // Invited people: Ada, by email, though she is in no org of this
    // node's; not Alan, a member.
    let invited = w
        .share(
            scope(),
            Audience::Invited(vec!["ADA@local.test".to_owned()]),
            ShareAccess::Read,
        )
        .await;
    let link = w.url(&format!("/s/{}/", invited.code));
    page_with(&link, Some(&w.ada), "Mix notes.").await;
    assert_eq!(get(&link, Some(&w.alan)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(get(&link, None).await.0, StatusCode::UNAUTHORIZED);

    // Public: anyone, and it may be listed.
    let public = w.share(scope(), Audience::Public, ShareAccess::Read).await;
    let link = w.url(&format!("/s/{}/", public.code));
    page_with(&link, None, "Mix notes.").await;
    assert!(!get(&link, None).await.1.contains_key("x-robots-tag"));

    // Only the org's people make shares; a member lists them.
    let ada: ShareLinksClient = w.lane(&w.ada).await;
    assert!(ada.shares().await.is_err());
    let alan: ShareLinksClient = w.lane(&w.alan).await;
    assert_eq!(alan.shares().await.unwrap().len(), 3);

    // Signing in starts the issuer's device sign-in: an approval link,
    // and the node never asks for a password.
    let (status, headers, body) =
        get(&w.url(&format!("/s/_signin?to=/s/{}/", org.code)), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Approve sign-in"), "{body}");
    assert!(!body.to_lowercase().contains("password"), "{body}");
    let set = headers["set-cookie"].to_str().unwrap();
    assert!(
        set.starts_with("architect_signin=") && set.contains("HttpOnly"),
        "{set}"
    );

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_expires_takes_comments_when_asked_and_has_its_own_domain() {
    let w = world().await;
    let links: ShareLinksClient = w.lane(&w.grace).await;

    // Comments: the node-rendered page's form is gone with the page
    // (#646); a link that takes comments takes none from a form, and its
    // owner's list stays (the guest service gains them later).
    let talk = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Comment,
        )
        .await;
    let link = format!("/s/{}/", talk.code);
    page_with(&w.url(&link), None, "The album").await;
    let res = reqwest::Client::new()
        .post(w.url(&format!("{link}_comment")))
        .form(&[
            ("page", "/track/"),
            ("text", "Love the bridge."),
            ("name", "Sam"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(links.comments(talk.code.clone()).await.unwrap().is_empty());

    let read = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    page_with(&w.url(&format!("/s/{}/", read.code)), None, "The album").await;

    // A domain of its own: the share at that host's root, as a Site (its
    // built pages; no web app is on that host).
    links
        .update(
            read.code.clone(),
            ShareChange::Domain(Some("album.example.test".to_owned())),
        )
        .await
        .unwrap();
    let mut body = String::new();
    for _ in 0..200 {
        let res = reqwest::Client::new()
            .get(w.url("/track/"))
            .header("host", "album.example.test")
            .send()
            .await
            .unwrap();
        if res.status() == StatusCode::OK {
            body = res.text().await.unwrap();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(body.contains("Mix notes."), "{body}");
    assert!(body.contains("href=\"/\""), "rooted at the domain: {body}");

    // Expiry: open until then, 404 from then on.
    let soon = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Shared".to_owned()),
            title: Some("For a moment".to_owned()),
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: Some(node::shares::now() + 2),
            domain: None,
            listen: Listen::default(),
        })
        .await
        .unwrap();
    let link = w.url(&format!("/s/{}/", soon.code));
    page_with(&link, None, "The album").await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let (status, _, body) = get(&link, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // An expiry already past is refused.
    assert!(
        links
            .create(NewShare {
                vault: "notes".to_owned(),
                scope: ShareScope::Vault,
                title: None,
                audience: Audience::Link,
                access: ShareAccess::Read,
                expires_at: Some(1),
                domain: None,
                listen: Listen::default(),
            })
            .await
            .is_err()
    );

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_share_actions_make_list_and_revoke_a_link() {
    use std::sync::Arc;

    use architect::action::ActionList;
    use architect_accounts::{AccountStore, Accounts, MemorySecrets};
    use serde_json::{Value, json};

    let w = world().await;
    // The app, signed in as Grace, in `indie`.
    let store = Arc::new(AccountStore::in_memory(Arc::new(MemorySecrets::new())));
    let accounts = Accounts::new(store, Issuer::new(&w.issuer), "architect-test").without_browser();
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &accounts).unwrap();
    accounts
        .sign_in_password("grace@local.test", auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    accounts.refresh_orgs().await.unwrap();
    list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap();
    let run = |id: &'static str, args: Value| {
        let list = Arc::clone(&list);
        async move {
            list.run_json_async(id, args.to_string().as_bytes())
                .await
                .map(|out| serde_json::from_slice::<Value>(&out).unwrap())
                .map_err(|e| format!("{e:?}"))
        }
    };
    let host = format!("http://{}", w.host.http);

    let made = run(
        "SHARE_CREATE",
        json!({"folder": "Shared", "audience": "link", "expires": "7d", "host": host}),
    )
    .await
    .unwrap();
    let url = made["share"]["url"].as_str().unwrap().to_owned();
    let code = made["share"]["code"].as_str().unwrap().to_owned();
    assert_eq!(url, format!("{host}/s/{code}/"));
    assert_eq!(made["share"]["scope"], "the folder Shared");
    assert_eq!(made["share"]["state"], "live");
    page_with(&url, None, "The album").await;

    let listed = run("SHARE_LIST", json!({})).await.unwrap();
    assert_eq!(listed["shares"].as_array().unwrap().len(), 1, "{listed}");

    let changed = run("SHARE_UPDATE", json!({"code": code, "audience": "org"}))
        .await
        .unwrap();
    assert_eq!(changed["share"]["audience"], "org");
    assert_eq!(get(&url, None).await.0, StatusCode::UNAUTHORIZED);
    page_with(&url, Some(&w.alan), "The album").await;

    let domained = run(
        "PUBLISH_DOMAIN",
        json!({"code": code, "domain": "album.example.test"}),
    )
    .await
    .unwrap();
    assert_eq!(domained["share"]["domain"], "album.example.test");

    // The sheet opens on a scope (from a file's context menu it would be
    // that file).
    let sheet = run("SHARE_SHEET", json!({"tag": "mix"})).await.unwrap();
    assert_eq!(sheet["sheet"], "share");
    assert_eq!(sheet["scope"], "the notes tagged #mix");

    run("SHARE_REVOKE", json!({"code": code})).await.unwrap();
    let (status, _, _) = get(&url, Some(&w.alan)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let listed = run("SHARE_LIST", json!({})).await.unwrap();
    assert_eq!(listed["shares"][0]["state"], "revoked");

    w.host.stop().await;
}

/// The setlist shared with two audiences; the share, the violins' link
/// and the cellos' (each `/s/<code>/`).
async fn setlist(w: &World) -> (Share, String, String) {
    let share = w
        .share(
            ShareScope::Folder("Set".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let add = |name: &str, filter: &str| {
        links.add_audience(
            share.code.clone(),
            NewAudience {
                name: name.to_owned(),
                filter: filter.to_owned(),
                listen: None,
            },
        )
    };
    add("violin", "instrument == \"violin\"").await.unwrap();
    let share = add("cello", "instrument == \"cello\"").await.unwrap();
    let path = |name: &str| share.audience_named(name).unwrap().path();
    let (violin, cello) = (path("violin"), path("cello"));
    (share, violin, cello)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_audiences_are_each_served_only_their_own_notes_and_files() {
    let w = world().await;
    let (share, violin, cello) = setlist(&w).await;
    let whole = share.path();
    assert_ne!(violin, cello);
    assert_ne!(violin, whole);
    let status = |url: String| async move { get(&url, None).await.0 };

    // The share's own link is the whole set: every chart and every file.
    page_with(&w.url(&format!("{whole}04-noel-cello/")), None, "Noel").await;
    for page in ["01-joy-violin", "02-joy-cello", "03-silent-violin"] {
        assert_eq!(
            status(w.url(&format!("{whole}{page}/"))).await,
            StatusCode::OK
        );
    }
    for file in ["joy-violin", "joy-cello", "silent-violin", "noel-cello"] {
        let url = w.url(&format!("{whole}Set/charts/{file}.pdf"));
        assert_eq!(status(url).await, StatusCode::OK, "{file}");
    }

    // Each audience's link serves its charts and their files…
    let mine = [
        (
            &violin,
            ["01-joy-violin", "03-silent-violin"],
            ["joy-violin", "silent-violin"],
        ),
        (
            &cello,
            ["02-joy-cello", "04-noel-cello"],
            ["joy-cello", "noel-cello"],
        ),
    ];
    for (link, pages, _) in &mine {
        page_with(&w.url(&format!("{link}{}/", pages[1])), None, "part of").await;
    }
    for (i, (link, pages, files)) in mine.iter().enumerate() {
        let (_, theirs_pages, theirs_files) = &mine[1 - i];
        let mut seen = String::new();
        for page in pages {
            let url = w.url(&format!("{link}{page}/"));
            let (code, _, body) = get(&url, None).await;
            assert_eq!(code, StatusCode::OK, "{link}{page}");
            assert!(is_app_page(&body));
            seen.push_str(&words_at(&url, None).await.unwrap());
        }
        for file in files {
            let url = w.url(&format!("{link}Set/charts/{file}.pdf"));
            let (code, headers, body) = get(&url, None).await;
            assert_eq!(code, StatusCode::OK, "{url}");
            assert_eq!(headers["content-type"], "application/pdf");
            assert!(body.starts_with("%PDF"), "{body}");
        }
        // …and the other audience's are 404 on it: the page route (with
        // and without its slash, and by its file), and the byte route.
        for page in theirs_pages {
            for path in [
                format!("{page}/"),
                page.to_string(),
                format!("{page}/index.html"),
            ] {
                let url = w.url(&format!("{link}{path}"));
                assert_eq!(status(url.clone()).await, StatusCode::NOT_FOUND, "{url}");
            }
        }
        for file in theirs_files {
            for path in [
                format!("Set/charts/{file}.pdf"),
                format!("Set/charts/{file}%2Epdf"),
                format!("Set/charts/../charts/{file}.pdf"),
            ] {
                let url = w.url(&format!("{link}{path}"));
                assert_eq!(status(url.clone()).await, StatusCode::NOT_FOUND, "{url}");
            }
        }
        // Nothing of the other audience is listed: not in a note's words,
        // the link's listing or its files.
        let code = link
            .trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned();
        let listed = guest(&w).link(code.clone(), String::new()).await.unwrap();
        let _ = write!(seen, "{listed:?}");
        let files = guest(&w).files(code, String::new(), None).await.unwrap();
        let _ = write!(seen, "{files:?}");
        let (_, _, json) = get(&w.url(&format!("{link}_files.json")), None).await;
        seen.push_str(&json);
        let other = if i == 0 { "cello" } else { "violin" };
        assert!(
            !seen.to_lowercase().contains(other),
            "{other} in {link}: {seen}"
        );
    }

    // A chart that leaves an audience's filter is 404 on its link, and
    // the audience it joins has it.
    let root = w.host.state.vault_root("notes").unwrap();
    touch(
        &root,
        "Set/03-silent-violin.md",
        "---\ntitle: Silent Night (cello)\nsong: Silent Night\ninstrument: cello\n---\nNow the cello part of Silent Night.\n\n![[charts/silent-violin.pdf]]\n",
    );
    page_with(
        &w.url(&format!("{cello}03-silent-violin/")),
        None,
        "Now the cello",
    )
    .await;
    assert_eq!(
        status(w.url(&format!("{cello}Set/charts/silent-violin.pdf"))).await,
        StatusCode::OK
    );
    for path in ["03-silent-violin/", "Set/charts/silent-violin.pdf"] {
        let url = w.url(&format!("{violin}{path}"));
        assert_eq!(status(url.clone()).await, StatusCode::NOT_FOUND, "{url}");
    }
    assert_eq!(
        status(w.url(&format!("{violin}01-joy-violin/"))).await,
        StatusCode::OK
    );

    // An audience is removed alone: its link is 404 on the next request,
    // the other's and the share's own stay.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let left = links
        .remove_audience(share.code.clone(), "Violin".to_owned())
        .await
        .unwrap();
    assert_eq!(left.audiences.len(), 1);
    assert_eq!(status(w.url(&violin)).await, StatusCode::NOT_FOUND);
    assert_eq!(
        status(w.url(&format!("{violin}01-joy-violin/"))).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(w.url(&format!("{cello}04-noel-cello/"))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(w.url(&format!("{whole}01-joy-violin/"))).await,
        StatusCode::OK
    );
    // A name is free again, a name taken is refused, and so is a filter
    // that is not a Bases filter.
    let bad = |name: &str, filter: &str| {
        links.add_audience(
            share.code.clone(),
            NewAudience {
                name: name.to_owned(),
                filter: filter.to_owned(),
                listen: None,
            },
        )
    };
    assert!(bad("cello", "instrument == \"cello\"").await.is_err());
    assert!(bad("brass", "instrument ==").await.is_err());
    let again = bad("violin", "instrument == \"violin\"").await.unwrap();
    assert_ne!(again.audience_named("violin").unwrap().path(), violin);

    // The whole share revoked takes every audience's link with it.
    links.revoke(share.code.clone()).await.unwrap();
    assert_eq!(
        status(w.url(&format!("{cello}04-noel-cello/"))).await,
        StatusCode::NOT_FOUND
    );

    w.host.stop().await;
}

/// GET `url`; status, headers and the body's bytes.
async fn get_bytes(url: &str) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
    let res = reqwest::Client::new().get(url).send().await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.bytes().await.unwrap().to_vec())
}

/// The zip at `url`: each file's path and text, in the archive's order.
async fn zip_at(url: &str) -> Vec<(String, String)> {
    let (status, headers, bytes) = get_bytes(url).await;
    assert_eq!(status, StatusCode::OK, "{url}");
    assert_eq!(headers["content-type"], "application/zip");
    // Streamed: the node never knew its length.
    assert!(headers.get("content-length").is_none(), "{headers:?}");
    architect_sites::archive::read_stored(&bytes)
        .unwrap_or_else(|| panic!("{url}: not a zip ({} bytes)", bytes.len()))
        .into_iter()
        .map(|(name, bytes)| (name, String::from_utf8(bytes).unwrap()))
        .collect()
}

fn pairs(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_links_files_page_switches_views_and_its_zip_holds_exactly_its_files_in_order() {
    let w = world().await;
    let (share, violin, cello) = setlist(&w).await;
    let whole = share.path();
    page_with(
        &w.url(&format!("{violin}03-silent-violin/")),
        None,
        "part of",
    )
    .await;
    page_with(&w.url(&format!("{cello}04-noel-cello/")), None, "part of").await;

    // The files page is the web app's Files view of the link (#646);
    // what it lists is the link's `_files.json` and the guest service's
    // `files`: the violins' charts and no others, in list order.
    let files = format!("{violin}_files/");
    let (status, _, page) = get(&w.url(&files), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(is_app_page(&page), "{page}");
    let listed = json_at(&w.url(&format!("{violin}_files.json"))).await;
    let paths: Vec<&str> = listed["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["Set/charts/joy-violin.pdf", "Set/charts/silent-violin.pdf"]
    );
    let everyone = json_at(&w.url(&format!("{whole}_files.json"))).await;
    assert_eq!(everyone["files"].as_array().unwrap().len(), 4);
    // A view that is none: refused.
    let (status, _, _) = get(&w.url(&format!("{violin}_archive.zip?by=shuffle")), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The violins' zip: exactly their files, a numbered folder per chart
    // in list order, each file's own bytes.
    let joy = "%PDF Joy to the World for violin";
    let silent = "%PDF Silent Night for violin";
    assert_eq!(
        zip_at(&w.url(&format!("{violin}_archive.zip?by=note"))).await,
        pairs(&[
            ("01 Joy to the World (violin)/joy-violin.pdf", joy),
            ("02 Silent Night (violin)/silent-violin.pdf", silent),
        ])
    );
    // In order: numbered files. By a property: a folder per value.
    assert_eq!(
        zip_at(&w.url(&format!("{violin}_archive.zip"))).await,
        pairs(&[("01 joy-violin.pdf", joy), ("02 silent-violin.pdf", silent)])
    );
    assert_eq!(
        zip_at(&w.url(&format!("{violin}_archive.zip?by=property:song"))).await,
        pairs(&[
            (
                "Joy to the World/01 Joy to the World (violin)/joy-violin.pdf",
                joy
            ),
            (
                "Silent Night/01 Silent Night (violin)/silent-violin.pdf",
                silent
            ),
        ])
    );
    // The cellos' holds theirs, and none of the violins'.
    assert_eq!(
        zip_at(&w.url(&format!("{cello}_archive.zip?by=note"))).await,
        pairs(&[
            (
                "01 Joy to the World (cello)/joy-cello.pdf",
                "%PDF Joy to the World for cello"
            ),
            ("02 Noel (cello)/noel-cello.pdf", "%PDF Noel for cello"),
        ])
    );
    // One group alone: one song of the whole set, both parts, in order.
    let (status, headers, bytes) = get_bytes(&w.url(&format!(
        "{whole}_archive.zip?by=property:song&group=Joy%20to%20the%20World"
    )))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("Joy to the World.zip"),
        "{headers:?}"
    );
    let names: Vec<String> = architect_sites::archive::read_stored(&bytes)
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        [
            "Joy to the World/01 Joy to the World (violin)/joy-violin.pdf",
            "Joy to the World/02 Joy to the World (cello)/joy-cello.pdf"
        ]
    );
    // A group the audience does not have is not there to download, and
    // a group of another audience's is no way round its filter.
    for group in ["Nowhere", "Noel"] {
        let url = w.url(&format!(
            "{violin}_archive.zip?by=property:song&group={group}"
        ));
        assert_eq!(get_bytes(&url).await.0, StatusCode::NOT_FOUND, "{url}");
    }

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_audience_actions_add_list_archive_and_remove() {
    use std::sync::Arc;

    use architect::action::ActionList;
    use architect_accounts::{AccountStore, Accounts, MemorySecrets};
    use serde_json::{Value, json};

    let w = world().await;
    let store = Arc::new(AccountStore::in_memory(Arc::new(MemorySecrets::new())));
    let accounts = Accounts::new(store, Issuer::new(&w.issuer), "architect-test").without_browser();
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &accounts).unwrap();
    accounts
        .sign_in_password("grace@local.test", auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    accounts.refresh_orgs().await.unwrap();
    list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap();
    let run = |id: &'static str, args: Value| {
        let list = Arc::clone(&list);
        async move {
            list.run_json_async(id, args.to_string().as_bytes())
                .await
                .map(|out| serde_json::from_slice::<Value>(&out).unwrap())
                .map_err(|e| format!("{e:?}"))
        }
    };
    let host = format!("http://{}", w.host.http);
    let made = run(
        "SHARE_CREATE",
        json!({"folder": "Set", "title": "Carols", "host": host}),
    )
    .await
    .unwrap();
    let code = made["share"]["code"].as_str().unwrap().to_owned();
    assert_eq!(made["share"]["audiences"], json!([]));

    let added = run(
        "SHARE_AUDIENCE_ADD",
        json!({"code": code, "name": "violin", "filter": "instrument == \"violin\""}),
    )
    .await
    .unwrap();
    let url = added["audience"]["url"].as_str().unwrap().to_owned();
    let audience_code = added["audience"]["code"].as_str().unwrap();
    assert_eq!(url, format!("{host}/s/{audience_code}/"));
    assert_ne!(audience_code, code);
    assert_eq!(added["share"]["audiences"][0]["name"], "violin");
    run(
        "SHARE_AUDIENCE_ADD",
        json!({"code": code, "name": "everyone"}),
    )
    .await
    .unwrap();
    assert!(
        run(
            "SHARE_AUDIENCE_ADD",
            json!({"code": code, "name": "bad", "filter": "=="})
        )
        .await
        .is_err()
    );
    let listed = run("SHARE_AUDIENCES", json!({"code": code})).await.unwrap();
    let names: Vec<&str> = listed["audiences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["violin", "everyone"]);
    assert_eq!(listed["audiences"][0]["filter"], "instrument == \"violin\"");
    page_with(&format!("{url}03-silent-violin/"), None, "part of").await;
    assert_eq!(
        get(&format!("{url}04-noel-cello/"), None).await.0,
        StatusCode::NOT_FOUND
    );

    // The same zip the link gives, saved by the app.
    let tmp = tempfile::tempdir().unwrap();
    let to = tmp.path().join("violins.zip");
    let saved = run(
        "SHARE_ARCHIVE",
        json!({"code": code, "audience": "violin", "to": to.to_string_lossy()}),
    )
    .await
    .unwrap();
    assert_eq!(saved["files"], 2, "{saved}");
    let bytes = std::fs::read(&to).unwrap();
    assert_eq!(saved["bytes"], bytes.len());
    assert_eq!(
        bytes,
        get_bytes(&format!("{url}_archive.zip?by=note")).await.2
    );
    let names: Vec<String> = architect_sites::archive::read_stored(&bytes)
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        [
            "01 Joy to the World (violin)/joy-violin.pdf",
            "02 Silent Night (violin)/silent-violin.pdf"
        ]
    );
    // The whole share's, one group of it.
    let one = tmp.path().join("noel.zip");
    let saved = run(
        "SHARE_ARCHIVE",
        json!({"code": code, "by": "property:song", "group": "Noel", "to": one.to_string_lossy()}),
    )
    .await
    .unwrap();
    assert_eq!(saved["files"], 1, "{saved}");
    // No such audience or group: an error, and no file left behind.
    let none = tmp.path().join("none.zip");
    for args in [
        json!({"code": code, "audience": "brass", "to": none.to_string_lossy()}),
        json!({"code": code, "by": "property:song", "group": "Nowhere", "to": none.to_string_lossy()}),
    ] {
        assert!(run("SHARE_ARCHIVE", args).await.is_err());
        assert!(!none.exists());
    }

    let left = run(
        "SHARE_AUDIENCE_REMOVE",
        json!({"code": code, "name": "violin"}),
    )
    .await
    .unwrap();
    assert_eq!(left["audiences"].as_array().unwrap().len(), 1, "{left}");
    assert_eq!(get(&url, None).await.0, StatusCode::NOT_FOUND);
    assert!(
        run(
            "SHARE_AUDIENCE_REMOVE",
            json!({"code": code, "name": "violin"})
        )
        .await
        .is_err()
    );

    w.host.stop().await;
}

// ── The player (#511) ────────────────────────────────────────────────

/// GET `url` with `range` as its `Range` header; status, headers, bytes.
async fn get_range(
    url: &str,
    range: Option<&str>,
) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
    let mut req = reqwest::Client::new().get(url);
    if let Some(range) = range {
        req = req.header("range", range);
    }
    let res = req.send().await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.bytes().await.unwrap().to_vec())
}

#[tokio::test(flavor = "multi_thread")]
async fn media_streams_with_ranges_lyrics_follow_and_the_queue_is_the_links_order() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    // A gig: two songs in list order, one for the band alone.
    touch(
        &vault,
        "Gig/01-joy.md",
        "---\ntitle: Joy\nsection: band\n---\n![[media/joy.mp3]]\n\n![[media/joy.lrc]]\n",
    );
    touch(
        &vault,
        "Gig/02-noel.md",
        "---\ntitle: Noel\nsection: crew\n---\n![[media/clip.mp4]]\n\n```lrc\n[00:02.00]Noel <b>\n[00:04.50]Born is the King\n```\n",
    );
    touch(
        &vault,
        "Gig/media/joy.lrc",
        "[ti:Joy]\n[00:01.50]Joy to the world\n[00:03.25]The Lord is come\n",
    );
    let audio: Vec<u8> = (0..1000_u32)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    std::fs::write(vault.join("Gig/media/joy.mp3"), &audio).unwrap();
    std::fs::write(vault.join("Gig/media/clip.mp4"), &audio[..500]).unwrap();

    let share = w
        .share(
            ShareScope::Folder("Gig".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .add_audience(
            share.code.clone(),
            NewAudience {
                name: "band".to_owned(),
                filter: "section == \"band\"".to_owned(),
                listen: None,
            },
        )
        .await
        .unwrap();
    let whole = share.path();
    let band = share.audience_named("band").unwrap().path();

    // The song's page is the app's; its guest reads its recording and its
    // lyrics as timed lines, and the link lists its recordings in list
    // order: the app's Now Playing queue (`guest_view::queue`).
    let joy = page_with(&w.url(&format!("{whole}01-joy/")), None, "Joy to the world").await;
    assert!(joy.contains(&format!("{whole}Gig/media/joy.mp3")), "{joy}");
    assert!(joy.contains("The Lord is come"), "{joy}");
    let code_of = |link: &str| {
        link.trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned()
    };
    let listed = guest(&w)
        .link(code_of(&whole), String::new())
        .await
        .unwrap();
    let media: Vec<&str> = listed
        .notes
        .iter()
        .flat_map(|n| n.files.iter())
        .filter(|f| {
            [".mp3", ".mp4"]
                .iter()
                .any(|e| f.to_ascii_lowercase().ends_with(e))
        })
        .map(String::as_str)
        .collect();
    assert_eq!(media, ["Gig/media/joy.mp3", "Gig/media/clip.mp4"]);
    assert_eq!(
        listed.notes[0].page.as_deref(),
        Some(format!("{whole}01-joy/").as_str())
    );

    // A fence of LRC reads the same, its words as words.
    let noel = page_with(
        &w.url(&format!("{whole}02-noel/")),
        None,
        "Born is the King",
    )
    .await;
    assert!(noel.contains("Noel <b>"), "{noel}");

    // Ranges: a slice, the tail, the whole, and one past the end.
    let mp3 = w.url(&format!("{whole}Gig/media/joy.mp3"));
    let (code, headers, body) = get_range(&mp3, Some("bytes=100-199")).await;
    assert_eq!(code, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-range"], "bytes 100-199/1000");
    assert_eq!(headers["accept-ranges"], "bytes");
    assert_eq!(headers["content-type"], "audio/mpeg");
    assert_eq!(headers["content-length"], "100");
    assert_eq!(body, audio[100..200]);
    let (code, headers, body) = get_range(&mp3, Some("bytes=-10")).await;
    assert_eq!(code, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-range"], "bytes 990-999/1000");
    assert_eq!(body, audio[990..]);
    let (code, headers, body) = get_range(&mp3, Some("bytes=900-")).await;
    assert_eq!(code, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-range"], "bytes 900-999/1000");
    assert_eq!(body.len(), 100);
    let (code, headers, body) = get_range(&mp3, None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(headers["accept-ranges"], "bytes");
    assert_eq!(body, audio);
    let (code, headers, _) = get_range(&mp3, Some("bytes=2000-")).await;
    assert_eq!(code, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(headers["content-range"], "bytes */1000");
    // Still the share's: no-referrer and its cache rule on a 206.
    let (_, headers, _) = get_range(&mp3, Some("bytes=0-1")).await;
    assert_eq!(headers["referrer-policy"], "no-referrer");
    let lrc = w.url(&format!("{whole}Gig/media/joy.lrc"));
    let (code, headers, _) = get_range(&lrc, None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");

    // The band's link: its song plays with ranges; the crew's video is
    // 404 there, ranged or not, and is not in its queue.
    page_with(&w.url(&format!("{band}01-joy/")), None, "Joy to the world").await;
    let band_listed = guest(&w).link(code_of(&band), String::new()).await.unwrap();
    assert_eq!(band_listed.notes.len(), 1, "{band_listed:?}");
    assert_eq!(
        band_listed.notes[0].files,
        ["Gig/media/joy.mp3", "Gig/media/joy.lrc"]
    );
    let (code, headers, body) = get_range(
        &w.url(&format!("{band}Gig/media/joy.mp3")),
        Some("bytes=10-19"),
    )
    .await;
    assert_eq!(code, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-range"], "bytes 10-19/1000");
    assert_eq!(body, audio[10..20]);
    for range in [None, Some("bytes=0-99"), Some("bytes=-1")] {
        let (code, _, body) = get_range(&w.url(&format!("{band}Gig/media/clip.mp4")), range).await;
        assert_eq!(code, StatusCode::NOT_FOUND, "{range:?}");
        assert_ne!(body, audio[..500], "{range:?}");
    }
    let (code, _, _) = get_range(&w.url(&format!("{band}02-noel/")), None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

/// #488's done-when: a note with a fence an installed extension claims
/// shows the extension's drawing at `/s/<code>/`, made safe, rebuilt live
/// as the note changes; an audience that cannot see a note never gets its
/// drawing.
#[tokio::test(flavor = "multi_thread")]
async fn a_claimed_fence_is_the_installed_extensions_drawing_live_and_per_audience() {
    use crdt::sync::{DocSyncClient, SyncedDoc};
    let w = world().await;
    let share = w
        .share(
            ShareScope::Folder("Charts".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let link = share.path();

    // The fence is the extension's SVG, not a code block: in the build
    // (never served on the link, #646), and to the link's guest as a drawn
    // block with its claim and its words.
    let code = share.code.clone();
    let words = page_with(&w.url(&format!("{link}joy/")), None, "fence:chart").await;
    assert!(words.contains("Am"), "{words}");
    let joy = built_page(&w, &code, None, "joy", "demo-chart").await;
    assert!(
        joy.contains("data-claim=\"fence:chart\" data-by=\"demo.chart\""),
        "{joy}"
    );
    for kept in [
        "<svg class=\"demo-chart\" viewBox=\"0 0 180 40\" width=\"180\" height=\"40\" data-follow-playhead=\"\">",
        "<rect class=\"demo-bar\" data-at=\"0\" x=\"0\" y=\"0\" width=\"60\" height=\"40\" fill=\"transparent\"></rect>",
        "data-at=\"4\"",
        ">Am</text>",
    ] {
        assert!(joy.contains(kept), "lost {kept}: {joy}");
    }
    assert!(!joy.contains("language-chart"), "{joy}");
    for never in [
        "alert(1)",
        "foreignObject",
        "html inside",
        "javascript:",
        "onclick",
    ] {
        assert!(!joy.contains(never), "`{never}` on the page: {joy}");
    }

    // Rebuilt live: the fence edited from a signed-in client is drawn
    // again on the link.
    let sync: DocSyncClient = w.lane(&w.alan).await;
    let doc_id = vault_proto::collab_doc_id("notes", "Charts/joy.md");
    let replica = crdt::CrdtDoc::ephemeral();
    let mut synced = SyncedDoc::new(doc_id, replica.clone());
    let session = tokio::spawn(async move {
        let _ = synced.run(&sync).await;
    });
    let text = replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
    for _ in 0..200 {
        if text.to_string().contains("| Am |") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let body = text.to_string();
    let (before, _) = body.split_once("| Am |").expect("the fence's last bar");
    text.insert(before.chars().count() + "| Am ".len(), "| E7 ")
        .unwrap();
    replica.loro().commit();
    page_with(&w.url(&format!("{link}joy/")), None, "E7").await;
    let joy = built_page(&w, &code, None, "joy", ">E7</text>").await;
    assert!(joy.contains("viewBox=\"0 0 240 40\""), "{joy}");
    assert!(joy.contains("data-at=\"6\""), "{joy}");
    session.abort();

    // Each audience gets the drawings of its own notes only.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    for (name, filter) in [
        ("violin", "instrument == \"violin\""),
        ("cello", "instrument == \"cello\""),
    ] {
        links
            .add_audience(
                share.code.clone(),
                NewAudience {
                    name: name.to_owned(),
                    filter: filter.to_owned(),
                    listen: None,
                },
            )
            .await
            .unwrap();
    }
    let share = links
        .shares()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.code == share.code)
        .unwrap();
    let violin = share.audience_named("violin").unwrap().path();
    let cello = share.audience_named("cello").unwrap().path();
    let mine = page_with(&w.url(&format!("{violin}joy/")), None, "fence:chart").await;
    assert!(mine.contains("E7"), "{mine}");
    let theirs = page_with(&w.url(&format!("{cello}noel/")), None, "fence:chart").await;
    assert!(theirs.contains("Bb"), "{theirs}");
    for (link, other) in [(&violin, "noel"), (&cello, "joy")] {
        let (status, _, body) = get(&w.url(&format!("{link}{other}/")), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{link}{other}/: {body}");
    }
    let mut seen = String::new();
    for path in ["", "joy/", "tags/"] {
        seen.push_str(&get(&w.url(&format!("{violin}{path}")), None).await.2);
    }
    seen.push_str(&mine);
    let violin_code = violin
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    let _ = write!(
        seen,
        "{:?}",
        guest(&w).link(violin_code, String::new()).await.unwrap()
    );
    for leak in ["Bb", "Dm7", "Noel chart"] {
        assert!(!seen.contains(leak), "`{leak}` on the violin link: {seen}");
    }

    w.host.stop().await;
}

/// An embedder that starts the node from Rust hands it its own blocks
/// (Studio's `kf` engraver as an `FnBlock`): its fence is drawn on a link,
/// made safe like any other; a fence an installed extension also claims
/// is still the installed one's.
#[tokio::test(flavor = "multi_thread")]
async fn an_embedders_own_blocks_draw_on_a_link_after_the_installed_ones() {
    use std::sync::Arc;

    use architect_sites::{Block, Drawn, FnBlock};
    let mut blocks = architect_sites::Blocks::new();
    blocks.add(Arc::new(FnBlock::new(
        "studio.kf",
        &["kf", "chart"],
        |block: &Block<'_>| {
            let bars = block.source.split('|').filter(|b| !b.trim().is_empty()).count();
            Ok(Drawn::Html(format!(
                "<svg class=\"kf-chart\" data-follow-playhead=\"\" data-bars=\"{bars}\" onload=\"alert(1)\"><rect class=\"kf-bar\" data-at=\"0\" width=\"10\" height=\"10\"/><script>alert(2)</script></svg>"
            )))
        },
    )));
    let w = world_with(blocks).await;
    let share = w
        .share(
            ShareScope::Folder("Embedded".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let words = page_with(&w.url(&format!("{}song/", share.path())), None, "fence:kf").await;
    assert!(words.contains("fence:chart"), "{words}");
    let song = built_page(&w, &share.code, None, "song", "kf-chart").await;
    for kept in [
        "data-claim=\"fence:kf\" data-by=\"studio.kf\"",
        "<svg class=\"kf-chart\" data-follow-playhead=\"\" data-bars=\"2\">",
        "<rect class=\"kf-bar\" data-at=\"0\" width=\"10\" height=\"10\"></rect>",
        // The `chart` fence: the installed extension draws it, not the embedder.
        "data-claim=\"fence:chart\" data-by=\"demo.chart\"",
    ] {
        assert!(song.contains(kept), "lost {kept}: {song}");
    }
    assert!(!song.contains("language-kf"), "{song}");
    let (_, from) = song.split_once("data-by=\"studio.kf\"").unwrap();
    let (block, _) = from.split_once("</div>").unwrap();
    for never in ["alert(", "onload", "<script"] {
        assert!(!block.contains(never), "`{never}` in the block: {block}");
    }
    assert_eq!(song.matches("data-by=\"studio.kf\"").count(), 1, "{song}");

    w.host.stop().await;
}

// ── Layers (#528) ────────────────────────────────────────────────────

/// An extension installed on the node that owns `type: song` notes and
/// places their files: the session and the reference for a Reference
/// link, every other proxy (the stems) for a Full one alone.
fn install_song_layers(pkg: &Path) {
    touch(
        pkg,
        "extension.toml",
        "id = \"demo.songs\"\nname = \"Songs\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
         [[share_layers]]\nnote_type = \"song\"\n\
         reference = [\"*.session\", \"Media/Proxies/Reference*\"]\n\
         full = [\"Media/Proxies/**\"]\n",
    );
}

/// Two songs, each attaching its session, its reference, its stems and
/// a chart; Joy is the band's, Noel the strings'.
fn seed_songs(vault: &Path) {
    for (song, section) in [("Joy", "band"), ("Noel", "strings")] {
        touch(
            vault,
            &format!("Songs/{song}/{song}.md"),
            &format!(
                "---\ntitle: {song}\ntype: song\nsection: {section}\n---\n# {song}\n\n\
                 ![[{song}.session]]\n\n![[Media/Proxies/Reference.ogg]]\n\n\
                 ![[Media/Proxies/Bass.ogg]]\n\n![[Media/Proxies/Gänse.ogg]]\n\n![[Charts/{song}.pdf]]\n"
            ),
        );
        let at = |rel: &str| format!("Songs/{song}/{rel}");
        touch(
            vault,
            &at(&format!("{song}.session")),
            &format!("session of {song}"),
        );
        touch(
            vault,
            &at("Media/Proxies/Reference.ogg"),
            &format!("REFERENCE {song}"),
        );
        touch(
            vault,
            &at("Media/Proxies/Bass.ogg"),
            &format!("STEM BASS {song}"),
        );
        touch(
            vault,
            &at("Media/Proxies/Gänse.ogg"),
            &format!("STEM GANSE {song}"),
        );
        touch(
            vault,
            &at(&format!("Charts/{song}.pdf")),
            &format!("%PDF {song}"),
        );
    }
}

/// GET exactly `path` (no client normalising its dots, slashes or
/// escapes) from `addr`, with `host` as its Host; the status and the body.
async fn raw_get(addr: std::net::SocketAddr, path: &str, host: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, text)
}

/// The names in the zip at `url`.
async fn zip_names(url: &str) -> Vec<String> {
    zip_at(url)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// The JSON at `url`, once it answers 200.
async fn json_at(url: &str) -> serde_json::Value {
    let (status, _, body) = get(url, None).await;
    assert_eq!(status, StatusCode::OK, "{url}: {body}");
    serde_json::from_str(&body).unwrap()
}

/// The `path` of each file a `_files.json` lists.
fn listed_paths(json: &serde_json::Value) -> Vec<String> {
    json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_links_layer_is_held_by_the_node_on_every_route() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    seed_songs(&vault);
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs".to_owned()),
            title: Some("Songs".to_owned()),
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::Full,
        })
        .await
        .unwrap();
    assert_eq!(share.listen, Listen::Full);
    let add = |name: &str, filter: &str, listen: Option<Listen>| {
        let (links, code) = (links.clone(), share.code.clone());
        let new = NewAudience {
            name: name.to_owned(),
            filter: filter.to_owned(),
            listen,
        };
        async move { links.add_audience(code, new).await.unwrap() }
    };
    add("orchestra", "", Some(Listen::Reference)).await;
    add("strings", "section == \"strings\"", None).await;
    let share = add("page", "", Some(Listen::Page)).await;
    let link = |name: Option<&str>| {
        let code = name.map_or(share.code.as_str(), |n| {
            &share.audience_named(n).unwrap().code
        });
        w.url(&format!("/s/{code}/"))
    };
    let (full, orchestra, strings, page_only) = (
        link(None),
        link(Some("orchestra")),
        link(Some("strings")),
        link(Some("page")),
    );
    assert_eq!(
        share.listen_for(share.audience_named("strings")),
        Listen::Full
    );
    for at in [&full, &orchestra, &page_only] {
        page_with(&format!("{at}joy/joy/"), None, "Joy").await;
    }
    // Pages are at lower-case slugs (`joy/joy/`); files keep their vault
    // path's case.
    let bass = "Songs/Joy/Media/Proxies/Bass.ogg".to_owned();
    let reference = "Songs/Joy/Media/Proxies/Reference.ogg";
    let session = "Songs/Joy/Joy.session";
    let chart = "Songs/Joy/Charts/Joy.pdf";

    // A Full link serves the stem, with a range too.
    let (status, _, bytes) = get_range(&format!("{full}{bass}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"STEM BASS Joy");
    let (status, headers, bytes) = get_range(&format!("{full}{bass}"), Some("bytes=5-8")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes, b"BASS");
    assert_eq!(headers["content-range"], "bytes 5-8/13");

    // A Reference link refuses it, saying the layer it needs; ranges too.
    for range in [None, Some("bytes=0-3")] {
        let (status, headers, bytes) = get_range(&format!("{orchestra}{bass}"), range).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{range:?}");
        assert_eq!(headers["x-architect-layer"], "full");
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            body.contains("the full layer") && !body.contains("STEM"),
            "{body}"
        );
    }
    // And serves what it plays, ranges and all.
    let (status, _, bytes) = get_range(&format!("{orchestra}{reference}"), Some("bytes=0-8")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes, b"REFERENCE");
    for file in [session, chart] {
        assert_eq!(
            get(&format!("{orchestra}{file}"), None).await.0,
            StatusCode::OK
        );
    }
    // A Page link: the chart alone.
    for file in [reference, session, bass.as_str()] {
        let (status, headers, _) = get(&format!("{page_only}{file}"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{file}");
        assert!(headers.contains_key("x-architect-layer"));
    }
    assert_eq!(
        get(&format!("{page_only}{chart}"), None).await.0,
        StatusCode::OK
    );

    // Other spellings of the stem: its case, its Unicode form, doubled
    // slashes, escapes, dot segments. Each is refused or not there; none
    // hands out its bytes.
    let addr = w.host.http;
    let path = |link: &str, rest: &str| format!("{}{rest}", link.trim_start_matches(&w.url("")));
    let tricks = [
        "Songs/Joy/Media/Proxies/bass.ogg".to_owned(),
        "Songs/Joy/Media/Proxies/BASS.OGG".to_owned(),
        "Songs/joy/media/proxies/Bass.ogg".to_owned(),
        "Songs//Joy/Media//Proxies/Bass.ogg".to_owned(),
        "Songs/Joy/Media/Proxies/%42ass.ogg".to_owned(),
        "Songs%2FJoy%2FMedia%2FProxies%2FBass.ogg".to_owned(),
        "Songs/Joy/Media/./Proxies/Bass.ogg".to_owned(),
        "Songs/Joy/Media/%2e/Proxies/Bass.ogg".to_owned(),
        "Songs/Joy/Charts/../Media/Proxies/Bass.ogg".to_owned(),
        "Songs/Joy/Charts/%2e%2e/Media/Proxies/Bass.ogg".to_owned(),
        // Gänse, composed (NFC) as written, and decomposed (NFD).
        "Songs/Joy/Media/Proxies/G%C3%A4nse.ogg".to_owned(),
        "Songs/Joy/Media/Proxies/Ga%CC%88nse.ogg".to_owned(),
        "Songs/Joy/Media/Proxies/GA%CC%88NSE.ogg".to_owned(),
        "Songs/Joy/Media/Proxies/Bass.ogg?listen=full".to_owned(),
        "Songs/Joy/Media/Proxies/Bass.ogg/".to_owned(),
    ];
    for trick in &tricks {
        for link in [&orchestra, &page_only] {
            let (status, text) = raw_get(addr, &path(link, trick), "127.0.0.1").await;
            assert_ne!(status, 200, "{trick}: {text}");
            assert_ne!(status, 206, "{trick}: {text}");
            assert!(!text.contains("STEM"), "{trick} leaked: {text}");
        }
    }
    // The held stem is not in a lower link's folder at all (#540's second
    // review): no spelling can reach bytes that are not there. On macOS
    // the file system would open it by another spelling (any case, either
    // Unicode form); those answer 403 or 404, never the stem.
    let orchestra_dir = w
        .tmp
        .path()
        .join("host/data/shares")
        .join(&share.code)
        .join("audiences")
        .join(&share.audience_named("orchestra").unwrap().code)
        .join("site");
    assert!(
        orchestra_dir
            .join("Songs/Joy/Media/Proxies/Reference.ogg")
            .is_file()
    );
    assert!(
        !orchestra_dir
            .join("Songs/Joy/Media/Proxies/Bass.ogg")
            .exists()
    );
    assert!(
        !orchestra_dir
            .join("Songs/Joy/Media/Proxies/Gänse.ogg")
            .exists()
    );
    #[cfg(target_os = "macos")]
    for asked in [
        "Songs/Joy/Media/Proxies/bass.ogg",
        "Songs/Joy/Media/Proxies/Ga%CC%88nse.ogg",
    ] {
        let (status, text) = raw_get(addr, &path(&orchestra, asked), "127.0.0.1").await;
        assert!(status == 403 || status == 404, "{asked}: {text}");
        assert!(!text.contains("STEM"), "{asked}: {text}");
    }
    // The same request on the Full link does reach the stem (the tricks
    // above are refused for its layer, not for being odd), where the file
    // system allows the spelling.
    let (status, _) = raw_get(
        addr,
        &path(&full, "Songs/Joy/Media/Proxies/%42ass.ogg"),
        "127.0.0.1",
    )
    .await;
    assert_eq!(status, 200);

    // Another audience's notes: not there at all (404, nothing about a
    // layer), even on a Full audience.
    let (status, headers, _) = get(&format!("{strings}{bass}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!headers.contains_key("x-architect-layer"));
    assert_eq!(
        get(&format!("{strings}Songs/Noel/Media/Proxies/Bass.ogg"), None)
            .await
            .0,
        StatusCode::OK
    );

    // The files the app lists (the guest's `files`, `_files.json`), the
    // archive and the link's recordings leave out what is above.
    let code_of = |link: &str| {
        link.trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned()
    };
    let files = format!(
        "{:?}",
        guest_of(&orchestra)
            .files(code_of(&orchestra), String::new(), None)
            .await
            .unwrap()
    );
    assert!(
        files.contains("Reference.ogg") && files.contains("Joy.session"),
        "{files}"
    );
    assert!(
        !files.contains("Bass.ogg") && !files.contains("nse.ogg"),
        "{files}"
    );
    let names = zip_names(&format!("{orchestra}_archive.zip?by=order")).await;
    assert!(
        names.iter().any(|n| n.ends_with("Reference.ogg")),
        "{names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.contains("Bass") || n.contains("nse")),
        "{names:?}"
    );
    let names = zip_names(&format!("{orchestra}_archive.zip?by=note&group=Joy")).await;
    assert!(!names.iter().any(|n| n.contains("Bass")), "{names:?}");
    let names = zip_names(&format!("{page_only}_archive.zip?by=order")).await;
    assert!(
        names
            .iter()
            .all(|n| n.contains("Charts") || n.contains("Joy.pdf") || n.contains("Noel.pdf")),
        "{names:?}"
    );
    let names = zip_names(&format!("{full}_archive.zip?by=order")).await;
    assert!(names.iter().any(|n| n.contains("Bass")), "{names:?}");
    let listed = format!(
        "{:?}",
        guest_of(&orchestra)
            .link(code_of(&orchestra), String::new())
            .await
            .unwrap()
    );
    assert!(
        listed.contains("Reference.ogg") && !listed.contains("Bass.ogg"),
        "{listed}"
    );
    let listed = format!(
        "{:?}",
        guest_of(&full)
            .link(code_of(&full), String::new())
            .await
            .unwrap()
    );
    assert!(listed.contains("Bass.ogg"), "{listed}");

    // The JSON listing: what the link serves, its layer and the most it
    // plays; a viewer may ask for less, never more.
    let listing = json_at(&format!("{full}_files.json")).await;
    assert_eq!(
        (listing["listen"].as_str(), listing["max"].as_str()),
        (Some("full"), Some("full"))
    );
    let stem = listing["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == bass.as_str())
        .unwrap()
        .clone();
    assert_eq!(stem["layer"], "full");
    assert_eq!(stem["size"], 13);
    assert_eq!(stem["url"], format!("/s/{}/{}", share.code, bass));
    assert_eq!(stem["note"], "Songs/Joy/Joy.md");
    let joy = listing["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["path"] == "Songs/Joy/Joy.md")
        .unwrap();
    assert_eq!(joy["type"], "song");
    let lower = json_at(&format!("{full}_files.json?listen=reference")).await;
    assert_eq!(
        (lower["listen"].as_str(), lower["max"].as_str()),
        (Some("reference"), Some("full"))
    );
    let paths = listed_paths(&lower);
    assert!(paths.contains(&reference.to_owned()) && paths.contains(&session.to_owned()));
    assert!(
        !paths
            .iter()
            .any(|p| p.contains("Bass") || p.contains("nse")),
        "{paths:?}"
    );
    for file in lower["files"].as_array().unwrap() {
        assert_ne!(file["layer"], "full");
    }
    let asked_more = json_at(&format!("{orchestra}_files.json?listen=full")).await;
    assert_eq!(
        (asked_more["listen"].as_str(), asked_more["max"].as_str()),
        (Some("reference"), Some("reference"))
    );
    assert_eq!(listed_paths(&asked_more), paths);
    let page_listing = json_at(&format!("{page_only}_files.json")).await;
    assert!(
        listed_paths(&page_listing)
            .iter()
            .all(|p| p.contains("/Charts/"))
    );
    assert_eq!(
        get(&format!("{full}_files.json?listen=stems"), None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // Narrowed, the share's own link is held at once; an audience never
    // plays past it.
    let share = links
        .update(share.code.clone(), ShareChange::Listen(Listen::Reference))
        .await
        .unwrap();
    assert_eq!(
        get(&format!("{full}{bass}"), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&format!("{strings}Songs/Noel/Media/Proxies/Bass.ogg"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let share = links
        .audience_listen(
            share.code.clone(),
            "orchestra".to_owned(),
            Some(Listen::Full),
        )
        .await
        .unwrap();
    assert_eq!(
        share.audience_named("orchestra").unwrap().listen,
        Some(Listen::Full)
    );
    assert_eq!(
        get(&format!("{orchestra}{bass}"), None).await.0,
        StatusCode::FORBIDDEN
    );

    // Its domain is the share's own link, held the same.
    links
        .update(
            share.code.clone(),
            ShareChange::Domain(Some("songs.example.test".to_owned())),
        )
        .await
        .unwrap();
    let mut seen = 0;
    for _ in 0..200 {
        let (status, _) = raw_get(addr, "/joy/joy/", "songs.example.test").await;
        if status == 200 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        seen += 1;
    }
    assert!(seen < 200, "the domain never served");
    let (status, text) = raw_get(addr, &format!("/{bass}"), "songs.example.test").await;
    assert_eq!(status, 403, "{text}");
    let (status, text) = raw_get(
        addr,
        "/Songs/Joy/Media/Proxies/bass.ogg",
        "songs.example.test",
    )
    .await;
    assert_ne!(status, 200, "{text}");
    let (status, _) = raw_get(addr, &format!("/{reference}"), "songs.example.test").await;
    assert_eq!(status, 200);

    // The extension gone, its rules are kept: the stem stays held back
    // (#540's review; `an_uninstalled_extensions_rules_are_kept_until_forgotten`).
    std::fs::remove_dir_all(w.tmp.path().join("host/data/extensions/demo.songs")).unwrap();
    assert_eq!(
        get(&format!("{page_only}{bass}"), None).await.0,
        StatusCode::FORBIDDEN
    );

    w.host.stop().await;
}

/// A share note with no `listen` (one written before layers, or one that
/// lost the field) plays to Reference, the default, and its audience to
/// the share's: access fails closed. Links from before layers are lowered;
/// none was handed out (the hosted node was off), so nothing in use
/// changes.
#[tokio::test(flavor = "multi_thread")]
async fn a_share_note_without_a_layer_plays_to_reference() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    seed_songs(&vault);
    // A share note as a node wrote it before #528, byte for byte: no
    // `listen` on the share or its audience.
    let (code, band) = (
        architect_spaces::share::code_from([21; 10]),
        architect_spaces::share::code_from([22; 10]),
    );
    touch(
        &vault,
        &format!("_architect/shares/{code}.md"),
        &format!(
            "---\npublish: false\n---\n# Share {code}\n\n```json\n{{\"code\":\"{code}\",\"vault\":\"notes\",\"scope\":{{\"Folder\":\"Songs\"}},\"title\":null,\"audience\":\"Link\",\"access\":\"Read\",\"expires_at\":null,\"revoked_at\":null,\"created_at\":7,\"created_by\":\"u1\",\"domain\":null,\"audiences\":[{{\"name\":\"band\",\"filter\":\"\",\"code\":\"{band}\",\"created_at\":8}}]}}\n```\n"
        ),
    );
    let stem = "Songs/Joy/Media/Proxies/Bass.ogg";
    for at in [&code, &band] {
        let url = w.url(&format!("/s/{at}/"));
        page_with(&format!("{url}joy/joy/"), None, "Joy").await;
        let (status, headers, bytes) = get_range(&format!("{url}{stem}"), Some("bytes=0-3")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{at}");
        assert_eq!(headers["x-architect-layer"], "full");
        assert!(!String::from_utf8_lossy(&bytes).contains("STEM"));
        let reference = "Songs/Joy/Media/Proxies/Reference.ogg";
        let (status, _, bytes) = get_range(&format!("{url}{reference}"), Some("bytes=0-8")).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT, "{at}");
        assert_eq!(bytes, b"REFERENCE");
        let listing = json_at(&format!("{url}_files.json")).await;
        assert_eq!(listing["max"], "reference");
        assert!(!listed_paths(&listing).contains(&stem.to_owned()));
    }
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let old = links
        .shares()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.code == code)
        .unwrap();
    assert_eq!(old.listen, Listen::Reference);
    assert_eq!(old.audiences[0].listen, None);
    assert_eq!(old.listen_for(Some(&old.audiences[0])), Listen::Reference);

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_share_actions_set_and_read_how_far_a_link_plays() {
    use std::sync::Arc;

    use architect::action::ActionList;
    use architect_accounts::{AccountStore, Accounts, MemorySecrets};
    use serde_json::{Value, json};

    let w = world().await;
    let store = Arc::new(AccountStore::in_memory(Arc::new(MemorySecrets::new())));
    let accounts = Accounts::new(store, Issuer::new(&w.issuer), "architect-test").without_browser();
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &accounts).unwrap();
    accounts
        .sign_in_password("grace@local.test", auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    accounts.refresh_orgs().await.unwrap();
    list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap();
    let run = |id: &'static str, args: Value| {
        let list = Arc::clone(&list);
        async move {
            list.run_json_async(id, args.to_string().as_bytes())
                .await
                .map(|out| serde_json::from_slice::<Value>(&out).unwrap())
                .map_err(|e| format!("{e:?}"))
        }
    };
    let host = format!("http://{}", w.host.http);
    // A new link plays to Reference unless told otherwise.
    let made = run("SHARE_CREATE", json!({"folder": "Set", "host": host}))
        .await
        .unwrap();
    assert_eq!(made["share"]["listen"], "reference");
    let code = made["share"]["code"].as_str().unwrap().to_owned();
    let page = run("SHARE_CREATE", json!({"folder": "Set", "listen": "page"}))
        .await
        .unwrap();
    assert_eq!(page["share"]["listen"], "page");
    assert!(
        run("SHARE_CREATE", json!({"folder": "Set", "listen": "stems"}))
            .await
            .is_err()
    );
    let added = run(
        "SHARE_AUDIENCE_ADD",
        json!({"code": code, "name": "band", "listen": "full"}),
    )
    .await
    .unwrap();
    // Asked for more than the link: it plays to the link's.
    assert_eq!(added["audience"]["listen"], "reference");
    assert_eq!(added["audience"]["listen_set"], "full");
    let changed = run("SHARE_UPDATE", json!({"code": code, "listen": "full"}))
        .await
        .unwrap();
    assert_eq!(changed["share"]["listen"], "full");
    assert_eq!(changed["share"]["audiences"][0]["listen"], "full");
    let narrowed = run(
        "SHARE_AUDIENCE_UPDATE",
        json!({"code": code, "name": "band", "listen": "page"}),
    )
    .await
    .unwrap();
    assert_eq!(narrowed["audience"]["listen"], "page");
    let back = run(
        "SHARE_AUDIENCE_UPDATE",
        json!({"code": code, "name": "BAND", "listen": "share"}),
    )
    .await
    .unwrap();
    assert_eq!(back["audience"]["listen"], "full");
    assert_eq!(back["audience"]["listen_set"], Value::Null);
    assert!(
        run(
            "SHARE_AUDIENCE_UPDATE",
            json!({"code": code, "name": "nobody", "listen": "page"})
        )
        .await
        .is_err()
    );
    let rows = run("SHARE_AUDIENCES", json!({"code": code})).await.unwrap();
    assert_eq!(rows["audiences"][0]["listen"], "full");
    // Kept layer rules: none here; forgetting one that is not kept fails.
    let kept = run("SHARE_LAYERS_KEPT", json!({})).await.unwrap();
    assert_eq!(kept["kept"], json!([]));
    assert!(
        run("SHARE_LAYERS_FORGET", json!({"extension": "demo.nothing"}))
            .await
            .is_err()
    );

    w.host.stop().await;
}

/// A file is in the layer of every note that links it, in the share's
/// scope or not, hidden or not: a plain note in the scope linking a stem
/// does not bring it down to the page.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_placed_by_every_note_that_links_it() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    touch(
        &vault,
        "Loose/Song.md",
        "---\ntype: [song, live]\npublish: false\n---\n![[Media/Proxies/Stem.ogg]]\n",
    );
    touch(
        &vault,
        "Loose/notes.md",
        "# Notes\n\nThe stem: ![[Media/Proxies/Stem.ogg]]\n",
    );
    touch(&vault, "Loose/Media/Proxies/Stem.ogg", "STEM LOOSE");
    let links: ShareLinksClient = w.lane(&w.grace).await;
    for scope in [
        ShareScope::Folder("Loose".to_owned()),
        ShareScope::Notes(vec!["Loose/notes.md".to_owned()]),
    ] {
        let share = links
            .create(NewShare {
                vault: "notes".to_owned(),
                scope: scope.clone(),
                title: None,
                audience: Audience::Link,
                access: ShareAccess::Read,
                expires_at: None,
                domain: None,
                listen: Listen::Reference,
            })
            .await
            .unwrap();
        let url = w.url(&format!("/s/{}/", share.code));
        page_with(&url, None, "<!DOCTYPE html>").await;
        let (status, headers, body) =
            get(&format!("{url}Loose/Media/Proxies/Stem.ogg"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{scope:?}: {body}");
        assert_eq!(headers["x-architect-layer"], "full");
        assert!(
            !listed_paths(&json_at(&format!("{url}_files.json")).await)
                .iter()
                .any(|p| p.contains("Stem"))
        );
        let full = links
            .update(share.code.clone(), ShareChange::Listen(Listen::Full))
            .await
            .unwrap();
        assert_eq!(full.listen, Listen::Full);
        // Raised: served once the link is built again (a moment).
        page_with(
            &format!("{url}Loose/Media/Proxies/Stem.ogg"),
            None,
            "STEM LOOSE",
        )
        .await;
        assert_eq!(
            get(&format!("{url}Loose/Media/Proxies/Stem.ogg"), None)
                .await
                .0,
            StatusCode::OK
        );
    }
    w.host.stop().await;
}

// ── Layers: what must never open a link (#540's review) ──────────────

/// The owner's manifest for `demo.songs`, written whole.
const SONG_LAYERS: &str = "id = \"demo.songs\"\nname = \"Songs\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
     [[share_layers]]\nnote_type = \"song\"\n\
     reference = [\"*.session\", \"Media/Proxies/Reference*\"]\n\
     full = [\"Media/Proxies/**\"]\n";

/// A Reference link of `Songs` (the songs seeded, the owner's rules
/// installed); its URL once its pages are built.
async fn reference_link(w: &World) -> (Share, String) {
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    songs_link(w).await
}

/// A Reference link of `Songs` (the songs seeded; the rules are the
/// caller's); its URL once its pages are built.
async fn songs_link(w: &World) -> (Share, String) {
    let vault = w.tmp.path().join("host/data/vaults/notes");
    seed_songs(&vault);
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs".to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::Reference,
        })
        .await
        .unwrap();
    let url = w.url(&format!("/s/{}/", share.code));
    page_with(&format!("{url}joy/joy/"), None, "Joy").await;
    (share, url)
}

/// Wait until the stem opens on `url` (206): a file that opens (a layer
/// raised, a rule lowered or forgotten) is served once the link is built
/// again, a moment later; one that closes is refused at once.
async fn stem_opens(url: &str) -> StatusCode {
    let mut last = StatusCode::NOT_FOUND;
    for _ in 0..200 {
        last = stem_status(url).await;
        if last == StatusCode::PARTIAL_CONTENT {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

/// What the link answers for the Full stem (and that a refusal hands
/// out none of its bytes).
async fn stem_status(url: &str) -> StatusCode {
    let (status, _, bytes) = get_range(
        &format!("{url}Songs/Joy/Media/Proxies/Bass.ogg"),
        Some("bytes=0-3"),
    )
    .await;
    if status != StatusCode::PARTIAL_CONTENT && status != StatusCode::OK {
        assert!(!String::from_utf8_lossy(&bytes).contains("STEM"));
    }
    status
}

/// Install `aaa.other` (asked before `demo.songs`) with `rules`.
fn install_other(w: &World, rules: &str) {
    touch(
        &w.tmp.path().join("host/data/extensions/aaa.other"),
        "extension.toml",
        &format!(
            "id = \"aaa.other\"\nname = \"Other\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n{rules}"
        ),
    );
}

/// The owner's manifest.
fn owner(w: &World) -> PathBuf {
    w.tmp
        .path()
        .join("host/data/extensions/demo.songs/extension.toml")
}

#[tokio::test(flavor = "multi_thread")]
async fn another_extensions_page_rule_never_lowers_a_file() {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    assert_eq!(
        stem_status(&url).await,
        StatusCode::FORBIDDEN,
        "the baseline"
    );
    install_other(
        &w,
        "[[share_layers]]\nnote_type = \"song\"\npage = [\"**\"]\n",
    );
    assert_eq!(stem_status(&url).await, StatusCode::FORBIDDEN);
    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn another_extensions_rule_that_matches_nothing_never_lowers_a_file() {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    install_other(
        &w,
        "[[share_layers]]\nnote_type = \"song\"\nreference = [\"nothing\"]\n",
    );
    assert_eq!(stem_status(&url).await, StatusCode::FORBIDDEN);
    w.host.stop().await;
}

/// With the owner's manifest unreadable, where the stem is is unknown:
/// every attached file is held back below Full, and the listings name
/// none of them, until it is fixed.
async fn an_unreadable_owner_holds_everything_back(broken: &str) {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    std::fs::write(owner(&w), broken).unwrap();
    // Unknown: a link below Full answers nothing (#540's third review),
    // its pages included, until it is known again.
    assert_eq!(stem_status(&url).await, StatusCode::SERVICE_UNAVAILABLE);
    let (status, _, _) = get(&format!("{url}Songs/Joy/Joy.session"), None).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "unknown: held back too"
    );
    assert_eq!(
        get(&format!("{url}_files.json"), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (_, _, files) = get(&format!("{url}_files/"), None).await;
    assert!(
        !files.contains("Bass.ogg") && !files.contains("Joy.session"),
        "{files}"
    );
    let code = url.trim_matches('/').rsplit('/').next().unwrap().to_owned();
    assert!(
        guest_of(&url)
            .files(code, String::new(), None)
            .await
            .is_err()
    );
    let (status, _, zip) = get_bytes(&format!("{url}_archive.zip?by=order")).await;
    assert!(
        status != StatusCode::OK || !String::from_utf8_lossy(&zip).contains("STEM"),
        "{status}"
    );
    let (_, _, page) = get(&format!("{url}joy/joy/"), None).await;
    assert!(
        !page.contains("site-queue") || !page.contains("Bass.ogg"),
        "{page}"
    );
    // Fixed: as before, once the link is built again.
    std::fs::write(owner(&w), SONG_LAYERS).unwrap();
    assert_eq!(stem_status(&url).await, StatusCode::FORBIDDEN);
    page_with(
        &format!("{url}Songs/Joy/Joy.session"),
        None,
        "session of Joy",
    )
    .await;
    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owners_manifest_with_a_bad_kind_holds_everything_back() {
    an_unreadable_owner_holds_everything_back(
        &SONG_LAYERS.replace("kind = \"data\"", "kind = \"bogus\""),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_half_written_manifest_holds_everything_back() {
    an_unreadable_owner_holds_everything_back(SONG_LAYERS.get(..SONG_LAYERS.len() / 2).unwrap())
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_uninstalled_extensions_rules_are_kept_until_forgotten() {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    std::fs::remove_dir_all(w.tmp.path().join("host/data/extensions/demo.songs")).unwrap();
    assert_eq!(
        stem_status(&url).await,
        StatusCode::FORBIDDEN,
        "uninstalled"
    );
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let kept = links.kept_layers().await.unwrap().kept;
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].extension, "demo.songs");
    assert_eq!(kept[0].folder, "demo.songs");
    assert_eq!(kept[0].note_types, ["song"]);
    assert!(
        links
            .forget_layers("demo.nothing".to_owned())
            .await
            .is_err()
    );
    // Kept across a restart of the node: they are in its data dir.
    let kept_file = w.tmp.path().join("host/data/share-layers.json");
    assert!(
        std::fs::read_to_string(&kept_file)
            .unwrap()
            .contains("demo.songs")
    );
    links.forget_layers("demo.songs".to_owned()).await.unwrap();
    assert!(links.kept_layers().await.unwrap().kept.is_empty());
    assert_eq!(
        stem_opens(&url).await,
        StatusCode::PARTIAL_CONTENT,
        "forgotten: a page file"
    );
    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reinstalled_extension_places_exactly_what_it_declares() {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    let dir = w.tmp.path().join("host/data/extensions/demo.songs");
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(stem_status(&url).await, StatusCode::FORBIDDEN);
    // Back, declaring the Bass stem part of the reference now.
    touch(
        &dir,
        "extension.toml",
        &SONG_LAYERS.replace(
            "reference = [\"*.session\", \"Media/Proxies/Reference*\"]",
            "reference = [\"*.session\", \"Media/Proxies/Reference*\", \"Media/Proxies/Bass*\"]",
        ),
    );
    assert_eq!(stem_opens(&url).await, StatusCode::PARTIAL_CONTENT);
    let (status, headers, _) = get(
        &format!("{url}Songs/Joy/Media/Proxies/G%C3%A4nse.ogg"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(headers["x-architect-layer"], "full");
    let links: ShareLinksClient = w.lane(&w.grace).await;
    // Installed, its rules are its manifest's: nothing kept, nothing to
    // forget.
    assert!(links.kept_layers().await.unwrap().kept.is_empty());
    assert!(links.forget_layers("demo.songs".to_owned()).await.is_err());
    w.host.stop().await;
}

/// The status, the layer header and the body of a raw answer.
fn answer_of(text: &str) -> (u16, bool, String) {
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text, ""));
    (
        status,
        head.to_ascii_lowercase().contains("x-architect-layer"),
        body.to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_says_nothing_about_files_outside_it() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    seed_songs(&vault);
    let links: ShareLinksClient = w.lane(&w.grace).await;
    // Joy alone, at Reference: Noel's stem is in the vault, not the link.
    let joy = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs/Joy".to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: Some("joy.example.test".to_owned()),
            listen: Listen::Reference,
        })
        .await
        .unwrap();
    let joy = links
        .add_audience(
            joy.code.clone(),
            NewAudience {
                name: "all".to_owned(),
                filter: String::new(),
                listen: None,
            },
        )
        .await
        .unwrap();
    // Every song, its strings audience at Reference: Joy's stem is the
    // share's, not the audience's.
    let all = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs".to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Comment,
            expires_at: None,
            domain: None,
            listen: Listen::Full,
        })
        .await
        .unwrap();
    let all = links
        .add_audience(
            all.code.clone(),
            NewAudience {
                name: "strings".to_owned(),
                filter: "section == \"strings\"".to_owned(),
                listen: Some(Listen::Reference),
            },
        )
        .await
        .unwrap();
    let addr = w.host.http;
    let (joy_own, joy_all, strings) = (
        format!("/s/{}/", joy.code),
        format!("/s/{}/", joy.audience_named("all").unwrap().code),
        format!("/s/{}/", all.audience_named("strings").unwrap().code),
    );
    for at in [&joy_own, &joy_all, &strings] {
        page_with(&w.url(at), None, "<!DOCTYPE html>").await;
    }
    let mut domain_up = false;
    for _ in 0..200 {
        if raw_get(addr, "/", "joy.example.test").await.0 == 200 {
            domain_up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(domain_up);
    let cases = [
        // (host, link, a file outside it that exists, one that does not)
        (
            "127.0.0.1",
            joy_own.as_str(),
            "Songs/Noel/Media/Proxies/Bass.ogg",
            "Songs/Nope/Media/Proxies/Bass.ogg",
        ),
        (
            "127.0.0.1",
            joy_all.as_str(),
            "Songs/Noel/Media/Proxies/Bass.ogg",
            "Songs/Nope/Media/Proxies/Bass.ogg",
        ),
        (
            "joy.example.test",
            "/",
            "Songs/Noel/Media/Proxies/Bass.ogg",
            "Songs/Nope/Media/Proxies/Bass.ogg",
        ),
        (
            "127.0.0.1",
            strings.as_str(),
            "Songs/Joy/Media/Proxies/Bass.ogg",
            "Songs/Nope/Media/Proxies/Bass.ogg",
        ),
        (
            "127.0.0.1",
            strings.as_str(),
            "Songs/Joy/Joy.session",
            "Songs/Nope/Nope.session",
        ),
        ("127.0.0.1", strings.as_str(), "joy/joy/", "nope/nope/"),
    ];
    for (host, link, there, nowhere) in cases {
        let (status, layer, body) =
            answer_of(&raw_get(addr, &format!("{link}{there}"), host).await.1);
        let (status2, layer2, body2) =
            answer_of(&raw_get(addr, &format!("{link}{nowhere}"), host).await.1);
        assert_eq!(status, 404, "{host}{link}{there}: {body}");
        assert_eq!(
            (status, layer, &body),
            (status2, layer2, &body2),
            "{host}{link}{there}"
        );
    }
    // A comment on another audience's page and on a page that is nowhere:
    // answered the same.
    let comment = |page: &'static str| {
        let url = w.url(&format!("{strings}_comment"));
        async move {
            let res = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap()
                .post(url)
                .form(&[("page", page), ("text", "hi")])
                .send()
                .await
                .unwrap();
            (res.status(), res.text().await.unwrap())
        }
    };
    let (theirs, nowhere) = (comment("/joy/joy/").await, comment("/nope/nope/").await);
    assert_eq!(theirs.0, StatusCode::NOT_FOUND);
    assert_eq!(theirs, nowhere);
    // Its own page takes none either: the page's comment form went with
    // the page (#646).
    assert_eq!(comment("/noel/noel/").await, nowhere);
    // In the link, above it: still 403 with the layer it needs.
    let (status, layer, _) = answer_of(
        &raw_get(
            addr,
            &format!("{joy_own}Songs/Joy/Media/Proxies/Bass.ogg"),
            "127.0.0.1",
        )
        .await
        .1,
    );
    assert_eq!((status, layer), (403, true));

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thousand_requests_place_the_files_once() {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    let reference = format!("{url}Songs/Joy/Media/Proxies/Reference.ogg");
    let stem = format!("{url}Songs/Joy/Media/Proxies/Bass.ogg");
    assert_eq!(get(&stem, None).await.0, StatusCode::FORBIDDEN);
    // Settled first: the share's own note reaches the vault a moment after
    // it is made, and a vault change is placed again (once), as it must be.
    let mut settled = false;
    for _ in 0..50 {
        let at = architect_desktop::node::share_layers::placements();
        let _ = get(&stem, None).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        let _ = get(&stem, None).await;
        if architect_desktop::node::share_layers::placements() == at {
            settled = true;
            break;
        }
    }
    assert!(settled, "placement never settled");
    let before = architect_desktop::node::share_layers::placements();
    let client = reqwest::Client::new();
    for i in 0..1000 {
        let at = if i % 2 == 0 { &reference } else { &stem };
        let res = client
            .get(at)
            .header("range", format!("bytes={}-{}", i % 5, i % 5 + 2))
            .send()
            .await
            .unwrap();
        assert!(
            res.status() == StatusCode::PARTIAL_CONTENT || res.status() == StatusCode::FORBIDDEN
        );
    }
    let placed = architect_desktop::node::share_layers::placements() - before;
    assert!(placed <= 1, "{placed} placements for 1000 requests");

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_files_name_is_in_nothing_built_for_its_link() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    seed_songs(&vault);
    // A plain link to the stem too, and lyrics held for Full.
    touch(
        &vault,
        "Songs/Joy/Joy.md",
        "---\ntitle: Joy\ntype: song\nsection: band\n---\n# Joy\n\n![[Joy.session]]\n\n![[Media/Proxies/Reference.ogg]]\n\n![[Media/Proxies/Bass.ogg]]\n\nThe [[Media/Proxies/Bass.ogg|bass stem]] and [the other stem](Media/Proxies/Bass.ogg).\n\n![[Charts/Joy.pdf]]\n",
    );
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs".to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::Full,
        })
        .await
        .unwrap();
    let share = links
        .add_audience(
            share.code.clone(),
            NewAudience {
                name: "orchestra".to_owned(),
                filter: String::new(),
                listen: Some(Listen::Reference),
            },
        )
        .await
        .unwrap();
    let orchestra = share.audience_named("orchestra").unwrap().code.clone();
    let full = w.url(&format!("/s/{}/", share.code));
    let low = w.url(&format!("/s/{orchestra}/"));
    page_with(&format!("{full}joy/joy/"), None, "Bass.ogg").await;
    let page = page_with(&format!("{low}joy/joy/"), None, "Reference.ogg").await;
    for name in ["Bass", "nse.ogg", "bass stem", "the other stem"] {
        assert!(!page.contains(name), "{name} in the page: {page}");
    }
    let built = w.tmp.path().join("host/data/shares").join(&share.code);
    let named = |dir: &Path| -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(text) = std::fs::read_to_string(&path) {
                    if [
                        "Bass.ogg",
                        "G\u{e4}nse.ogg",
                        "Ga\u{308}nse.ogg",
                        "bass stem",
                    ]
                    .iter()
                    .any(|n| text.contains(n))
                    {
                        out.push(path.display().to_string());
                    }
                }
            }
        }
        out
    };
    let held = named(&built.join("audiences").join(&orchestra));
    assert!(held.is_empty(), "held files named in {held:?}");
    assert!(
        !named(&built.join("site")).is_empty(),
        "the Full link names them"
    );
    // Lowered, the share's own link is built again without them.
    links
        .update(share.code.clone(), ShareChange::Listen(Listen::Reference))
        .await
        .unwrap();
    let mut gone = false;
    for _ in 0..200 {
        if named(&built.join("site")).is_empty() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "still named: {:?}", named(&built.join("site")));

    w.host.stop().await;
}

// ── Layers: races and installs (#540's second review) ────────────────

/// What the held stem answered across `requests` on `client`: how many
/// were served (200/206, or its bytes), of how many answered.
async fn stem_hits(client: &reqwest::Client, url: &str, requests: usize) -> (usize, usize) {
    let (mut served, mut asked) = (0_usize, 0_usize);
    for i in 0..requests {
        let Ok(res) = client
            .get(format!("{url}Songs/Joy/Media/Proxies/Bass.ogg"))
            .header(
                "range",
                format!("bytes={}-{}", i % 3, (i % 3).saturating_add(3)),
            )
            .send()
            .await
        else {
            continue;
        };
        asked = asked.saturating_add(1);
        let status = res.status();
        let bytes = res.bytes().await.unwrap_or_default();
        if status.is_success() || String::from_utf8_lossy(&bytes).contains("STEM") {
            served = served.saturating_add(1);
        }
    }
    (served, asked)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_file_is_never_served_while_its_link_is_swapped_in() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let w = world().await;
    let (share, url) = reference_link(&w).await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    let own = w
        .tmp
        .path()
        .join("host/data/shares")
        .join(&share.code)
        .join("site");
    let swapping = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicUsize::new(0));
    {
        let (swapping, swaps, own) = (Arc::clone(&swapping), Arc::clone(&swaps), own.clone());
        // Between the two renames the link's folder is not there: hold
        // that moment for 100 ms (under a request's wait for the folder).
        architect_sites::set_swap_hook(Some(Arc::new(move |out: &Path| {
            if out == own {
                swapping.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
                swapping.store(false, Ordering::SeqCst);
                swaps.fetch_add(1, Ordering::SeqCst);
            }
        })));
    }
    let mut asked = 0_usize;
    for round in 0..6 {
        touch(&vault, &format!("Songs/Joy/edit-{round}.md"), "# An edit\n");
        let start = std::time::Instant::now();
        while !swapping.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(
            swapping.load(Ordering::SeqCst),
            "round {round}: no swap seen"
        );
        let tries: Vec<_> = (0..6)
            .map(|_| {
                let url = url.clone();
                tokio::spawn(async move { stem_hits(&reqwest::Client::new(), &url, 1).await })
            })
            .collect();
        for t in tries {
            let (served, n) = t.await.unwrap();
            assert_eq!(
                served, 0,
                "round {round}: the held stem was served mid-swap"
            );
            asked = asked.saturating_add(n);
        }
    }
    architect_sites::set_swap_hook(None);
    assert!(swaps.load(Ordering::SeqCst) >= 6);
    assert_eq!(asked, 36);
    w.host.stop().await;
}

/// The reviewer's loop: the vault edited every 300 ms (each a rebuild and
/// a folder swap), 8 clients asking for the held stem for 30 s: never
/// once served.
#[tokio::test(flavor = "multi_thread")]
async fn a_held_file_is_never_served_under_rebuilds_and_load() {
    let w = world().await;
    let (_, url) = reference_link(&w).await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    let until = std::time::Instant::now() + Duration::from_secs(30);
    let editor = tokio::spawn(async move {
        let mut i = 0_u32;
        while std::time::Instant::now() < until {
            touch(&vault, "Songs/Joy/edit.md", &format!("# Edit {i}\n"));
            i = i.saturating_add(1);
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        i
    });
    let clients: Vec<_> = (0..8)
        .map(|_| {
            let url = url.clone();
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let (mut served, mut asked) = (0_usize, 0_usize);
                while std::time::Instant::now() < until {
                    let (s, a) = stem_hits(&client, &url, 20).await;
                    served = served.saturating_add(s);
                    asked = asked.saturating_add(a);
                }
                (served, asked)
            })
        })
        .collect();
    let (mut served, mut asked) = (0_usize, 0_usize);
    for c in clients {
        let (s, a) = c.await.unwrap();
        served = served.saturating_add(s);
        asked = asked.saturating_add(a);
    }
    let edits = editor.await.unwrap();
    assert!(edits >= 50, "{edits} edits");
    assert!(asked > 1000, "{asked} requests");
    assert_eq!(
        served, 0,
        "the held stem was served {served} times of {asked}"
    );
    w.host.stop().await;
}

/// A development copy: the slot holds only `installed.toml`, its
/// `link` the package's folder (as `EXT_INSTALL` of a path writes it).
fn install_linked(w: &World, package: &Path) {
    touch(
        &w.tmp.path().join("host/data/extensions/demo.songs"),
        "installed.toml",
        &format!(
            "id = \"demo.songs\"\nversion = \"0.1.0\"\nkind = \"data\"\nsource = \"path\"\nlink = {:?}\n",
            package.display().to_string()
        ),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_extensions_rules_place_its_files() {
    let w = world().await;
    let package = w.tmp.path().join("dev/demo.songs");
    touch(&package, "extension.toml", SONG_LAYERS);
    install_linked(&w, &package);
    let (_, url) = songs_link(&w).await;
    assert_eq!(stem_status(&url).await, StatusCode::FORBIDDEN);
    let listing = json_at(&format!("{url}_files.json")).await;
    assert!(
        !listed_paths(&listing).iter().any(|p| p.contains("Bass")),
        "{listing}"
    );
    // Edited where it is linked from: read again.
    std::fs::write(
        package.join("extension.toml"),
        SONG_LAYERS.replace(
            "reference = [\"*.session\", \"Media/Proxies/Reference*\"]",
            "reference = [\"*.session\", \"Media/Proxies/Reference*\", \"Media/Proxies/Bass*\"]",
        ),
    )
    .unwrap();
    assert_eq!(stem_opens(&url).await, StatusCode::PARTIAL_CONTENT);
    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_slot_whose_package_does_not_read_holds_everything_back() {
    let w = world().await;
    install_linked(&w, &w.tmp.path().join("dev/gone"));
    seed_songs(&w.tmp.path().join("host/data/vaults/notes"));
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs".to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::Reference,
        })
        .await
        .unwrap();
    let url = w.url(&format!("/s/{}/", share.code));
    // Unknown from the start: nothing below Full is answered, built or not.
    for file in ["Songs/Joy/Media/Proxies/Bass.ogg", "Songs/Joy/Joy.session"] {
        assert_eq!(
            served_over(&url, file, 20, Duration::from_millis(50)).await,
            0,
            "{file}"
        );
        assert_eq!(
            get(&format!("{url}{file}"), None).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    assert_eq!(
        get(&format!("{url}_files.json"), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    w.host.stop().await;
}

// ── Layers: what the vault cannot read (#540's third review) ─────────

/// Ask for `path` on `url` `rounds` times, `every` apart; how many times
/// it was served (2xx, or the stem's bytes).
async fn served_over(url: &str, path: &str, rounds: usize, every: Duration) -> usize {
    let client = reqwest::Client::new();
    let mut served = 0_usize;
    for _ in 0..rounds {
        if let Ok(res) = client.get(format!("{url}{path}")).send().await {
            let ok = res.status().is_success();
            let bytes = res.bytes().await.unwrap_or_default();
            if ok || String::from_utf8_lossy(&bytes).contains("STEM") {
                served = served.saturating_add(1);
            }
        }
        tokio::time::sleep(every).await;
    }
    served
}

/// A typed note the vault cannot read (a stray Latin-1 byte) is not
/// dropped from placement: where its files are is unknown, and the stem
/// it places stays held back even though a readable note links it too.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_typed_note_holds_its_files_back() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    touch(
        &vault,
        "Songs/Joy/notes.md",
        "# Notes\n\nThe bass: ![[Media/Proxies/Bass.ogg]]\n",
    );
    let (_, url) = reference_link(&w).await;
    let stem = "Songs/Joy/Media/Proxies/Bass.ogg";
    assert_eq!(
        get(&format!("{url}{stem}"), None).await.0,
        StatusCode::FORBIDDEN
    );
    let joy = vault.join("Songs/Joy/Joy.md");
    let mut bytes = std::fs::read(&joy).unwrap();
    bytes.extend_from_slice(b"\nCaf\xe9\n");
    std::fs::write(&joy, bytes).unwrap();
    // Long enough for the node to hear of it and build the link again.
    assert_eq!(
        served_over(&url, stem, 60, Duration::from_millis(50)).await,
        0
    );
    w.host.stop().await;
}

/// While the vault cannot be read, a link below Full answers nothing
/// from a folder built before its layer was lowered (#540's third review:
/// it served the old folder's stems).
#[tokio::test(flavor = "multi_thread")]
async fn a_lowered_link_serves_nothing_while_the_vault_cannot_be_read() {
    let w = world().await;
    install_song_layers(&w.tmp.path().join("host/data/extensions/demo.songs"));
    seed_songs(&w.tmp.path().join("host/data/vaults/notes"));
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Songs".to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::Full,
        })
        .await
        .unwrap();
    let url = w.url(&format!("/s/{}/", share.code));
    let stem = "Songs/Joy/Media/Proxies/Bass.ogg";
    page_with(&format!("{url}{stem}"), None, "STEM BASS").await;
    architect_desktop::node::shares::fail_vault_views_for_tests(true);
    links
        .update(share.code.clone(), ShareChange::Listen(Listen::Reference))
        .await
        .unwrap();
    let served = served_over(&url, stem, 40, Duration::from_millis(50)).await;
    // Unavailable for now: stored by no cache, and when to ask again.
    let (status, headers, _) = get(&format!("{url}{stem}"), None).await;
    architect_desktop::node::shares::fail_vault_views_for_tests(false);
    assert_eq!(served, 0, "the old folder's stem was served");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["retry-after"], "5");
    // Readable again: held back by its layer.
    let mut held = StatusCode::OK;
    for _ in 0..100 {
        held = get(&format!("{url}{stem}"), None).await.0;
        if held == StatusCode::FORBIDDEN {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(held, StatusCode::FORBIDDEN);
    w.host.stop().await;
}

/// A file an in-scope note stopped linking, now placed above the link's
/// layer by a note the link does not list, is not served from the folder
/// built before: it answers as a path that is nowhere.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_left_the_list_and_is_now_held_is_not_served() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    let file = "Songs/Joy/Media/Proxies/Loose.ogg";
    touch(&vault, file, "STEM LOOSE");
    touch(
        &vault,
        "Songs/Joy/notes.md",
        "# Notes\n\n![[Media/Proxies/Loose.ogg]]\n",
    );
    let (_, url) = reference_link(&w).await;
    // A page file now: built into the link.
    page_with(&format!("{url}{file}"), None, "STEM LOOSE").await;
    // A hidden song note takes it (Full: 403 while the link lists it),
    // then the plain note lets go of it (Full, and not the link's: 404).
    touch(
        &vault,
        "Songs/Joy/Hidden.md",
        "---\ntype: song\npublish: false\n---\n![[Media/Proxies/Loose.ogg]]\n",
    );
    let mut held = StatusCode::OK;
    for _ in 0..100 {
        held = get(&format!("{url}{file}"), None).await.0;
        if held == StatusCode::FORBIDDEN {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(held, StatusCode::FORBIDDEN);
    touch(
        &vault,
        "Songs/Joy/notes.md",
        "# Notes\n\nNothing attached now.\n",
    );
    assert_eq!(
        served_over(&url, file, 60, Duration::from_millis(20)).await,
        0
    );
    let (status, headers, _) = get(&format!("{url}{file}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!headers.contains_key("x-architect-layer"));
    w.host.stop().await;
}

// ── A link's guests (#629) ───────────────────────────────────────────

/// The status a guest's call answers with, as the page's would be.
fn guest_status<T>(
    answer: &Result<T, architect::ClientError<architect_spaces::GuestFault>>,
) -> u16 {
    use architect_spaces::GuestFault;
    match answer {
        Ok(_) => 200,
        Err(architect::ClientError::App(GuestFault::Gone)) => 404,
        Err(architect::ClientError::App(GuestFault::SignIn)) => 401,
        Err(architect::ClientError::App(GuestFault::Denied | GuestFault::Held(_))) => 403,
        Err(architect::ClientError::App(GuestFault::Building)) => 503,
        Err(other) => panic!("not an answer of the node's: {other:?}"),
    }
}

fn guest(w: &World) -> architect_spaces::guest::ShareGuestHttpClient {
    architect_spaces::guest::ShareGuestHttpClient::at(w.url(""))
}

/// The guest service of the node at `url`'s origin.
fn guest_of(url: &str) -> architect_spaces::guest::ShareGuestHttpClient {
    let origin = reqwest::Url::parse(url)
        .unwrap()
        .origin()
        .ascii_serialization();
    architect_spaces::guest::ShareGuestHttpClient::at(origin)
}

/// The guest service answers exactly as the link's pages do, for the
/// same visitor: one table over the shares above (a folder link, its
/// two audiences, an org link, an invited link, a revoked and an
/// expired one; nobody, a member, an outsider, a session that is none),
/// each note's page against `read` of that note.
#[tokio::test(flavor = "multi_thread")]
async fn a_guest_reads_exactly_what_the_page_gives_the_same_visitor() {
    let w = world().await;
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let (set, violin, cello) = setlist(&w).await;
    let violin_code = violin
        .trim_start_matches("/s/")
        .trim_end_matches('/')
        .to_owned();
    let cello_code = cello
        .trim_start_matches("/s/")
        .trim_end_matches('/')
        .to_owned();
    let track = || ShareScope::Notes(vec!["Shared/track.md".to_owned()]);
    let org = w.share(track(), Audience::Org, ShareAccess::Read).await;
    let invited = w
        .share(
            track(),
            Audience::Invited(vec!["ada@local.test".to_owned()]),
            ShareAccess::Read,
        )
        .await;
    let revoked = w.share(track(), Audience::Link, ShareAccess::Read).await;
    let expiring = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: track(),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: Some(node::shares::now() + 2),
            domain: None,
            listen: Listen::default(),
        })
        .await
        .unwrap();
    // Every link built.
    page_with(
        &w.url(&format!("{}04-noel-cello/", set.path())),
        None,
        "Noel",
    )
    .await;
    page_with(&w.url(&format!("{violin}01-joy-violin/")), None, "violin").await;
    page_with(&w.url(&format!("{cello}02-joy-cello/")), None, "cello").await;
    page_with(
        &w.url(&format!("/s/{}/", org.code)),
        Some(&w.alan),
        "Mix notes.",
    )
    .await;
    page_with(&w.url(&format!("/s/{}/", revoked.code)), None, "Mix notes.").await;
    links.revoke(revoked.code.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let anon = String::new();
    let garbage = "not-a-session".to_owned();
    // (link code, page below it, note path, who)
    let table: Vec<(&str, &str, &str, &String)> = vec![
        (&set.code, "01-joy-violin/", "Set/01-joy-violin.md", &anon),
        (&set.code, "04-noel-cello/", "Set/04-noel-cello.md", &anon),
        (&set.code, "plan/", "Private/plan.md", &anon),
        (
            &violin_code,
            "01-joy-violin/",
            "Set/01-joy-violin.md",
            &anon,
        ),
        (
            &violin_code,
            "03-silent-violin/",
            "Set/03-silent-violin.md",
            &anon,
        ),
        (&violin_code, "02-joy-cello/", "Set/02-joy-cello.md", &anon),
        (&cello_code, "04-noel-cello/", "Set/04-noel-cello.md", &anon),
        (&cello_code, "01-joy-violin/", "Set/01-joy-violin.md", &anon),
        (&org.code, "", "Shared/track.md", &anon),
        (&org.code, "", "Shared/track.md", &w.alan),
        (&org.code, "", "Shared/track.md", &w.grace),
        (&org.code, "", "Shared/track.md", &w.ada),
        (&org.code, "", "Shared/track.md", &garbage),
        (&invited.code, "", "Shared/track.md", &w.ada),
        (&invited.code, "", "Shared/track.md", &w.alan),
        (&invited.code, "", "Shared/track.md", &anon),
        (&revoked.code, "", "Shared/track.md", &anon),
        (&expiring.code, "", "Shared/track.md", &anon),
        ("aaaaaaaaaaaaaaaa", "", "Shared/track.md", &anon),
    ];
    let client = guest(&w);
    let mut seen = Vec::new();
    for (code, page, path, who) in table {
        let auth = (!who.is_empty()).then_some(who.as_str());
        let page_status = get(&w.url(&format!("/s/{code}/{page}")), auth)
            .await
            .0
            .as_u16();
        let read = client
            .read(code.to_owned(), who.clone(), path.to_owned())
            .await;
        let read_status = guest_status(&read);
        let link_status = guest_status(&client.link(code.to_owned(), who.clone()).await);
        let row = format!(
            "{code} {page} {path} {}",
            if who.is_empty() { "nobody" } else { "someone" }
        );
        assert_eq!(read_status, page_status, "read: {row}");
        if page_status != 404 {
            // The link itself opens to exactly those its pages open to.
            assert_eq!(link_status, page_status, "link: {row}");
        }
        seen.push(page_status);
    }
    // The table covers every answer.
    for status in [200, 401, 403, 404] {
        assert!(seen.contains(&status), "no row answered {status}");
    }
    w.host.stop().await;
}

/// A guest reads a note as plain blocks of what its page says, and the
/// link as its notes in list order; nothing the page leaves out.
#[tokio::test(flavor = "multi_thread")]
async fn a_guest_reads_a_link_as_its_notes_and_a_note_as_its_pages_words() {
    use architect_spaces::BlockKind;
    let w = world().await;
    let share = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    page_with(
        &w.url(&format!("/s/{}/track/", share.code)),
        None,
        "Mix notes.",
    )
    .await;
    let client = guest(&w);
    let link = client
        .link(share.code.clone(), String::new())
        .await
        .unwrap();
    assert_eq!(link.link, format!("/s/{}/", share.code));
    assert_eq!(link.audience, "link");
    let paths: Vec<&str> = link.notes.iter().map(|n| n.path.as_str()).collect();
    assert!(
        paths.contains(&"Shared/track.md") && paths.contains(&"Shared/index.md"),
        "{paths:?}"
    );
    assert!(!paths.iter().any(|p| p.starts_with("Private/")));

    let note = client
        .read(
            share.code.clone(),
            String::new(),
            "Shared/track.md".to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(note.title, "Track");
    assert_eq!(note.page, format!("/s/{}/track/", share.code));
    assert!(note.source.is_none(), "a guest never gets the note's text");
    assert!(
        note.blocks
            .iter()
            .any(|b| b.kind == BlockKind::Paragraph && b.text == "Mix notes."),
        "{:?}",
        note.blocks
    );
    // The album links a private note (plain text there) and embeds a
    // private picture (not on the link): neither comes through.
    let index = client
        .read(
            share.code.clone(),
            String::new(),
            "Shared/index.md".to_owned(),
        )
        .await
        .unwrap();
    let said = format!("{:?}", index.blocks);
    assert!(said.contains("the plan"), "{said}");
    assert!(
        !said.contains("secret") && !said.contains("cover.png") && !said.contains("Private/"),
        "{said}"
    );
    let images: Vec<&str> = index
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Image)
        .filter_map(|b| b.src.as_deref())
        .collect();
    assert_eq!(
        images,
        [format!("/s/{}/Shared/art.png", share.code)],
        "{said}"
    );
    // Every medium a guest is pointed at is served by the link.
    assert_eq!(get(&w.url(images[0]), None).await.0, StatusCode::OK);

    // A note outside the link: the same answer as one that is nowhere.
    let outside = client
        .read(
            share.code.clone(),
            String::new(),
            "Private/plan.md".to_owned(),
        )
        .await;
    assert_eq!(guest_status(&outside), 404);

    // The service's answers are kept by nobody and name no referrer.
    let res = reqwest::Client::new()
        .post(w.url(&format!("/s/{}/_guest/link", share.code)))
        .json(&serde_json::json!({ "token": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["cache-control"], "private, no-store");
    assert_eq!(res.headers()["referrer-policy"], "no-referrer");
    // Any origin (a web app elsewhere) may call it, without credentials.
    assert_eq!(res.headers()["access-control-allow-origin"], "*");
    assert!(
        !res.headers()
            .contains_key("access-control-allow-credentials")
    );
    let pre = reqwest::Client::new()
        .request(
            reqwest::Method::OPTIONS,
            w.url(&format!("/s/{}/_guest/read", share.code)),
        )
        .header("origin", "https://app.example")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), StatusCode::NO_CONTENT);
    assert_eq!(pre.headers()["access-control-allow-methods"], "POST");

    // Revoked: gone, from the next call on.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    links.revoke(share.code.clone()).await.unwrap();
    assert_eq!(
        guest_status(&client.link(share.code.clone(), String::new()).await),
        404
    );
    assert_eq!(
        guest_status(
            &client
                .read(
                    share.code.clone(),
                    String::new(),
                    "Shared/track.md".to_owned()
                )
                .await
        ),
        404
    );
    w.host.stop().await;
}

/// A guest lists exactly the files `_files.json` lists, at the link's
/// layer and never past it.
#[tokio::test(flavor = "multi_thread")]
async fn a_guests_files_are_the_links_at_its_layer() {
    let w = world().await;
    let (share, url) = reference_link(&w).await;
    let client = guest(&w);
    for (asked, query) in [
        (None, ""),
        (Some("page"), "?listen=page"),
        (Some("full"), "?listen=full"),
    ] {
        let files = client
            .files(share.code.clone(), String::new(), asked.map(str::to_owned))
            .await
            .unwrap();
        let mut theirs: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        let json = json_at(&format!("{url}_files.json{query}")).await;
        let mut page: Vec<String> = json["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_owned())
            .collect();
        theirs.sort();
        page.sort();
        assert_eq!(theirs, page, "listen {asked:?}");
        // Never a stem on a Reference link, whatever is asked.
        assert!(!theirs.iter().any(|f| f.contains("Bass")), "{theirs:?}");
        // Every file listed is served by the link.
        for file in &files {
            let status = get(&w.url(&file.url), None).await.0;
            assert_eq!(status, StatusCode::OK, "{}", file.path);
        }
    }
    w.host.stop().await;
}

/// A path matches an association component's `/` pattern as iOS reads
/// it: `*` any run of characters (`/` too), `?` any one, the rest itself;
/// the query is not part of the path.
fn aasa_matches(pattern: &str, path: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| go(rest, &s[i..])),
            Some((b'?', rest)) => !s.is_empty() && go(rest, &s[1..]),
            Some((c, rest)) => s.first() == Some(c) && go(rest, &s[1..]),
        }
    }
    let path = path.split('?').next().unwrap_or_default();
    go(pattern.as_bytes(), path.as_bytes())
}

/// `/.well-known/apple-app-site-association` names the configured apps
/// for the share links' pages alone; with none configured it is not there.
#[tokio::test(flavor = "multi_thread")]
async fn the_node_claims_its_links_for_the_configured_apps_alone() {
    let w = world().await;
    let (status, _, _) = get(&w.url("/.well-known/apple-app-site-association"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    w.host.stop().await;

    let body = node::shares::apple_app_site_association(&[
        "28C2G63DA7.app.fasttrackstudio.architect".to_owned(),
    ])
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let details = &json["applinks"]["details"][0];
    assert_eq!(
        details["appIDs"][0],
        "28C2G63DA7.app.fasttrackstudio.architect"
    );
    let components = details["components"].as_array().unwrap();
    // Sign-in and the link's reserved names are excluded first; then its
    // pages, and nothing else.
    assert_eq!(components[0]["/"], "/s/_*");
    assert_eq!(components[0]["exclude"], true);
    assert_eq!(components[1]["/"], "/s/*/_*");
    assert_eq!(components[1]["exclude"], true);
    assert_eq!(components.last().unwrap()["/"], "/s/*/");
    // As iOS reads the components: the first that matches decides.
    let claims = |path: &str| {
        components
            .iter()
            .find(|c| aasa_matches(c["/"].as_str().unwrap(), path))
            .is_some_and(|c| c["exclude"] != true)
    };
    for page in [
        "/s/merrybrightstack/",
        "/s/merrybrightstack/joy/",
        "/s/merrybrightstack/Songs/Joy/",
    ] {
        assert!(claims(page), "{page} opens the app");
    }
    for not in [
        "/s/_signin/",
        "/s/merrybrightstack/_archive.zip",
        "/s/merrybrightstack/_archive.zip?by=order",
        "/s/merrybrightstack/_files/",
        "/s/merrybrightstack/_files.json",
        "/s/merrybrightstack/_guest/link",
        "/s/merrybrightstack/_live",
        "/s/merrybrightstack/joy/_note.json",
        "/s/merrybrightstack/Songs/Joy.mp3",
        "/s/merrybrightstack/Art/joy.png",
        "/s/merrybrightstack/Charts/Joy.pdf",
        "/s/merrybrightstack",
        "/elsewhere/",
    ] {
        assert!(!claims(not), "{not} stays the browser's");
    }
    assert!(node::shares::apple_app_site_association(&[]).is_none());

    // Configured: served as JSON, no redirect.
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {
        name: "aasa".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: tmp.path().join("data"),
        iroh: IrohMode::Local,
        apple_app_ids: vec!["28C2G63DA7.app.fasttrackstudio.architect".to_owned()],
        ..Config::default()
    };
    let host = node::start_with(config, architect_sites::Blocks::new())
        .await
        .unwrap();
    let res = reqwest::Client::new()
        .get(format!(
            "http://{}/.well-known/apple-app-site-association",
            host.http
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "application/json");
    assert_eq!(res.text().await.unwrap(), body);
    host.stop().await;
}

// ── What a guest's view reads (#648, #669) ───────────────────────────

/// An island whose web bundle claims `setlist` notes with view `list`,
/// reading the note (`embed_note`) or not, its extension declaring
/// `reads`.
fn setlist_island(reads_note: bool, reads: &[&str]) -> architect_sites::SiteIsland {
    use std::borrow::Cow;
    let island = architect_sites::SiteIsland::new(
        "demo.setlist",
        vec![
            (
                "web-setlist.js".to_owned(),
                Cow::Borrowed(b"export function mount(){}".as_slice()),
            ),
            (
                "web-setlist_bg.wasm".to_owned(),
                Cow::Borrowed(b"\0asm\x01\0\0\0".as_slice()),
            ),
        ],
    )
    .web("web-setlist.js")
    .reads(reads.iter().copied());
    if reads_note {
        island.embed_note("list", ["setlist"], ["date"])
    } else {
        island.embed("list", ["setlist"])
    }
}

/// A gig's folder: a setlist that links its song, a note outside the
/// link and its picture, a comment, art, lyrics, audio, a file over the
/// cap, a symlink out of the vault and one inside it to the private
/// picture: no symlink is ever shared.
fn seed_gig(vault: &Path) {
    touch(
        vault,
        "Gig/set.md",
        "---\ntitle: The gig\ntype: setlist\ndate: 2026-12-24\nsecret: the alarm code\n---\n\
         1. [[Gig/joy|Joy]]\n2. [[Private/plan|the plan]] and [[Private/plan]]\n\
         %%a private note%%\n<!-- an html note -->\n\n\
         ![[Gig/art/joy.png]]\n\n![[Private/cover.png]]\n\n![[Gig/joy.lrc]]\n\n\
         ![[Gig/joy.mp3]]\n\n![[Gig/big.png]]\n\n![[Gig/evil.png]]\n\n![[Gig/inner.png]]\n",
    );
    touch(
        vault,
        "Gig/joy.md",
        "---\ntitle: Joy\ntype: song\ncover: art/joy.png\n---\nJoy to the world.\n",
    );
    touch(vault, "Gig/art/joy.png", "png joy");
    touch(vault, "Gig/joy.lrc", "[00:01.00]Joy to the world");
    touch(vault, "Gig/joy.mp3", "mp3 joy");
    std::fs::write(
        vault.join("Gig/big.png"),
        vec![
            b'p';
            usize::try_from(architect_spaces::view::VIEW_FILE_MAX)
                .unwrap()
                .saturating_add(1)
        ],
    )
    .unwrap();
    let outside = vault.parent().unwrap().join("outside.png");
    std::fs::write(&outside, "outside secret").unwrap();
    std::os::unix::fs::symlink(&outside, vault.join("Gig/evil.png")).unwrap();
    std::os::unix::fs::symlink("../Private/cover.png", vault.join("Gig/inner.png")).unwrap();
}

/// A link to the gig, its pages built.
async fn gig_link(w: &World) -> Share {
    let share = w
        .share(
            ShareScope::Folder("Gig".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    page_with(&w.url(&format!("/s/{}/set/", share.code)), None, "Joy").await;
    share
}

/// sha256 of `bytes`, lower-case hex.
fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A guest's view on a shared setlist reads its text as the page shows it
/// and the link's images and lyrics, and nothing past the link: not a
/// note or file outside it, not what a path trick names, not the build's
/// data as a file.
#[tokio::test(flavor = "multi_thread")]
async fn a_guests_view_reads_the_shared_text_and_files_and_nothing_past_the_link() {
    let w = world_full(
        architect_sites::Blocks::new(),
        vec![setlist_island(true, &["documents", "files"])],
        seed_gig,
    )
    .await;
    let share = gig_link(&w).await;
    let code = share.code.clone();
    let client = guest(&w);
    let set = client
        .read(code.clone(), String::new(), "Gig/set.md".to_owned())
        .await
        .unwrap();

    // Its text, as the page shows it.
    let text = set.source.clone().expect("the view reads the note");
    assert!(text.contains("1. [[Gig/joy|Joy]]"), "{text}");
    assert!(text.contains("2. the plan and Private/plan"), "{text}");
    assert!(text.contains("![[Gig/art/joy.png]]"), "{text}");
    for never in [
        "a private note",
        "an html note",
        "[[Private/plan",
        "Private/cover.png",
        "the alarm code",
        "secret:",
        "---",
    ] {
        assert!(!text.contains(never), "{never:?} in {text}");
    }

    // Its files: the link's images and lyrics, its own first; never audio,
    // a file over the cap, one outside the link or a symlink, out of the
    // vault or inside it.
    let paths: Vec<&str> = set.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["Gig/art/joy.png", "Gig/joy.lrc"], "{:?}", set.files);
    for f in &set.files {
        assert!(f.url.starts_with(&format!("/s/{code}/")), "{}", f.url);
        let (status, _, bytes) = get_bytes(&w.url(&f.url)).await;
        assert_eq!(status, StatusCode::OK, "{}", f.url);
        assert_eq!(u64::try_from(bytes.len()).unwrap(), f.size, "{}", f.path);
        let sha = f.sha256.as_deref().expect("a guest's file is hashed");
        assert_eq!(sha, hex_sha256(&bytes), "{}", f.path);
    }

    // A note of the link whose type no island reads: its words only.
    let song = client
        .read(code.clone(), String::new(), "Gig/joy.md".to_owned())
        .await
        .unwrap();
    assert!(song.source.is_none() && song.files.is_empty(), "{song:?}");

    // A note outside the link, and every path trick on it: one answer,
    // the same as for a note that is nowhere.
    let abs = w.tmp.path().join("host/data/vaults/notes/Private/plan.md");
    for path in [
        "Private/plan.md",
        "Gig/../Private/plan.md",
        "/Gig/set.md",
        "Gig//set.md",
        "Gig%2Fset.md",
        "Gig/set.md%00",
        "Gig\\set.md",
        "./Gig/set.md",
        abs.to_str().unwrap(),
        "Gig/evil.png",
        "Gig/inner.png",
    ] {
        let read = client
            .read(code.clone(), String::new(), path.to_owned())
            .await;
        assert_eq!(guest_status(&read), 404, "{path}");
    }

    // The build's guest data is read only through the guest service,
    // which asks the islands first: it is written beside the served
    // folder, never in it, so no spelling of a URL reaches it, not even
    // on a filesystem that folds case (`ſ`) or trailing dots and spaces.
    let served = w.tmp.path().join(format!("host/data/shares/{code}/site"));
    let data = architect_sites::data_dir(&served);
    assert!(
        data.join("set")
            .join(architect_sites::plain::NOTE_FILE)
            .is_file(),
        "{}",
        data.display()
    );
    assert!(!data.starts_with(&served), "{}", data.display());
    let mut walk = vec![served.clone()];
    while let Some(dir) = walk.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            assert!(!name.contains("_note"), "{}", entry.path().display());
            if entry.file_type().unwrap().is_dir() {
                walk.push(entry.path());
            }
        }
    }
    for path in [
        format!("/s/{code}/set/_note.json"),
        format!("/s/{code}/set/%5Fnote.json"),
        format!("/s/{code}/set/_NOTE.JSON"),
        format!("/s/{code}/set/_note.j%C5%BFon"),
        format!("/s/{code}/set/_note.json."),
        format!("/s/{code}/set/_note.json%20"),
        format!("/s/{code}/set/_note.json::$DATA"),
        format!("/s/{code}/set/_note.json/"),
        format!("/s/{code}/_note.json"),
        format!("/s/{code}/../site.data/set/_note.json"),
        format!("/s/{code}/%2e%2e/site.data/set/_note.json"),
    ] {
        let (status, body) = raw_get(w.host.http, &path, "127.0.0.1").await;
        assert_eq!(status, 404, "{path}");
        assert!(
            !body.contains("a private") && !body.contains("[[Gig"),
            "{path}: {body}"
        );
    }

    // The byte route: nothing outside the link, however the path is
    // spelled, and never the target of a symlink out of the vault.
    for path in [
        format!("/s/{code}/../Private/cover.png"),
        format!("/s/{code}/%2e%2e/Private/cover.png"),
        format!("/s/{code}/Gig%2F..%2F..%2FPrivate%2Fcover.png"),
        format!("/s/{code}/..%2fPrivate%2fcover.png"),
        format!("/s/{code}/%2FPrivate/cover.png"),
        format!("/s/{code}//etc/passwd"),
        format!("/s/{code}/Private/cover.png"),
        format!("/s/{code}/Gig/evil.png"),
        format!("/s/{code}/Gig%5Cevil.png"),
    ] {
        let (status, body) = raw_get(w.host.http, &path, "127.0.0.1").await;
        assert_ne!(status, 200, "{path}");
        assert!(
            !body.contains("private png")
                && !body.contains("outside secret")
                && !body.contains("root:"),
            "{path}: {body}"
        );
    }
    let listed = client
        .files(code.clone(), String::new(), None)
        .await
        .unwrap();
    assert!(
        !listed.iter().any(|f| f.path.contains("evil")
            || f.path.contains("inner")
            || f.path.starts_with("Private/")),
        "{listed:?}"
    );
    w.host.stop().await;
}

/// The node hands a guest's view only what its extension declared: the
/// text only to a claim that reads the note of an extension that
/// declares `documents`, the files only with `files` too.
#[tokio::test(flavor = "multi_thread")]
async fn a_view_that_did_not_declare_the_read_gets_nothing() {
    // (reads the note in its claim, declares, text?, files?)
    for (reads_note, reads, text, files) in [
        (true, &["documents"][..], true, false),
        (true, &["files"][..], false, false),
        (true, &[][..], false, false),
        (false, &["documents", "files"][..], false, false),
        (true, &["documents", "files"][..], true, true),
    ] {
        let w = world_full(
            architect_sites::Blocks::new(),
            vec![setlist_island(reads_note, reads)],
            seed_gig,
        )
        .await;
        let share = gig_link(&w).await;
        let set = guest(&w)
            .read(share.code.clone(), String::new(), "Gig/set.md".to_owned())
            .await
            .unwrap();
        let row = format!("claim reads: {reads_note}, declares {reads:?}");
        assert_eq!(set.source.is_some(), text, "{row}");
        assert_eq!(!set.files.is_empty(), files, "{row}");
        // Its page's words, whatever the view may read.
        assert!(!set.blocks.is_empty(), "{row}");
        w.host.stop().await;
    }
}

// ── Symlinks ─────────────────────────────────────────────────────────

/// A link follows no symlink: not one to a file outside the vault, not
/// one inside it naming a private folder's file or `.git`, not a
/// symlinked folder, not a file swapped for a symlink after the build,
/// not a symlink planted in the link's built output; on its pages, its
/// `_files.json`, its zip and its byte route alike.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_link_serves_nothing_through_a_symlink() {
    use std::os::unix::fs::symlink;
    const SECRET: &str = "outside secret";
    const GIT: &str = "[remote] token";
    // The world's private note and picture, and `.git`'s config.
    let secrets = [SECRET, GIT, "The secret plan.", "private png"];
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    let outside = w.tmp.path().join("outside");
    touch(&outside, "secret.png", SECRET);
    touch(
        &outside,
        "secret.md",
        &format!("---\ntitle: Leaked\n---\n{SECRET}\n"),
    );
    touch(&vault, ".git/config", GIT);
    std::fs::write(vault.join("Private/.fts-private"), "").unwrap();
    touch(
        &vault,
        "Shared/leak.md",
        "---\ntitle: Leak\n---\n![[evil.png]]\n\n![[out/secret.png]]\n\n![[x.png]]\n\n![[priv/cover.png]]\n\n[[evil]] [[notes]]\n",
    );
    // Out of the vault.
    symlink(outside.join("secret.png"), vault.join("Shared/evil.png")).unwrap();
    symlink(outside.join("secret.md"), vault.join("Shared/evil.md")).unwrap();
    symlink(&outside, vault.join("Shared/out")).unwrap();
    // Inside it, to what it never shares.
    symlink("../Private/plan.md", vault.join("Shared/notes.md")).unwrap();
    symlink("../.git/config", vault.join("Shared/x.png")).unwrap();
    symlink("../Private", vault.join("Shared/priv")).unwrap();
    let share = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let link = format!("/s/{}/", share.code);
    page_with(&w.url(&format!("{link}leak/")), None, "Leak").await;

    let leaks = |what: &str, status: StatusCode, body: &[u8]| {
        let text = String::from_utf8_lossy(body);
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{what}: {status} hands out `{secret}` through a symlink"
            );
        }
    };
    for path in [
        "leak/",
        "Shared/evil.png",
        "Shared/out/secret.png",
        "Shared/x.png",
        "Shared/priv/cover.png",
        "Shared/notes.md",
        "Shared/evil.md",
        "evil/",
        "notes/",
        "out/secret.png",
    ] {
        let (status, _, body) = get_bytes(&w.url(&format!("{link}{path}"))).await;
        leaks(path, status, &body);
    }
    let files = json_at(&w.url(&format!("{link}_files.json"))).await;
    leaks("_files.json", StatusCode::OK, files.to_string().as_bytes());
    let listed = listed_paths(&files);
    assert!(listed.iter().any(|f| f == "Shared/art.png"), "{listed:?}");
    for link in ["evil.png", "x.png", "out", "priv", "notes.md", "evil.md"] {
        assert!(
            !listed
                .iter()
                .any(|f| f.starts_with(&format!("Shared/{link}"))),
            "{link} listed: {listed:?}"
        );
    }
    let zip = zip_at(&w.url(&format!("{link}_archive.zip"))).await;
    assert!(
        zip.iter().any(|(name, _)| name.ends_with("art.png")),
        "{zip:?}"
    );
    for (name, text) in &zip {
        leaks(name, StatusCode::OK, text.as_bytes());
    }

    // Swapped after the build: the zip reads the vault as it is now and
    // follows no symlink.
    std::fs::remove_file(vault.join("Shared/art.png")).unwrap();
    symlink(outside.join("secret.png"), vault.join("Shared/art.png")).unwrap();
    let (status, _, body) = get_bytes(&w.url(&format!("{link}_archive.zip"))).await;
    leaks("the zip after a swap", status, &body);
    let (status, _, body) = get_bytes(&w.url(&format!("{link}Shared/art.png"))).await;
    leaks("Shared/art.png after a swap", status, &body);

    // Planted in the link's built output: never followed out of it.
    let built = w
        .tmp
        .path()
        .join(format!("host/data/shares/{}/site", share.code));
    assert!(built.is_dir(), "{}", built.display());
    symlink(outside.join("secret.png"), built.join("planted.png")).unwrap();
    symlink(&outside, built.join("planted")).unwrap();
    for path in ["planted.png", "planted/secret.png", "planted/"] {
        let (status, _, body) = get_bytes(&w.url(&format!("{link}{path}"))).await;
        leaks(path, status, &body);
    }

    // A guest's read of a note: the page's `_note.json` swapped for a
    // symlink (here to a forged one outside) is refused, never followed.
    let client = guest(&w);
    let read = || {
        client.read(
            share.code.clone(),
            String::new(),
            "Shared/leak.md".to_owned(),
        )
    };
    assert_eq!(read().await.unwrap().title, "Leak");
    let note = architect_sites::data_dir(&built)
        .join("leak")
        .join(architect_sites::plain::NOTE_FILE);
    let forged =
        std::fs::read_to_string(&note)
            .unwrap()
            .replacen("\"Leak\"", &format!("\"{SECRET}\""), 1);
    assert!(forged.contains(SECRET), "{forged}");
    std::fs::write(outside.join("note.json"), forged).unwrap();
    std::fs::remove_file(&note).unwrap();
    symlink(outside.join("note.json"), &note).unwrap();
    match read().await {
        Err(architect::ClientError::App(architect_spaces::GuestFault::Building)) => {}
        other => panic!("a guest read through a symlink: {other:?}"),
    }
    w.host.stop().await;
}

/// A link's own files never run on the node's origin (the web app's,
/// #646): a page, a picture that is a document (SVG) and a script a note
/// links are downloads under a sandbox that allows nothing, never this
/// origin's page; its pictures and recordings stay inline.
#[tokio::test(flavor = "multi_thread")]
async fn a_links_own_page_svg_or_script_never_runs_on_the_origin() {
    let w = world_full(architect_sites::Blocks::new(), Vec::new(), |vault| {
        touch(
            vault,
            "Shared/run.md",
            "---\ntitle: Run\n---\n![[x.html]]\n\n![[x.svg]]\n\n![[x.js]]\n\n![[x.xhtml]]\n\n![[x.xml]]\n\n![[take.mp3]]\n",
        );
        let script = "<script>fetch('/s/_signout')</script>";
        touch(vault, "Shared/x.html", &format!("<!doctype html>{script}"));
        touch(
            vault,
            "Shared/x.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(localStorage.length)</script></svg>",
        );
        touch(vault, "Shared/x.js", "alert(localStorage.length)");
        touch(
            vault,
            "Shared/x.xhtml",
            &format!("<html xmlns=\"http://www.w3.org/1999/xhtml\">{script}</html>"),
        );
        touch(vault, "Shared/x.xml", "<a>b</a>");
        touch(vault, "Shared/take.mp3", "ID3 not really");
    })
    .await;
    let share = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let link = format!("/s/{}/", share.code);
    page_with(&w.url(&format!("{link}run/")), None, "Run").await;
    for file in ["x.html", "x.svg", "x.js", "x.xhtml", "x.xml"] {
        let (status, headers, body) = get(&w.url(&format!("{link}Shared/{file}")), None).await;
        assert_eq!(status, StatusCode::OK, "{file}: {body}");
        // The bytes as they are, but as a download in a sandbox that runs
        // nothing: no script, no same origin, no storage, no cookie.
        assert_eq!(
            headers["content-security-policy"], "sandbox; default-src 'none'",
            "{file}"
        );
        assert_eq!(headers["content-disposition"], "attachment", "{file}");
        assert_eq!(headers["x-content-type-options"], "nosniff", "{file}");
        assert_eq!(
            headers.get_all("content-security-policy").iter().count(),
            1,
            "{file}: one policy, the sandbox"
        );
    }
    // Pictures and recordings: inline, under the node's own policy (no
    // script of theirs runs: they are not documents).
    for (file, kind) in [
        ("Shared/art.png", "image/png"),
        ("Shared/take.mp3", "audio/mpeg"),
    ] {
        let (status, headers, _) = get(&w.url(&format!("{link}{file}")), None).await;
        assert_eq!(status, StatusCode::OK, "{file}");
        assert_eq!(headers["content-type"], kind, "{file}");
        assert!(headers.get("content-disposition").is_none(), "{file}");
        assert!(
            !headers["content-security-policy"]
                .to_str()
                .unwrap()
                .starts_with("sandbox"),
            "{file}"
        );
        assert_eq!(headers["x-content-type-options"], "nosniff", "{file}");
    }
    w.host.stop().await;
}

/// Everything of an answer but its date.
async fn answer_bytes(url: &str) -> (u16, Vec<(String, String)>, String) {
    let res = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    let mut headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .filter(|(k, _)| k.as_str() != "date")
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
        .collect();
    headers.sort();
    (status, headers, res.text().await.unwrap())
}

/// A link that never was, an expired one and a revoked one answer byte
/// for byte alike, at the link, a page below it, a file and without the
/// link's slash: a code's answer never says whether it was ever a link.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_code_answers_exactly_as_a_revoked_or_expired_one() {
    let w = world().await;
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let revoked = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let expired = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    page_with(&w.url(&format!("/s/{}/", revoked.code)), None, "The album").await;
    links.revoke(revoked.code.clone()).await.unwrap();
    links
        .update(expired.code.clone(), ShareChange::Expires(Some(1)))
        .await
        .unwrap();
    let unknown = "abcdefghjkmnpqrs";
    assert_eq!(unknown.len(), revoked.code.len());
    for rest in ["/", "/track/", "/Shared/art.png", "", "/_files/"] {
        let never = answer_bytes(&w.url(&format!("/s/{unknown}{rest}"))).await;
        let never = (never.0, never.1, never.2.replace(unknown, "CODE"));
        for code in [&revoked.code, &expired.code] {
            let (status, headers, body) = answer_bytes(&w.url(&format!("/s/{code}{rest}"))).await;
            let body = body.replace(code.as_str(), "CODE");
            let headers: Vec<_> = headers
                .into_iter()
                .map(|(k, v)| (k, v.replace(code.as_str(), unknown)))
                .collect();
            assert_eq!(
                (status, &headers, &body),
                (never.0, &never.1, &never.2),
                "{rest}"
            );
        }
        if rest.is_empty() {
            assert_eq!(never.0, 308);
        } else {
            assert_eq!(never.0, 404, "{rest}");
            assert!(
                never
                    .1
                    .contains(&("cache-control".to_owned(), "no-store".to_owned()))
            );
        }
    }
    w.host.stop().await;
}

/// Sign-in and sign-out go back only to a path on this node: every way a
/// browser would read `to` as another origin is `/`.
#[tokio::test(flavor = "multi_thread")]
async fn sign_out_goes_back_only_to_a_path_on_the_node() {
    let w = world().await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let origin = format!("http://{}", w.host.http);
    for (to, want) in [
        ("/%09/evil.example", "/"),
        ("/%0a/evil.example", "/"),
        ("/%0d/evil.example", "/"),
        ("%5C/evil.example", "/"),
        ("//evil.example", "/"),
        ("/%2F/evil.example", "/"),
        ("/%5Cevil.example", "/"),
        ("https:evil.example", "/"),
        ("https://evil.example/", "/"),
        ("/s/abcdefghjkmnpqrs/joy/", "/s/abcdefghjkmnpqrs/joy/"),
    ] {
        let res = client
            .post(w.url(&format!("/s/_signout?to={to}")))
            .header("origin", &origin)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER, "{to}");
        assert_eq!(res.headers()["location"], want, "{to}");
    }
    w.host.stop().await;
}

/// The guest service answers any origin without credentials: a call that
/// carried the node's cookie (its own app's) and its preflight say
/// `Access-Control-Allow-Origin: *`, never the caller's origin and never
/// `Access-Control-Allow-Credentials`, so no other site reads an answer a
/// browser sent the cookie with.
#[tokio::test(flavor = "multi_thread")]
async fn the_guest_service_never_shares_a_cookies_answer_with_another_origin() {
    let w = world().await;
    let share = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Org,
            ShareAccess::Read,
        )
        .await;
    let call = w.url(&format!("/s/{}/_guest/link", share.code));
    let evil = "https://evil.example";
    let body = format!("{{\"code\":\"{}\",\"token\":\"\"}}", share.code);
    let res = reqwest::Client::new()
        .post(&call)
        .header("origin", evil)
        .header("content-type", "application/json")
        .header("cookie", format!("architect_session={}", w.alan))
        .body(body.clone())
        .send()
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let text = res.text().await.unwrap();
    // The cookie opened it (a member of the org).
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(
        !text.contains("SignIn") && !text.contains("Denied"),
        "{text}"
    );
    let preflight = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &call)
        .header("origin", evil)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
    for (what, headers) in [("answer", &headers), ("preflight", preflight.headers())] {
        assert_eq!(headers["access-control-allow-origin"], "*", "{what}");
        assert_eq!(
            headers
                .get_all("access-control-allow-origin")
                .iter()
                .count(),
            1,
            "{what}"
        );
        assert!(
            headers.get("access-control-allow-credentials").is_none(),
            "{what}"
        );
        assert!(
            !headers
                .iter()
                .any(|(_, v)| v.to_str().is_ok_and(|v| v.contains("evil.example"))),
            "{what}: the origin is never reflected"
        );
    }
    w.host.stop().await;
}

/// A setlist's listing fence (#668: a collection's rows as Studio writes
/// them) is cut by the node to the link's notes, in its words and in the
/// text a view that reads the note gets: a row for a song outside the
/// link is gone, its title, path, recording and artist with it.
#[tokio::test(flavor = "multi_thread")]
async fn a_guest_never_reads_a_listing_fences_row_for_a_note_outside_the_link() {
    let w = world_full(
        architect_sites::Blocks::new(),
        vec![setlist_island(true, &["documents"])],
        |vault| {
            seed_gig(vault);
            touch(
                vault,
                "Gig/list.md",
                "---\ntitle: The list\ntype: setlist\nsongs: [\"[[Gig/joy]]\", \"[[Private/Secret Song]]\"]\n---\n\
                 The list.\n\n```studio-setlist\n## Opening\n\
                 1 | Joy | Gig/joy.md | B · 86 bpm | Gig/joy.mp3 | Choir | 186.0\n\
                 2 | Secret Song | Private/Secret Song.md | E · 99 bpm | Private/secret-take.mp3 | Unreleased Artist\n```\n",
            );
            touch(vault, "Private/Secret Song.md", "---\ntitle: Secret Song\n---\nNot shared.\n");
        },
    )
    .await;
    let share = gig_link(&w).await;
    let list = guest(&w)
        .read(share.code.clone(), String::new(), "Gig/list.md".to_owned())
        .await
        .unwrap();
    let text = list.source.clone().expect("the view reads the note");
    assert!(text.contains("1 | Joy | Gig/joy.md |"), "{text}");
    assert!(
        list.blocks
            .iter()
            .any(|b| b.text.contains("1 | Joy | Gig/joy.md |")),
        "{:?}",
        list.blocks
    );
    let said = format!("{text}\n{:?}", list.blocks);
    for never in ["Secret", "Private/", "secret-take", "Unreleased"] {
        assert!(!said.contains(never), "{never:?} in {said}");
    }
    // On a domain of its own the link serves its built pages: cut at the
    // build, so they never carry the row either.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    links
        .update(
            share.code.clone(),
            ShareChange::Domain(Some("gig.example.test".to_owned())),
        )
        .await
        .unwrap();
    let mut body = String::new();
    for _ in 0..200 {
        let res = reqwest::Client::new()
            .get(w.url("/list/"))
            .header("host", "gig.example.test")
            .send()
            .await
            .unwrap();
        if res.status() == StatusCode::OK {
            body = res.text().await.unwrap();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(body.contains("1 | Joy | Gig/joy.md |"), "{body}");
    for never in ["Secret", "Private/", "secret-take", "Unreleased"] {
        assert!(!body.contains(never), "{never:?} on the domain: {body}");
    }
    w.host.stop().await;
}

/// A recording and a cover a song names only by `recording:` and
/// `cover:` (#668), linked in no note's body, are the reference layer's
/// when no rule places them: a link that serves only the page never
/// serves them, as it never did before; a reference link does.
#[tokio::test(flavor = "multi_thread")]
async fn a_recording_named_only_by_a_property_is_the_references_never_the_pages() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    touch(
        &vault,
        "Songs/Joy.md",
        "---\ntitle: Joy\ntype: song\nrecording: take.mp3\ncover: cover.png\n---\nJoy to the world.\n",
    );
    touch(&vault, "Songs/take.mp3", "MP3 JOY");
    touch(&vault, "Songs/cover.png", "PNG JOY");
    let links: ShareLinksClient = w.lane(&w.grace).await;
    for listen in [Listen::Page, Listen::Reference] {
        let share = links
            .create(NewShare {
                vault: "notes".to_owned(),
                scope: ShareScope::Folder("Songs".to_owned()),
                title: None,
                audience: Audience::Link,
                access: ShareAccess::Read,
                expires_at: None,
                domain: None,
                listen,
            })
            .await
            .unwrap();
        let url = w.url(&format!("/s/{}/", share.code));
        page_with(&url, None, "<!DOCTYPE html>").await;
        for (file, bytes) in [
            ("Songs/take.mp3", "MP3 JOY"),
            ("Songs/cover.png", "PNG JOY"),
        ] {
            if listen == Listen::Page {
                let (status, headers, body) = get(&format!("{url}{file}"), None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{file}: {body}");
                assert_eq!(headers["x-architect-layer"], "reference", "{file}");
            } else {
                page_with(&format!("{url}{file}"), None, bytes).await;
            }
            let listed = listed_paths(&json_at(&format!("{url}_files.json")).await);
            assert_eq!(
                listed.iter().any(|p| p == file),
                listen == Listen::Reference,
                "{listen:?} {file}: {listed:?}"
            );
        }
    }
    w.host.stop().await;
}

/// A page-only link to `scope` of the test vault, and its URL.
async fn page_link(w: &World, scope: &str) -> String {
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let share = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder(scope.to_owned()),
            title: None,
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: None,
            domain: None,
            listen: Listen::Page,
        })
        .await
        .unwrap();
    let url = w.url(&format!("/s/{}/", share.code));
    page_with(&url, None, "<!DOCTYPE html>").await;
    url
}

/// A song's recording, named only by `recording:` (#668), stays the
/// reference's when a note outside the share embeds it in its body: a
/// note outside never lowers a file, so a page link never serves it.
#[tokio::test(flavor = "multi_thread")]
async fn a_note_outside_the_share_never_lowers_a_songs_recording() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    touch(
        &vault,
        "Songs/Joy.md",
        "---\ntitle: Joy\ntype: song\nrecording: \"[[Rec/take.mp3]]\"\n---\nJoy to the world.\n",
    );
    touch(&vault, "Songs/Rec/take.mp3", "MP3 JOY");
    touch(
        &vault,
        "Private/notes.md",
        "The take: ![[Songs/Rec/take.mp3]]\n",
    );
    let url = page_link(&w, "Songs").await;
    let (status, _, body) = get(&format!("{url}Songs/Rec/take.mp3"), None).await;
    assert_ne!(status, StatusCode::OK, "{body}");
    assert_ne!(body, "MP3 JOY");
    let listed = listed_paths(&json_at(&format!("{url}_files.json")).await);
    assert!(
        !listed.iter().any(|p| p == "Songs/Rec/take.mp3"),
        "{listed:?}"
    );
    w.host.stop().await;
}

/// With no layer rules, one note the vault cannot read (a stray Latin-1
/// byte) holds back nothing of the others: a page link still serves the
/// files their bodies link (#668's placement without rules).
#[tokio::test(flavor = "multi_thread")]
async fn with_no_rules_an_unreadable_note_holds_back_no_other_notes_files() {
    let w = world().await;
    let vault = w.tmp.path().join("host/data/vaults/notes");
    touch(
        &vault,
        "Songs/Eve.md",
        "---\ntitle: Eve\n---\n![[eve.png]]\n",
    );
    touch(&vault, "Songs/eve.png", "PNG EVE");
    std::fs::write(vault.join("Songs/Broken.md"), b"# Broken\n\nCaf\xe9\n").unwrap();
    let url = page_link(&w, "Songs").await;
    page_with(&format!("{url}Songs/eve.png"), None, "PNG EVE").await;
    w.host.stop().await;
}
