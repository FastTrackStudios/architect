//! The `actions` command (feature `cli`): an app's Action List from the
//! command line, the same as over MCP.
//!
//! An app embeds it as a subcommand of its own — [`ActionsArgs`] is a
//! clap `Args` to flatten in, or [`Cli::main`] parses `actions …` itself:
//!
//! ```ignore
//! // `task actions list`, `task actions run TASKS_CREATE --arg title=Hi`
//! let cli = architect_control::cli::Cli::new("task")
//!     .headless(|| Ok(Box::new(ListOnly::new(my_surface()))));
//! if std::env::args().nth(1).as_deref() == Some("actions") {
//!     return cli.main(std::env::args_os().skip(1));
//! }
//! ```
//!
//! Every command is one request ([`Request`]) to the running app's socket
//! (`--app` picks another app, or a socket by its path), or, `--headless`,
//! to the app's Action List started here with no window. Either way the
//! answer is the one MCP gives for the same request.
//!
//! **Exit codes** come from the failure's [`Code`] ([`Code::exit`]): `0`
//! done, `1` the action failed, `2` usage, `3` no such action, `4` bad
//! arguments, `5` not allowed, `6` disabled, `7` a person must approve it
//! first (the ticket is printed), `69` no app answered.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{Read as _, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use architect::action::ActionCaller;
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

use crate::client::Client;
use crate::protocol::{Code, Failure, Filter, MAX_WAIT, Reply, Request, Step, Watch};
use crate::{Surface, endpoint};

/// `actions …`: what to ask, and of which app.
#[derive(Debug, Clone, clap::Args)]
pub struct ActionsArgs {
    /// The running app to ask: its name (`task`) or its socket's path.
    /// This app unless given.
    #[arg(long, global = true, value_name = "NAME|SOCKET")]
    pub app: Option<String>,
    /// Start the app's Action List here, with no window, and ask it —
    /// for scripts and CI. Each command is a fresh start.
    #[arg(long, global = true)]
    pub headless: bool,
    #[command(subcommand)]
    pub command: ActionsCommand,
}

/// The `actions` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ActionsCommand {
    /// The actions you may run: id, effect, name and what each does.
    List {
        /// Text to look for in the id, name, description, category or group.
        #[arg(long)]
        filter: Option<String>,
        /// Only this category.
        #[arg(long)]
        category: Option<String>,
        /// Only this effect.
        #[arg(long, value_enum)]
        effect: Option<EffectArg>,
        /// Print the answer as JSON.
        #[arg(long)]
        json: bool,
    },
    /// One action in full: what it takes and gives (JSON Schema) and its
    /// effect.
    Describe {
        /// The action's id.
        id: String,
        /// Print the answer as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run an action. `--arg key=text` for a string, `--arg key:=json` for
    /// anything else, or the whole arguments as `--json '{…}'` (`-` reads
    /// them from stdin).
    Run {
        /// The action's id.
        id: String,
        /// One argument: `key=text`, or `key:=json`.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
        /// The arguments as JSON (`-`: from stdin).
        #[arg(long, value_name = "JSON", conflicts_with = "args")]
        json: Option<String>,
        /// How to print the result: `text` prints a string as it is and
        /// anything else as indented JSON; `json` one line of JSON.
        #[arg(long, value_enum, default_value_t = Output::Text)]
        output: Output,
        /// The ticket a `confirmation_required` answer gave, once a person
        /// approved the call in the app: good for this exact call, once.
        #[arg(long)]
        ticket: Option<String>,
    },
    /// Run several actions in order, stopping at the first failure. Not
    /// atomic: what ran stays done.
    Batch {
        /// The steps, `[{"id": …, "args": …}, …]` (`-`: from stdin).
        steps: String,
        /// Go on past a failed step.
        #[arg(long)]
        keep_going: bool,
        /// Print the answer as JSON.
        #[arg(long)]
        json: bool,
    },
    /// The whole set as one JSON Schema document; `--api`, the protocol's
    /// own requests.
    Schema {
        /// The protocol (every method and what it takes), not the actions.
        #[arg(long)]
        api: bool,
    },
    /// What happened in the app: actions run and state that changed, each
    /// with its sequence number. Events are invalidation: when one says
    /// `lost`, read the state again.
    Events {
        /// After this sequence number (every event the app keeps unless
        /// given; with `--follow`, from now).
        #[arg(long)]
        since: Option<u64>,
        /// Keep watching, printing each event as it happens.
        #[arg(long)]
        follow: bool,
        /// At most this many at a time.
        #[arg(long)]
        max: Option<u64>,
        /// One JSON object a line.
        #[arg(long)]
        json: bool,
    },
    /// The running app's window (or one pane) as a PNG, drawn by the app
    /// itself (`APP_SHOT`): prints the file's absolute path.
    Shot {
        /// Where to write it (`app-shot.png` here unless given); must end
        /// in `.png`.
        path: Option<PathBuf>,
        /// Only this pane (a workspace pane id).
        #[arg(long)]
        pane: Option<String>,
        /// Print the whole answer (path, size) as JSON.
        #[arg(long)]
        json: bool,
    },
    /// The apps answering on a control socket here.
    Apps {
        /// Print the answer as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// An effect, as `--effect` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EffectArg {
    ReadOnly,
    Mutating,
    Destructive,
}

impl EffectArg {
    const fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Mutating => "mutating",
            Self::Destructive => "destructive",
        }
    }
}

