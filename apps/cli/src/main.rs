//! `architect`: the command.
//!
//! - `architect site build --vault docs/content --out target/site`: a
//!   vault published as a static site (`architect-sites`), each fence an
//!   `--extension` claims drawn by its embed page.
//! - `architect site serve --vault docs/content`: the same, served on
//!   localhost through the architect server's own site router
//!   (`architect::host::site_router`), rebuilt on every save and reloaded
//!   in the browser.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use architect_sites::{Blocks, Options, PageBlocks, Report, Scope};
use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use clap::{Args, Parser, Subcommand};
use futures::Stream;
use notify::{RecursiveMode, Watcher as _};
use tokio::sync::broadcast;

/// The event stream a served site listens on for a rebuild.
const RELOAD: &str = "/_architect/reload";

#[derive(Parser)]
#[command(name = "architect", version, about = "architect's command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Publish a vault as a static site.
    #[command(subcommand)]
    Site(SiteCommand),
}

#[derive(Subcommand)]
enum SiteCommand {
    /// Build the site once.
    Build(SiteArgs),
    /// Build it, serve it on localhost, and rebuild and reload on save.
    Serve(ServeArgs),
}

#[derive(Args)]
struct SiteArgs {
    /// The vault: a folder of markdown notes.
    #[arg(long)]
    vault: PathBuf,
    /// Where the site is written.
    #[arg(long, default_value = "target/site")]
    out: PathBuf,
    /// The site's name. Default: the vault's home page title, else its
    /// folder's name.
    #[arg(long)]
    title: Option<String>,
    /// One line about the site.
    #[arg(long)]
    description: Option<String>,
    /// Where the site is served from (`/`, `/docs/`).
    #[arg(long, default_value = "/")]
    base: String,
    /// The theme family: `<theme>-light` and `<theme>-dark`.
    #[arg(long, default_value = "fasttrackstudio")]
    theme: String,
    /// Where it is published (`https://…`), for its sitemap.
    #[arg(long)]
    url: Option<String>,
    /// An extension's package (its `extension.toml`): the fences its embed
    /// pages claim are drawn by them. Repeatable.
    #[arg(long = "extension")]
    extensions: Vec<PathBuf>,
    /// A folder copied into the site as it is. Repeatable.
    #[arg(long = "static")]
    static_dirs: Vec<PathBuf>,
    /// Fail on a broken link or a block that could not be drawn.
    #[arg(long)]
    strict: bool,
    /// Publish one folder of the vault, rooted there (a share's scope).
    #[arg(long, conflicts_with_all = ["tag", "notes", "base_scope"])]
    folder: Option<String>,
    /// Publish the notes with this tag.
    #[arg(long, conflicts_with_all = ["notes", "base_scope"])]
    tag: Option<String>,
    /// Publish these notes (vault paths). Repeatable.
    #[arg(long = "note", conflicts_with = "base_scope")]
    notes: Vec<String>,
    /// Publish the notes a `.base` file's views select.
    #[arg(long = "base-scope")]
    base_scope: Option<String>,
}

#[derive(Args)]
struct ServeArgs {
    #[command(flatten)]
    site: SiteArgs,
    /// The port to serve on.
    #[arg(long, default_value_t = 4321)]
    port: u16,
    /// The address to serve on.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Site(SiteCommand::Build(args)) => build_once(&args),
        Command::Site(SiteCommand::Serve(args)) => serve(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("architect: {why}");
            ExitCode::FAILURE
        }
    }
}

impl SiteArgs {
    fn options(&self) -> Options {
        // Empty: the site takes its home page's title.
        let mut opts = Options::new(&self.vault, self.title.clone().unwrap_or_default());
        opts.description.clone_from(&self.description);
        opts.base.clone_from(&self.base);
        opts.theme.clone_from(&self.theme);
        opts.url.clone_from(&self.url);
        opts.strict = self.strict;
        opts.static_dirs.clone_from(&self.static_dirs);
        opts.scope = self.scope();
        opts
    }

