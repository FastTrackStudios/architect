//! The lab on its Action List, for the guide scenarios
//! (`apps/sandbox/scenarios`): where it is, read back by a check, and each
//! move heard as a `lab` change.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionList, ActionMeta, Effect};
use dioxus::prelude::*;
use facet::Facet;

use crate::{Lab, Route};

/// What `LAB_STATE` answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct LabState {
    /// The tab on show (`inbox`, `tasks`, `schedule`, `vault`).
    pub tab: String,
    /// `list`, `item`, `capture` or `help`.
    pub page: String,
    /// A phone's bar at the foot: `actions`, or `editing` while a field
    /// is typed in. Read when asked.
    pub bar: String,
}

impl LabState {
    fn of(route: Route) -> Self {
        let page = match route {
            Route::Tab(_) => "list",
            Route::Item(..) => "item",
            Route::Capture(_) => "capture",
            Route::Help(_) => "help",
        };
        Self {
            tab: route.tab().to_owned(),
            page: page.to_owned(),
            bar: String::new(),
        }
    }
}

static STATE: ActionMeta = ActionMeta {
    id: "LAB_STATE",
    trait_name: "",
    method_name: "",
    display_name: "Where the lab is",
    description: "The tab on show, whether it is the list, an item or a capture, and whether a phone's bar is the action bar or the editing bar.",
    category: "Lab",
    group: "Go",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static GO: ActionMeta = ActionMeta {
    id: "LAB_GO",
    trait_name: "",
    method_name: "",
    display_name: "Go to a tab",
    description: "Shows tab `tab` as ⌃⌘1–4 do: the Architect app's inbox, vault, project or schedule (each opening its surface: the inbox deck, the last note, the projects list, the calendar's day), the sandbox's inbox, tasks, schedule or vault. The Vault's workspace actions (WORKSPACE_*) are on the list while it shows.",
    category: "Lab",
    group: "Go",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What `LAB_GO` takes.
#[derive(Debug, Clone, Facet)]
pub struct GoArgs {
    /// The Architect app's `inbox`, `vault`, `project` or `schedule`; the
    /// sandbox's `inbox`, `tasks`, `schedule` or `vault`.
    pub tab: String,
}

/// Puts `LAB_STATE` and `LAB_GO` on `list`, and tells it (`lab`) when
/// the lab moves.
pub fn use_lab_actions(list: &Arc<ActionList>, lab: &Lab, opener: &crate::favourites::Opener) {
    let opener = *opener;
    let lab = *lab;
    // `LAB_GO` runs on the window's thread, where the route lives.
    let goes = use_coroutine(
        move |mut asks: UnboundedReceiver<&'static str>| async move {
            use futures_util::StreamExt as _;
            while let Some(tab) = asks.next().await {
                lab.go_tab(tab, &opener);
            }
        },
    );
    use_hook({
        let list = list.clone();
        move || {
            let tx = goes.tx();
            let _ = list.register(&GO, Effect::Mutating, move |a: GoArgs| {
                let tab = crate::tabs()
                    .iter()
                    .map(|(_, _, t)| *t)
                    .find(|t| *t == a.tab)
                    .ok_or_else(|| {
                        format!(
                            "no tab {:?}: {}",
                            a.tab,
                            crate::tabs().iter().map(|(_, _, t)| *t).collect::<Vec<_>>().join(", ")
                        )
                    })?;
                tx.unbounded_send(tab)
                    .map_err(|_| "the lab is gone".to_owned())?;
                Ok::<_, String>(LabState {
                    tab: tab.to_owned(),
                    page: "list".to_owned(),
                    bar: String::new(),
                })
            });
        }
    });
    let kept = use_hook(|| Arc::new(Mutex::new(LabState::of(*lab.route.peek()))));
    use_hook({
        let list = list.clone();
        let kept = kept.clone();
        move || {
            let _ = list.register(&STATE, Effect::ReadOnly, move |(): ()| {
                let mut now = kept.lock().unwrap_or_else(PoisonError::into_inner).clone();
                let bar = if architect_shell::keyboard().editing() {
                    "editing"
                } else {
                    "actions"
                };
                bar.clone_into(&mut now.bar);
                Ok(now)
            });
        }
    });
    let told = list.clone();
    use_effect(move || {
        let now = LabState::of((lab.route)());
        let mut state = kept.lock().unwrap_or_else(PoisonError::into_inner);
        if *state != now {
            let tab = now.tab.clone();
            *state = now;
            drop(state);
            told.changed("lab", &tab);
        }
    });
}

/// The Architect tour's id (`apps/ui/tours/architect-tour.styx`):
/// every system of the framework, a chapter each, over the live lab.
pub const ARCHITECT_TOUR: &str = "architect-tour";

/// Where the person left the Architect tour, kept in the lab's folder
/// ([`crate::app_dir`]): a step's id, or `done`.
const TOUR_PLACE: &str = "tour-place";

/// The lab's guided tours: the Architect tour and two of its scenarios
/// (`apps/sandbox/scenarios`), played live, and in the Architect app the
/// capture tour (`architect_capture::TOUR`). The lab's data is all made
/// up, so it is its own playground: every tour may play (each the
/// Architect app offers is `safe`). `remember` keeps the place a person
/// left each tour in the lab's folder (a window's lab; headless, nothing
/// is kept). Extensions' tours join them while they run
/// ([`use_extension_tours`]).
#[must_use]
pub fn tours(remember: bool) -> architect_guide::tour::TourSet {
    let mut sources = vec![
        include_str!("../tours/architect-tour.styx"),
        include_str!("../../sandbox/scenarios/palette-run.styx"),
        include_str!("../../sandbox/scenarios/stacked-tabs.styx"),
    ];
    if !crate::distribution().is_sandbox() {
        sources.push(architect_capture::TOUR);
    }
    let set = architect_guide::tour::TourSet::from_sources(sources)
        .unwrap_or_default()
        .resolver(architect_workspace::target_selector)
        .resolver(architect_commands::target_selector)
        .playground(|_| Ok(()));
    if !remember {
        return set;
    }
    set.on_move(|moved| {
        use architect_guide::tour::Moved;
        let (scenario, place) = match moved {
            Moved::At { scenario, step } | Moved::Left { scenario, step } => {
                (scenario, step.as_str())
            }
            Moved::Finished { scenario } => (scenario, "done"),
        };
        keep_place(scenario, place);
    })
}

/// Where the person left the tours other than the Architect tour, kept
/// in the lab's folder: `{tour: step id, or "done"}`.
const TOUR_PLACES: &str = "tour-places.json";

/// The extensions whose tour the app has invited the person to take, a
/// line each, kept in the lab's folder: each is invited once.
const TOURS_INVITED: &str = "tours-invited";

/// Keeps where the person is in tour `scenario`: a step's id, or `done`.
fn keep_place(scenario: &str, place: &str) {
    let Some(dir) = crate::app_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if scenario == ARCHITECT_TOUR {
        let _ = std::fs::write(dir.join(TOUR_PLACE), place);
        return;
    }
    let mut places = places();
    places.insert(scenario.to_owned(), place.to_owned());
    if let Ok(text) = serde_json::to_string_pretty(&places) {
        let _ = std::fs::write(dir.join(TOUR_PLACES), text);
    }
}

/// Every kept place but the Architect tour's.
fn places() -> std::collections::BTreeMap<String, String> {
    crate::app_dir()
        .and_then(|d| std::fs::read_to_string(d.join(TOUR_PLACES)).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Where the person left tour `id`.
#[must_use]
pub fn place_of(id: &str) -> TourPlace {
    if id == ARCHITECT_TOUR {
        return tour_place();
    }
    match places().get(id).map(|p| p.trim().to_owned()) {
        None => TourPlace::Never,
        Some(p) if p.is_empty() || p == "done" => TourPlace::Finished,
        Some(step) => TourPlace::At(step),
    }
}

/// Where the person left the Architect tour, as kept in the lab's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TourPlace {
    /// They never started it.
    Never,
    /// They played it to its end.
    Finished,
    /// At this step, by id.
    At(String),
}

/// Where the person left the Architect tour.
#[must_use]
pub fn tour_place() -> TourPlace {
    let Some(kept) =
        crate::app_dir().and_then(|d| std::fs::read_to_string(d.join(TOUR_PLACE)).ok())
    else {
        return TourPlace::Never;
    };
    match kept.trim() {
        "" | "done" => TourPlace::Finished,
        step => TourPlace::At(step.to_owned()),
    }
}

/// Every tour, by who ships it.
pub const LIST_TOURS: &str = "TOURS_LIST";
/// Starts a tour where the person left it.
pub const START_TOUR: &str = "TOUR_START";
/// The palette's source of tour rows (its prefix: `tours:`).
const TOUR_ROWS: &str = "tours";

/// One tour, as `TOURS_LIST` answers: what ⌘K, a Tours page or an agent
/// shows for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct TourRow {
    /// The scenario's id: what `TOUR_START` takes.
    pub id: String,
    pub title: String,
    pub description: String,
    /// The extension that ships it (`[[tours]]`); empty for the app's own.
    pub extension: String,
    /// Its row: "Take the Projects tour".
    pub label: String,
    /// Where the person left it: `never`, `finished`, or a step's id.
    pub place: String,
}

/// What `TOUR_START` takes: a tour by `id`, or an extension's first by
/// `extension`.
#[derive(Debug, Clone, Default, Facet)]
pub struct StartArgs {
    #[facet(default)]
    pub id: Option<String>,
    #[facet(default)]
    pub extension: Option<String>,
    /// The step to start at, by id. Unset: where the person left it, or
    /// the first once they finished it.
    #[facet(default)]
    pub step: Option<String>,
    /// From its first step, wherever they left it.
    #[facet(default)]
    pub restart: bool,
}

static LIST_TOURS_META: ActionMeta = ActionMeta {
    id: LIST_TOURS,
    trait_name: "",
    method_name: "",
    display_name: "Tours",
    description: "Every guided tour: the app's own and each running extension's (`[[tours]]`): its id, title, description, the extension that ships it (empty for the app's), its row (\"Take the Projects tour\") and where the person left it (never, finished, or a step's id).",
    category: "Guide",
    group: "Tours",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static START_TOUR_META: ActionMeta = ActionMeta {
    id: START_TOUR,
    trait_name: "",
    method_name: "",
    display_name: "Take a tour",
    description: "Starts tour `id` (TOURS_LIST names them), or extension `extension`'s first tour, where the person left it: at `step` when given, from the start with `restart` or once they finished it. With neither, it opens ⌘K on the tours (`tours:`) to pick one.",
    category: "Guide",
    group: "Tours",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// A tour to start on the window's thread: its id, and the step.
type Start = (String, Option<String>);

/// The tours running extensions ship (`[[tours]]`) offered beside the
/// app's own, `TOURS_LIST` and `TOUR_START` on `list`, the manager's
/// "Take the tour" ([`architect_extensions::host::Host::on_tour`]), and
/// a card inviting the person to take an extension's tour once, after
/// they install it or turn it on (`TURNED_ON`). An extension turned off
/// takes its tours with it.
pub fn use_extension_tours(
    list: &Arc<ActionList>,
    tours: &architect_guide::tour::Tours,
    ext: architect_extensions::interface::ExtensionUi,
    remember: bool,
) {
    let tours = *tours;
    let rows = use_hook(|| Arc::new(Mutex::new(Vec::<TourRow>::new())));
    let owners = use_hook(|| CopyValue::new(std::collections::BTreeSet::<String>::new()));
    // Each change to what extensions add: their tours into the set, when
    // they changed (a status item's text changes it often; a development
    // copy's saved tour is new text).
    let mut seen = use_hook(|| CopyValue::new(None::<Vec<(String, String, String)>>));
    {
        let rows = rows.clone();
        use_effect(move || {
            let _ = ext.revision();
            let host = ext.host();
            let now: Vec<(String, String, String)> = host
                .contributions()
                .tours()
                .into_iter()
                .map(|t| (t.owner, t.item.id, t.item.scenario_text.unwrap_or_default()))
                .collect();
            if seen.peek().as_ref() != Some(&now) {
                seen.set(Some(now));
                sync(&host, &tours, owners, &rows);
            }
        });
    }
    let held = list.clone();
    let starts = use_coroutine(move |mut asks: UnboundedReceiver<Start>| {
        let list = held.clone();
        async move {
            use futures_util::StreamExt as _;
            while let Some((id, step)) = asks.next().await {
                // No tour named (⌘K's bare "Take a tour"): the palette,
                // on the tours.
                if id.is_empty() {
                    let _ = list
                        .run_json_async(
                            "COMMANDS_SHOW",
                            format!(r#"{{"overlay":"palette","query":"{TOUR_ROWS}: "}}"#)
                                .as_bytes(),
                        )
                        .await;
                    continue;
                }
                // The settings window (where the manager's "Take the tour"
                // is) would cover the tour: put it away first.
                let _ = list
                    .run_json_async(
                        "EXT_WINDOW_CLOSE",
                        br#"{"extension":"settings","id":"settings"}"#,
                    )
                    .await;
                let _ = tours.start_at(&id, step.as_deref());
            }
        }
    });
    use_hook({
        let (list, rows) = (list.clone(), rows.clone());
        move || {
            let read = rows.clone();
            let _ = list.register(&LIST_TOURS_META, Effect::ReadOnly, move |(): ()| {
                Ok::<_, String>(with_places(
                    &read.lock().unwrap_or_else(PoisonError::into_inner),
                ))
            });
            let tx = starts.tx();
            let _ = list.register(&START_TOUR_META, Effect::Mutating, move |a: StartArgs| {
                if a.id.is_none() && a.extension.is_none() {
                    tx.unbounded_send((String::new(), None))
                        .map_err(|_| "the window is gone".to_owned())?;
                    return Ok(TourRow::default());
                }
                let row = pick(&rows.lock().unwrap_or_else(PoisonError::into_inner), &a)?;
                let step = match (a.step, a.restart, place_of(&row.id)) {
                    (Some(step), _, _) | (None, false, TourPlace::At(step)) => Some(step),
                    _ => None,
                };
                tx.unbounded_send((row.id.clone(), step))
                    .map_err(|_| "the window is gone".to_owned())?;
                Ok::<_, String>(with_places(&[row]).remove(0))
            });
        }
    });
    // ⌘K: a row a tour, "Take the Projects tour", once something is typed
    // (or `tours:`); picking one is TOUR_START, where the person left it.
    {
        let rows = rows.clone();
        let commands = architect_commands::try_use_commands();
        use_hook(move || {
            let Some(commands) = commands else {
                return;
            };
            let tx = starts.tx();
            let read = rows;
            commands.register_source(
                architect_commands::FnSource::new(TOUR_ROWS, "Tours", move |_| {
                    read.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .iter()
                        // The app's own have their rows in Help already.
                        .filter(|r| !r.extension.is_empty())
                        .map(|r| {
                            architect_commands::engine::Item::new(r.id.clone(), r.label.clone())
                                .detail(r.description.clone())
                        })
                        .collect()
                })
                .ranking(architect_commands::Ranking::Commands),
                move |item: &architect_commands::engine::Item| {
                    let step = match place_of(&item.id) {
                        TourPlace::At(step) => Some(step),
                        _ => None,
                    };
                    let _ = tx.unbounded_send((item.id.clone(), step));
                },
            );
        });
        use_drop(move || {
            if let Some(c) = commands {
                c.remove_source(TOUR_ROWS);
            }
        });
    }
    // The manager's "Take the tour".
    {
        let rows = rows.clone();
        use_hook(move || {
            let tx = starts.tx();
            ext.host().on_tour(move |extension, tour| {
                let args = StartArgs {
                    id: tour.map(str::to_owned),
                    extension: Some(extension.to_owned()),
                    ..StartArgs::default()
                };
                let row = pick(&rows.lock().unwrap_or_else(PoisonError::into_inner), &args)?;
                tx.unbounded_send((row.id, None))
                    .map_err(|_| "the window is gone".to_owned())
            });
        });
    }
    // An extension installed or turned on: its tour, offered once.
    use_future(move || {
        let rows = rows.clone();
        async move {
            let host = ext.host();
            let heard = host.list().clone();
            let mut cursor = heard.latest();
            let mut invited = if remember {
                crate::app_dir()
                    .and_then(|d| std::fs::read_to_string(d.join(TOURS_INVITED)).ok())
                    .map(|t| t.lines().map(str::to_owned).collect())
                    .unwrap_or_default()
            } else {
                std::collections::BTreeSet::<String>::new()
            };
            loop {
                futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
                let (events, next) = heard.events_since(cursor, 64, std::time::Duration::ZERO);
                cursor = next;
                for e in events {
                    let architect::action::EventKind::Changed { topic, detail } = e.kind else {
                        continue;
                    };
                    if topic != architect_extensions::extensions::TURNED_ON
                        || invited.contains(&detail)
                    {
                        continue;
                    }
                    let Some(first) = host
                        .contributions()
                        .tours()
                        .into_iter()
                        .find(|t| t.owner == detail)
                    else {
                        continue;
                    };
                    sync(&host, &tours, owners, &rows);
                    tours.invite(&first.item.id);
                    invited.insert(detail);
                    if remember {
                        if let Some(dir) = crate::app_dir() {
                            let kept: Vec<&str> = invited.iter().map(String::as_str).collect();
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::fs::write(dir.join(TOURS_INVITED), kept.join("\n"));
                        }
                    }
                }
            }
        }
    });
}

/// Puts what running extensions ship into `tours` (each owner's in place
/// of what it had, an owner gone withdrawn) and `rows`.
fn sync(
    host: &architect_extensions::host::Host,
    tours: &architect_guide::tour::Tours,
    mut owners: CopyValue<std::collections::BTreeSet<String>>,
    rows: &Mutex<Vec<TourRow>>,
) {
    let mut by: std::collections::BTreeMap<String, Vec<architect_guide::Scenario>> =
        std::collections::BTreeMap::new();
    let mut described = std::collections::BTreeMap::new();
    for t in host.contributions().tours() {
        // Read and checked when it loaded; one that no longer reads is
        // left out.
        let Some(scenario) = t
            .item
            .scenario_text
            .as_deref()
            .and_then(|text| architect_guide::load(text).ok())
        else {
            continue;
        };
        described.insert(t.item.id.clone(), t.item.clone());
        by.entry(t.owner).or_default().push(scenario);
    }
    let gone: Vec<String> = owners
        .peek()
        .iter()
        .filter(|o| !by.contains_key(*o))
        .cloned()
        .collect();
    for owner in gone {
        tours.offer(&owner, Vec::new());
    }
    owners.set(by.keys().cloned().collect());
    for (owner, scenarios) in by {
        tours.offer(&owner, scenarios);
    }
    let names: std::collections::BTreeMap<String, String> = host
        .loaded()
        .into_iter()
        .map(|l| (l.manifest.id, l.manifest.name))
        .collect();
    let now = tours
        .offered()
        .into_iter()
        .map(|(id, title)| {
            let extension = tours.owner(&id).unwrap_or_default();
            let decl = described.get(&id);
            let title = decl.map_or(title, |d| d.title.clone());
            let name = names
                .get(&extension)
                .cloned()
                .unwrap_or_else(|| title.clone());
            TourRow {
                label: if !extension.is_empty() {
                    format!("Take the {name} tour")
                } else if title.to_lowercase().ends_with("tour") {
                    format!("Take the {title}")
                } else {
                    format!("Tour: {title}")
                },
                description: decl.map(|d| d.description.clone()).unwrap_or_default(),
                id,
                title,
                extension,
                place: String::new(),
            }
        })
        .collect();
    *rows.lock().unwrap_or_else(PoisonError::into_inner) = now;
}

/// `rows` with where the person left each.
fn with_places(rows: &[TourRow]) -> Vec<TourRow> {
    rows.iter()
        .cloned()
        .map(|mut r| {
            r.place = match place_of(&r.id) {
                TourPlace::Never => "never".to_owned(),
                TourPlace::Finished => "finished".to_owned(),
                TourPlace::At(step) => step,
            };
            r
        })
        .collect()
}

/// The tour `a` names: by id, or its extension's first.
fn pick(rows: &[TourRow], a: &StartArgs) -> Result<TourRow, String> {
    let found = match (&a.id, &a.extension) {
        (Some(id), _) => rows.iter().find(|r| &r.id == id),
        (None, Some(ext)) => rows.iter().find(|r| &r.extension == ext),
        (None, None) => return Err("name a tour (`id`) or an extension (`extension`)".to_owned()),
    };
    found.cloned().ok_or_else(|| match (&a.id, &a.extension) {
        (Some(id), _) => format!("no tour {id:?} (TOURS_LIST names them)"),
        (None, Some(ext)) => format!("extension {ext:?} ships no tour, or is not running"),
        (None, None) => String::new(),
    })
}

/// Every tour this app ships of its own, in the mode it runs.
///
/// The Architect app's capture tour, and each built-in extension's
/// `[[tours]]` it links in, read and checked as a load checks them
/// (`architect_extensions::Manifest::tours`). What the films play, so a
/// tour that rots fails CI (`apps/sandbox/media/tests`).
///
/// # Errors
///
/// A built-in's tour that does not read, named.
pub fn builtin_tours() -> Result<Vec<(String, architect_guide::Scenario)>, String> {
    use architect_extensions::linked::Extension;
    let sandbox = crate::distribution().is_sandbox();
    let mut linked: Vec<&dyn Extension> = vec![
        &architect_bases::ui::Bases,
        &architect_theme_picker::ThemePicker,
        &architect_navigator_extension::NavigatorExtension,
    ];
    if sandbox {
        linked.push(&architect_sandbox_board::Board);
    } else {
        linked.push(&architect_projects::Projects);
        linked.push(&architect_tasks::Tasks);
        linked.push(&architect_time_tracker::TimeTracker);
        linked.push(&architect_calendar::Calendar);
    }
    let mut out = Vec::new();
    if !sandbox {
        let capture = architect_guide::load(architect_capture::TOUR).map_err(|e| e.to_string())?;
        out.push((String::new(), capture));
    }
    for ext in linked {
        let manifest =
            architect_extensions::Manifest::parse(ext.manifest()).map_err(|e| e.to_string())?;
        let files = ext.files();
        let tours = manifest
            .tours(|path| {
                files
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, text)| (*text).to_owned())
            })
            .map_err(|e| format!("{}: {e}", manifest.id))?;
        out.extend(tours.into_iter().map(|(_, sc)| (manifest.id.clone(), sc)));
    }
    Ok(out)
}