/// How `run` prints a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Output {
    Text,
    Json,
}

#[derive(Parser)]
#[command(
    name = "actions",
    about = "An app's Action List: list, describe and run its actions, the same as over MCP."
)]
struct Top {
    #[command(flatten)]
    args: ActionsArgs,
}

/// An app's Action List started with no window, for `--headless`.
pub trait Headless {
    /// The surface to ask: the app's list, name and caller.
    fn surface(&self) -> Surface;

    /// Let the app do what it has waiting (a document's tasks, a window
    /// thread's queue); `true` when it did anything. Asked over and over
    /// while a request is answered, so an action that hands its work to
    /// the app's own thread is answered.
    fn pump(&mut self) -> bool {
        false
    }

    /// Let it start up: pump until it has been idle a while (3 s at
    /// most).
    fn settle(&mut self) {
        let start = Instant::now();
        let mut idle = 0_u32;
        while idle < 24 && start.elapsed() < Duration::from_secs(3) {
            if self.pump() {
                idle = 0;
            } else {
                idle = idle.saturating_add(1);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// A headless app that is only its list: nothing to pump.
#[derive(Debug, Clone)]
pub struct ListOnly(Surface);

impl ListOnly {
    #[must_use]
    pub const fn new(surface: Surface) -> Self {
        Self(surface)
    }
}

impl Headless for ListOnly {
    fn surface(&self) -> Surface {
        self.0.clone()
    }

    fn settle(&mut self) {}
}

type Open = Box<dyn Fn() -> Result<Box<dyn Headless>, String>>;

/// The `actions` command for one app.
pub struct Cli {
    app: Option<String>,
    headless: Option<Open>,
}

impl std::fmt::Debug for Cli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cli")
            .field("app", &self.app)
            .field("headless", &self.headless.is_some())
            .finish()
    }
}

impl Cli {
    /// The command for app `app`: its socket unless `--app` names
    /// another.
    #[must_use]
    pub fn new(app: impl Into<String>) -> Self {
        Self {
            app: Some(app.into()),
            headless: None,
        }
    }

    /// The command for no app in particular (`architect-actions`):
    /// `--app` names one, or the one app serving a socket here is asked.
    #[must_use]
    pub const fn generic() -> Self {
        Self {
            app: None,
            headless: None,
        }
    }

    /// How `--headless` starts the app's list with no window.
    #[must_use]
    pub fn headless(
        mut self,
        open: impl Fn() -> Result<Box<dyn Headless>, String> + 'static,
    ) -> Self {
        self.headless = Some(Box::new(open));
        self
    }

    /// Parse `args` (the first is the command's own name: `actions`) and
    /// run it, printing to stdout and stderr.
    pub fn main<I, T>(&self, args: I) -> ExitCode
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let top = match Top::try_parse_from(args) {
            Ok(top) => top,
            Err(e) => {
                let _ = e.print();
                return ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(2));
            }
        };
        self.run(&top.args)
    }