    /// The part of the vault published: a link to a note outside it is
    /// plain text.
    fn scope(&self) -> Scope {
        self.folder
            .clone()
            .map(Scope::Folder)
            .or_else(|| self.tag.clone().map(Scope::Tag))
            .or_else(|| (!self.notes.is_empty()).then(|| Scope::Notes(self.notes.clone())))
            .or_else(|| self.base_scope.clone().map(Scope::Base))
            .unwrap_or(Scope::Vault)
    }

    fn blocks(&self) -> Result<Blocks, String> {
        let mut blocks = Blocks::new();
        for dir in &self.extensions {
            let renderer = PageBlocks::load(dir)?;
            println!(
                "  extension {} draws ```{}",
                renderer.id(),
                renderer.fences().join(", ```")
            );
            blocks.add(Arc::new(renderer));
        }
        Ok(blocks)
    }
}

fn build_once(args: &SiteArgs) -> Result<(), String> {
    let blocks = args.blocks()?;
    let report = build(&args.options(), &blocks, &args.out)?;
    print_report(&report);
    Ok(())
}

fn build(opts: &Options, blocks: &Blocks, out: &Path) -> Result<Report, String> {
    architect_sites::build(opts, blocks, out).map_err(|e| e.to_string())
}

fn print_report(report: &Report) {
    for problem in &report.problems {
        eprintln!("  warning: {problem}");
    }
    println!(
        "built {} pages, {} tags, {} files into {} ({} problem{})",
        report.pages,
        report.tags,
        report.files,
        report.out.display(),
        report.problems.len(),
        if report.problems.len() == 1 { "" } else { "s" }
    );
}

fn serve(args: &ServeArgs) -> Result<(), String> {
    let mut opts = args.site.options();
    opts.live_reload = Some(RELOAD.to_owned());
    let blocks = args.site.blocks()?;
    let out = args.site.out.clone();
    print_report(&build(&opts, &blocks, &out)?);

    let (reload, _) = broadcast::channel::<()>(16);
    let mut watched = vec![args.site.vault.clone()];
    watched.extend(args.site.extensions.iter().cloned());
    watched.extend(args.site.static_dirs.iter().cloned());
    let rebuilt = reload.clone();
    let rebuild_out = out.clone();
    std::thread::Builder::new()
        .name("site-watch".to_owned())
        .spawn(move || watch(&watched, &opts, &blocks, &rebuild_out, &rebuilt))
        .map_err(|e| format!("could not start the watcher: {e}"))?;

    let app = Router::new()
        .route(RELOAD, get(reload_stream))
        .with_state(reload)
        .merge(architect::host::site_router(&out));
    let addr = format!("{}:{}", args.host, args.port);
    architect::host::block_on(async move {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("could not listen on {addr}: {e}"))?;
        println!("serving {} at http://{addr}/ (Ctrl-C stops)", out.display());
        axum::serve(listener, app)
            .await
            .map_err(|e| format!("the server stopped: {e}"))
    })
}

/// Rebuild on every change under `paths`, a burst of saves at once, and
/// tell the browsers.
fn watch(
    paths: &[PathBuf],
    opts: &Options,
    blocks: &Blocks,
    out: &Path,
    reload: &broadcast::Sender<()>,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !e.kind.is_access()) {
            let _ = tx.send(());
        }
    })
    .map_err(|e| e.to_string())?;
    for path in paths {
        watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| format!("could not watch {}: {e}", path.display()))?;
    }
    while rx.recv().is_ok() {
        // A save is often several events (a temp file, a rename): wait
        // for them to settle, then build once.
        while rx.recv_timeout(Duration::from_millis(120)).is_ok() {}
        match build(opts, blocks, out) {
            Ok(report) => {
                print_report(&report);
                let _ = reload.send(());
            }
            Err(why) => eprintln!("architect: rebuild failed: {why}"),
        }
    }
    Ok(())
}

/// The live-reload stream: one event per rebuild.
async fn reload_stream(
    State(reload): State<broadcast::Sender<()>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = futures::stream::unfold(reload.subscribe(), |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(()) => return Some((Ok(Event::default().data("reload")), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