    /// Run `args`, printing the answer to stdout and what went wrong to
    /// stderr: the exit code the failure maps to ([`Code::exit`]).
    #[must_use]
    pub fn run(&self, args: &ActionsArgs) -> ExitCode {
        let mut out = std::io::stdout().lock();
        let mut err = std::io::stderr().lock();
        let code = self.execute(args, &mut out, &mut err);
        let _ = out.flush();
        ExitCode::from(code)
    }

    /// Run `args`, writing the answer to `out` and failures to `err`; the
    /// exit code.
    pub fn execute(&self, args: &ActionsArgs, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
        match self.answer(args, out) {
            Ok(()) => 0,
            Err(failure) => {
                let _ = tell(err, &failure);
                failure.code.exit()
            }
        }
    }

    fn answer(&self, args: &ActionsArgs, out: &mut dyn Write) -> Result<(), Failure> {
        if let ActionsCommand::Apps { json } = &args.command {
            return apps(out, *json);
        }
        let mut target = self.target(args)?;
        let written = |e: std::io::Error| Failure::new(Code::Failed, format!("writing: {e}"));
        match &args.command {
            ActionsCommand::List {
                filter,
                category,
                effect,
                json,
            } => {
                let request = Request::Actions(Filter {
                    text: filter.clone(),
                    category: category.clone(),
                    effect: effect.map(|e| e.name().to_owned()),
                });
                let value = settled(&target.call(&request)?, out, *json)?;
                if !*json {
                    print_list(out, &value).map_err(written)?;
                }
            }
            ActionsCommand::Describe { id, json } => {
                let value = settled(
                    &target.call(&Request::Describe { id: id.clone() })?,
                    out,
                    *json,
                )?;
                if !*json {
                    print_detail(out, &value).map_err(written)?;
                }
            }
            ActionsCommand::Run {
                id,
                args,
                json,
                output,
                ticket,
            } => {
                let request = Request::Run {
                    id: id.clone(),
                    args: arguments(args, json.as_deref())?,
                    ticket: ticket.clone(),
                };
                let as_json = *output == Output::Json;
                let value = settled(&target.call(&request)?, out, as_json)?;
                if !as_json {
                    print_result(out, &value).map_err(written)?;
                }
            }
            ActionsCommand::Batch {
                steps,
                keep_going,
                json,
            } => {
                let request = Request::Batch {
                    steps: batch_steps(steps)?,
                    stop_on_error: !keep_going,
                };
                let reply = target.call(&request)?;
                if *json {
                    writeln!(out, "{}", reply.value()).map_err(written)?;
                } else {
                    print_batch(out, reply.value()).map_err(written)?;
                }
                if let Some(failure) = reply.failure() {
                    return Err(failure);
                }
            }
            ActionsCommand::Schema { api } => {
                let value = if *api {
                    target.api_schema()
                } else {
                    settled(&target.call(&Request::Schema)?, out, false)?
                };
                writeln!(out, "{}", pretty(&value)).map_err(written)?;
            }
            ActionsCommand::Events {
                since,
                follow,
                max,
                json,
            } => events(&mut target, out, *since, *follow, *max, *json)?,
            ActionsCommand::Shot { path, pane, json } => {
                let request = shot_request(path.as_deref(), pane.as_deref())?;
                let value = settled(&target.call(&request)?, out, *json)?;
                if !*json {
                    let path = value
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Failure::new(Code::Failed, "the app answered no path"))?;
                    writeln!(out, "{path}").map_err(written)?;
                }
            }
            ActionsCommand::Apps { .. } => {}
        }
        Ok(())
    }

    /// What `args` asks: the app's socket, or its list started here.
    fn target(&self, args: &ActionsArgs) -> Result<Target, Failure> {
        if args.headless {
            let Some(open) = &self.headless else {
                let app = self.app.as_deref().unwrap_or("this command");
                return Err(Failure::new(
                    Code::BadRequest,
                    format!("{app} has no headless mode: ask a running app (--app)"),
                ));
            };
            let mut app = open().map_err(|e| Failure::new(Code::Failed, e))?;
            app.settle();
            return Ok(Target::Here(app));
        }
        let path = match args.app.as_deref().or(self.app.as_deref()) {
            Some(app) => {
                endpoint::resolve(app).map_err(|e| Failure::new(Code::BadRequest, e.to_string()))?
            }
            None => only_app()?,
        };
        Client::connect(&path).map(Target::Socket)
    }
}

/// The id `actions shot` runs.
pub const SHOT_ACTION: &str = "APP_SHOT";

/// `actions shot [path] [--pane id]` as the run it is: the path made
/// absolute here (the app's working folder is not the caller's).
fn shot_request(path: Option<&std::path::Path>, pane: Option<&str>) -> Result<Request, Failure> {
    let path = path.unwrap_or_else(|| std::path::Path::new("app-shot.png"));
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err(Failure::new(Code::BadRequest, "a shot's path ends in .png"));
    }
    let path = std::path::absolute(path)
        .map_err(|e| Failure::new(Code::BadRequest, format!("{}: {e}", path.display())))?;
    let mut args = serde_json::Map::new();
    args.insert("path".to_owned(), Value::String(path.display().to_string()));
    if let Some(pane) = pane {
        args.insert("pane".to_owned(), Value::String(pane.to_owned()));
    }
    Ok(Request::Run {
        id: SHOT_ACTION.to_owned(),
        args: Some(Value::Object(args)),
        ticket: None,
    })
}

/// The one app serving a socket here, for a command that names none.
fn only_app() -> Result<PathBuf, Failure> {
    let answering: Vec<PathBuf> = endpoint::sockets()
        .into_iter()
        .filter(|p| Client::connect(p).is_ok())
        .collect();
    match answering.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(Failure::new(
            Code::Unavailable,
            format!(
                "no app serves a control socket in {}",
                endpoint::socket_dir().display()
            ),
        )),
        many => Err(Failure::new(
            Code::BadRequest,
            format!(
                "{} apps are running; name one with --app ({})",
                many.len(),
                many.iter()
                    .filter_map(|p| endpoint::app_of(p))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// Who answers: a running app, or its list started here.
enum Target {
    Socket(Client),
    Here(Box<dyn Headless>),
}

impl Target {
    fn call(&mut self, request: &Request) -> Result<Reply, Failure> {
        match self {
            Self::Socket(client) => client.call(request),
            Self::Here(app) => Ok(here(app.as_mut(), request)),
        }
    }

    fn api_schema(&self) -> Value {
        match self {
            Self::Socket(client) => {
                let app = endpoint::app_of(client.path()).unwrap_or_default();
                Surface::new(app, "", architect::action::ActionList::new()).api_schema()
            }
            Self::Here(app) => app.surface().api_schema(),
        }
    }
}

/// `request` answered by the list started here, as the socket would
/// answer it: on a thread of its own while the app is pumped.
fn here(app: &mut dyn Headless, request: &Request) -> Reply {
    let surface = app.surface();
    let request = request.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("actions-headless".to_owned())
        .spawn(move || {
            let answer =
                futures::executor::block_on(surface.answer(&ActionCaller::app(), &request));
            let _ = tx.send(Reply::from(&answer));
        });
    if spawned.is_err() {
        return Reply::Err(Failure::new(Code::Failed, "could not start a thread").to_json());
    }
    loop {
        let wait = if app.pump() { 0 } else { 4 };
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(reply) => return reply,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Reply::Err(
                    Failure::new(Code::Failed, "the request's thread failed").to_json(),
                );
            }
        }
    }
}

/// The reply's value when it is not a failure — printed as JSON first
/// when `json` — or the failure.
fn settled(reply: &Reply, out: &mut dyn Write, json: bool) -> Result<Value, Failure> {
    if json {
        let _ = writeln!(out, "{}", reply.value());
    }
    reply
        .failure()
        .map_or_else(|| Ok(reply.value().clone()), Err)
}

/// `--arg`s or `--json` as the action's arguments.
fn arguments(pairs: &[String], json: Option<&str>) -> Result<Option<Value>, Failure> {
    if let Some(text) = json {
        let text = read_maybe_stdin(text)?;
        return serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Failure::new(Code::BadRequest, format!("--json is not JSON: {e}")));
    }
    if pairs.is_empty() {
        return Ok(None);
    }
    let mut object = serde_json::Map::new();
    for pair in pairs {
        let (key, value) = if let Some((key, raw)) = pair.split_once(":=") {
            let value = serde_json::from_str(raw).map_err(|e| {
                Failure::new(Code::BadRequest, format!("--arg {pair}: not JSON: {e}"))
            })?;
            (key, value)
        } else if let Some((key, text)) = pair.split_once('=') {
            (key, Value::String(text.to_owned()))
        } else {
            return Err(Failure::new(
                Code::BadRequest,
                format!("--arg {pair}: write key=text or key:=json"),
            ));
        };
        if key.is_empty() {
            return Err(Failure::new(
                Code::BadRequest,
                format!("--arg {pair}: no key"),
            ));
        }
        object.insert(key.to_owned(), value);
    }
    Ok(Some(Value::Object(object)))
}

fn batch_steps(text: &str) -> Result<Vec<Step>, Failure> {
    let text = read_maybe_stdin(text)?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::new(Code::BadRequest, format!("the steps are not JSON: {e}")))?;
    let steps = json!({ "steps": value });
    match Request::parse("batch", &steps)? {
        Request::Batch { steps, .. } if value.is_array() => Ok(steps),
        _ => Err(Failure::new(
            Code::BadRequest,
            "the steps are a JSON array of {\"id\", \"args\"}",
        )),
    }
}

/// `text`, or stdin's whole content when it is `-`.
fn read_maybe_stdin(text: &str) -> Result<String, Failure> {
    if text != "-" {
        return Ok(text.to_owned());
    }
    let mut read = String::new();
    std::io::stdin()
        .read_to_string(&mut read)
        .map_err(|e| Failure::new(Code::BadRequest, format!("reading stdin: {e}")))?;
    Ok(read)
}

fn events(
    target: &mut Target,
    out: &mut dyn Write,
    since: Option<u64>,
    follow: bool,
    max: Option<u64>,
    json: bool,
) -> Result<(), Failure> {
    let written = |e: std::io::Error| Failure::new(Code::Failed, format!("writing: {e}"));
    // A look back starts at the oldest event kept; a follow from now.
    let mut since = if follow {
        since
    } else {
        Some(since.unwrap_or(0))
    };
    loop {
        let wait = follow.then(|| u64::try_from(MAX_WAIT.as_millis()).unwrap_or(30_000));
        let request = Request::Watch(Watch {
            since,
            max,
            wait_ms: wait,
        });
        let value = settled(&target.call(&request)?, out, false)?;
        if at(&value, "lost") == true {
            if json {
                writeln!(out, "{}", json!({"lost": true, "next": at(&value, "next")}))
                    .map_err(written)?;
            } else {
                writeln!(out, "lost: events were missed; read the state again").map_err(written)?;
            }
        }
        for event in at(&value, "events").as_array().into_iter().flatten() {
            if json {
                writeln!(out, "{event}").map_err(written)?;
            } else {
                writeln!(out, "{}", event_line(event)).map_err(written)?;
            }
        }
        out.flush().map_err(written)?;
        since = at(&value, "next").as_u64().or(since);
        if !follow {
            if !json {
                writeln!(out, "next: {}", since.unwrap_or(0)).map_err(written)?;
            }
            return Ok(());
        }
    }
}

fn event_line(event: &Value) -> String {
    let seq = at(event, "seq").as_u64().unwrap_or(0);
    let text = |key: &str| event[key].as_str().unwrap_or_default().to_owned();
    if event.get("ran").is_some() {
        let ok = if at(event, "ok") == true {
            "ok"
        } else {
            "failed"
        };
        format!("{seq}  ran {} ({ok})", text("ran"))
    } else if event.get("changed").is_some() {
        format!("{seq}  changed {} {}", text("changed"), text("which"))
    } else if event.get("registered").is_some() {
        format!("{seq}  registered {}", text("registered"))
    } else if event.get("unregistered").is_some() {
        format!("{seq}  unregistered {}", text("unregistered"))
    } else {
        format!("{seq}  {event}")
    }
}

/// The apps answering on a socket here (a socket left by one that died,
/// or another program's, is not one).
fn apps(out: &mut dyn Write, json: bool) -> Result<(), Failure> {
    let written = |e: std::io::Error| Failure::new(Code::Failed, format!("writing: {e}"));
    let found: Vec<Value> = endpoint::sockets()
        .into_iter()
        .filter_map(|path| {
            let reply = Client::connect(&path)
                .and_then(|mut c| c.call(&Request::Hello))
                .ok()?;
            reply.failure().is_none().then(|| {
                json!({
                    "app": endpoint::app_of(&path),
                    "socket": path.display().to_string(),
                    "hello": reply.value(),
                })
            })
        })
        .collect();
    if json {
        writeln!(out, "{}", Value::Array(found)).map_err(written)?;
        return Ok(());
    }
    if found.is_empty() {
        writeln!(
            out,
            "no app answers on a control socket in {}",
            endpoint::socket_dir().display()
        )
        .map_err(written)?;
    }
    for app in &found {
        let hello = at(app, "hello");
        writeln!(
            out,
            "{}  {} {}  {} actions  {}",
            at(app, "app").as_str().unwrap_or_default(),
            at(hello, "name").as_str().unwrap_or_default(),
            at(hello, "version").as_str().unwrap_or_default(),
            at(hello, "actions"),
            at(app, "socket").as_str().unwrap_or_default(),
        )
        .map_err(written)?;
    }
    Ok(())
}

/// What went wrong, told to a person: the error, then what helps — the
/// schema for bad arguments, the ticket for a call that needs approval.
fn tell(err: &mut dyn Write, failure: &Failure) -> std::io::Result<()> {
    writeln!(err, "error: {} ({})", failure.error, failure.code)?;
    match failure.code {
        Code::BadArgs => {
            if let Some(takes) = &failure.takes {
                writeln!(err, "it takes:\n{}", pretty(takes))?;
            }
        }
        Code::ConfirmationRequired => {
            let ticket = failure.ticket().unwrap_or("?");
            let expires = failure
                .confirmation
                .as_deref()
                .map_or_else(String::new, |t| {
                    format!(
                        " (it expires in {} s)",
                        t.expires_in_ms.checked_div(1000).unwrap_or(0)
                    )
                });
            writeln!(err, "ticket: {ticket}{expires}")?;
            writeln!(
                err,
                "a person must approve this call in the app first; once they have, run the \
                 same command again with --ticket {ticket}"
            )?;
        }
        Code::Unknown => writeln!(err, "find it with `actions list --filter …`")?,
        _ => {}
    }
    Ok(())
}

/// `value`'s field `key`, `null` when it has none.
fn at<'v>(value: &'v Value, key: &str) -> &'v Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn print_list(out: &mut dyn Write, value: &Value) -> std::io::Result<()> {
    let actions = at(value, "actions").as_array().cloned().unwrap_or_default();
    let width = actions
        .iter()
        .filter_map(|a| at(a, "id").as_str().map(str::len))
        .max()
        .unwrap_or(0);
    for a in &actions {
        let id = at(a, "id").as_str().unwrap_or_default();
        let effect = at(a, "effect").as_str().unwrap_or_default();
        let name = at(a, "name").as_str().unwrap_or_default();
        let mut line = format!("{id:<width$}  {effect:<11}  {name}");
        if let Some(description) = at(a, "description").as_str().filter(|d| !d.is_empty()) {
            line.push_str(" — ");
            line.push_str(description);
        }
        if at(a, "enabled") == false {
            let _ = write!(
                line,
                " (disabled: {})",
                at(a, "disabled_reason")
                    .as_str()
                    .unwrap_or("no reason given")
            );
        }
        writeln!(out, "{line}")?;
    }
    writeln!(out, "{} actions", actions.len())
}

fn print_detail(out: &mut dyn Write, value: &Value) -> std::io::Result<()> {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    writeln!(out, "{} — {}", text("id"), text("name"))?;
    if !text("description").is_empty() {
        writeln!(out, "{}", text("description"))?;
    }
    writeln!(
        out,
        "effect: {}   category: {}{}",
        text("effect"),
        text("category"),
        if text("group").is_empty() {
            String::new()
        } else {
            format!(" / {}", text("group"))
        }
    )?;
    if let Some(shortcut) = at(value, "shortcut").as_str() {
        writeln!(out, "shortcut: {shortcut}")?;
    }
    if at(value, "enabled") == false {
        writeln!(
            out,
            "disabled: {}",
            at(value, "disabled_reason")
                .as_str()
                .unwrap_or("no reason given")
        )?;
    }
    for (key, label) in [("takes", "takes"), ("gives", "gives")] {
        if value[key].is_null() {
            writeln!(out, "{label}: nothing")?;
        } else {
            writeln!(out, "{label}:\n{}", pretty(&value[key]))?;
        }
    }
    Ok(())
}

fn print_result(out: &mut dyn Write, value: &Value) -> std::io::Result<()> {
    match value {
        Value::Null => Ok(()),
        Value::String(text) => writeln!(out, "{text}"),
        other => writeln!(out, "{}", pretty(other)),
    }
}

fn print_batch(out: &mut dyn Write, value: &Value) -> std::io::Result<()> {
    if let Some(error) = at(value, "error").as_str() {
        return writeln!(out, "refused: {error}");
    }
    for r in at(value, "results").as_array().into_iter().flatten() {
        let id = at(r, "id").as_str().unwrap_or_default();
        let step = at(r, "step").as_u64().unwrap_or(0);
        if at(r, "ok") == true {
            writeln!(out, "{step}  {id}  ok  {}", at(r, "result"))?;
        } else {
            let error = at(at(r, "error"), "error")
                .as_str()
                .map_or_else(|| at(r, "error").to_string(), str::to_owned);
            writeln!(out, "{step}  {id}  failed  {error}")?;
        }
    }
    writeln!(out, "ran {} of {}", at(value, "ran"), at(value, "of"))
}

#[cfg(test)]
mod tests {
    use super::{Top, arguments, batch_steps};
    use clap::Parser as _;
    use serde_json::json;

    /// `key=text` is a string, `key:=json` anything; `--json` the whole.
    #[test]
    fn arguments_from_the_command_line() {
        let args = arguments(
            &["name=ada".into(), "count:=3".into(), "tags:=[\"a\"]".into()],
            None,
        )
        .expect("parses");
        assert_eq!(
            args,
            Some(json!({"name": "ada", "count": 3, "tags": ["a"]}))
        );
        assert_eq!(
            arguments(&[], Some(r#"{"x": true}"#)).expect("json"),
            Some(json!({"x": true}))
        );
        assert_eq!(arguments(&[], None).expect("none"), None);
        assert!(arguments(&["nokey".into()], None).is_err());
        assert!(arguments(&["=v".into()], None).is_err());
        assert!(arguments(&["n:=nope".into()], None).is_err());
        assert_eq!(batch_steps(r#"[{"id": "A_B"}]"#).expect("steps").len(), 1);
        assert!(batch_steps(r#"{"id": "A_B"}"#).is_err());
    }

    /// The command line the issue names parses.
    #[test]
    fn the_command_line_parses() {
        for line in [
            "actions list --filter hit --json",
            "actions describe DRUMS_HITS",
            "actions run DRUMS_HITS --arg track=kick --output json",
            "actions run DRUMS_HITS --json {}",
            "actions schema --api",
            "actions events --since 3 --follow",
            "actions --app task list",
            "actions list --headless",
            "actions apps",
        ] {
            let parsed = Top::try_parse_from(line.split(' '));
            assert!(parsed.is_ok(), "{line}: {:?}", parsed.err());
        }
        assert!(Top::try_parse_from("actions run X --arg a=b --json {}".split(' ')).is_err());
    }
}
