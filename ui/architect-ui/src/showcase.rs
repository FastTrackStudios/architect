//! Component showcase — compact, single-screen visual parity check
//! across renderers (web, desktop, native, mobile).
//!
//! ```rust,ignore
//! use architect_ui::showcase::Showcase;
//! rsx! { Showcase { renderer: "Web".to_string() } }
//! ```
//!
//! The layout packs every component family into a dense, auto-fit grid of
//! tiles so a reviewer can A/B two windows side-by-side and immediately
//! spot rendering differences in spacing, color, typography, and SVG
//! handling. Theme controls live in a sticky top bar so they apply to the
//! entire visible surface without scrolling.

use crate::prelude::*;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ShowcaseProps {
    /// Label shown in the top bar (e.g. "Web", "Desktop", "Native").
    #[props(default)]
    pub renderer: Option<String>,
}

#[component]
pub fn Showcase(props: ShowcaseProps) -> Element {
    let theme_state = use_signal(|| ThemeState::new(default_theme_preset(), ThemeMode::Dark));
    let renderer = props.renderer.unwrap_or_else(|| "Showcase".to_string());

    rsx! {
        ThemeProvider { state: theme_state,
            div { class: "min-h-screen bg-background text-foreground",
                ShowcaseTopBar { state: theme_state, renderer: renderer.clone() }

                main { class: "p-3",
                    // Dense auto-fit grid of compact tiles.
                    div { class: "grid gap-3 grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4 auto-rows-min items-start",

                        Tile { title: "Buttons · Variants",
                            Cluster {
                                Button { size: ButtonSize::Small, "Primary" }
                                Button { size: ButtonSize::Small, variant: ButtonVariant::Secondary, "Secondary" }
                                Button { size: ButtonSize::Small, variant: ButtonVariant::Outline, "Outline" }
                                Button { size: ButtonSize::Small, variant: ButtonVariant::Ghost, "Ghost" }
                                Button { size: ButtonSize::Small, variant: ButtonVariant::Destructive, "Destruct" }
                                Button { size: ButtonSize::Small, variant: ButtonVariant::Link, "Link" }
                            }
                        }

                        Tile { title: "Buttons · Sizes & States",
                            Cluster {
                                Button { size: ButtonSize::Small, "S" }
                                Button { size: ButtonSize::Medium, "M" }
                                Button { size: ButtonSize::Large, "L" }
                                Button { size: ButtonSize::Small, disabled: true, "Disabled" }
                                Button { size: ButtonSize::Small, loading: true, "Loading" }
                            }
                        }

                        Tile { title: "Inputs",
                            div { class: "flex flex-col gap-1.5",
                                Input { value: use_signal(|| "Text".to_string()), size: InputSize::Small }
                                Input { value: use_signal(String::new), placeholder: "Placeholder…".to_string(), size: InputSize::Small }
                                Input { value: use_signal(|| "Disabled".to_string()), disabled: true, size: InputSize::Small }
                                Input { value: use_signal(|| "Error".to_string()), variant: InputVariant::Error, size: InputSize::Small }
                                Textarea { value: use_signal(|| "Multi-line\nsecond line".to_string()) }
                            }
                        }

                        Tile { title: "Badges & Status",
                            div { class: "flex flex-col gap-2",
                                Cluster {
                                    Badge { variant: BadgeVariant::Default, "Default" }
                                    Badge { variant: BadgeVariant::Secondary, "Secondary" }
                                    Badge { variant: BadgeVariant::Destructive, "Destruct" }
                                    Badge { variant: BadgeVariant::Outline, "Outline" }
                                }
                                Cluster {
                                    StatusBadge { variant: StatusBadgeVariant::Success, label: "OK".to_string() }
                                    StatusBadge { variant: StatusBadgeVariant::Warning, label: "Warn".to_string() }
                                    StatusBadge { variant: StatusBadgeVariant::Danger, label: "Err".to_string() }
                                    StatusBadge { variant: StatusBadgeVariant::Neutral, label: "Off".to_string() }
                                }
                                Cluster {
                                    StatusDot { color: StatusDotColor::Success }
                                    StatusDot { color: StatusDotColor::Warning }
                                    StatusDot { color: StatusDotColor::Danger }
                                    StatusDot { color: StatusDotColor::Neutral }
                                    Kbd { "Ctrl" }
                                    Kbd { "K" }
                                }
                            }
                        }

                        Tile { title: "Checkbox · Switch · Radio",
                            div { class: "flex flex-col gap-1.5 text-sm",
                                Cluster {
                                    div { class: "flex items-center gap-1.5",
                                        crate::components::Checkbox { checked: use_signal(|| false) }
                                        Label { "Off" }
                                    }
                                    div { class: "flex items-center gap-1.5",
                                        crate::components::Checkbox { checked: use_signal(|| true) }
                                        Label { "On" }
                                    }
                                    div { class: "flex items-center gap-1.5",
                                        crate::components::Checkbox { checked: use_signal(|| false), disabled: true }
                                        Label { "Disabled" }
                                    }
                                }
                                Cluster {
                                    div { class: "flex items-center gap-1.5",
                                        crate::components::Switch { checked: use_signal(|| false) }
                                        Label { "Off" }
                                    }
                                    div { class: "flex items-center gap-1.5",
                                        crate::components::Switch { checked: use_signal(|| true) }
                                        Label { "On" }
                                    }
                                }
                                RadioGroup { value: use_signal(|| "a".to_string()),
                                    div { class: "flex items-center gap-3",
                                        div { class: "flex items-center gap-1.5",
                                            RadioGroupItem { value: "a".to_string() }
                                            Label { "A" }
                                        }
                                        div { class: "flex items-center gap-1.5",
                                            RadioGroupItem { value: "b".to_string() }
                                            Label { "B" }
                                        }
                                        div { class: "flex items-center gap-1.5",
                                            RadioGroupItem { value: "c".to_string() }
                                            Label { "C" }
                                        }
                                    }
                                }
                            }
                        }

                        Tile { title: "Select",
                            Select {
                                value: use_signal(String::new),
                                placeholder: "Choose…".to_string(),
                                SelectContent {
                                    SelectGroup {
                                        SelectLabel { "Fruits" }
                                        SelectItem { value: "apple".to_string(), index: 0, "Apple" }
                                        SelectItem { value: "banana".to_string(), index: 1, "Banana" }
                                        SelectItem { value: "cherry".to_string(), index: 2, "Cherry" }
                                    }
                                    SelectSeparator {}
                                    SelectGroup {
                                        SelectLabel { "Vegetables" }
                                        SelectItem { value: "carrot".to_string(), index: 3, "Carrot" }
                                        SelectItem { value: "potato".to_string(), index: 4, "Potato" }
                                    }
                                }
                            }
                        }

                        Tile { title: "Progress · Spinner · Skeleton",
                            div { class: "flex flex-col gap-2",
                                Progress { value: 75.0 }
                                Progress { value: 45.0, variant: ProgressVariant::Success }
                                Progress { value: 30.0, variant: ProgressVariant::Warning }
                                Progress { value: 90.0, variant: ProgressVariant::Destructive }
                                Cluster {
                                    Spinner { size: SpinnerSize::Small }
                                    Spinner {}
                                    Spinner { size: SpinnerSize::Large }
                                }
                                div { class: "flex items-center gap-2",
                                    SkeletonCircle { class: "size-7".to_string() }
                                    div { class: "flex-1 flex flex-col gap-1.5",
                                        SkeletonText {}
                                        Skeleton { class: "h-3 w-2/3".to_string() }
                                    }
                                }
                            }
                        }

                        Tile { title: "Alerts",
                            div { class: "flex flex-col gap-2",
                                Alert {
                                    AlertTitle { "Heads up" }
                                    AlertDescription { "Default alert with description." }
                                }
                                Alert { variant: AlertVariant::Destructive,
                                    AlertTitle { "Error" }
                                    AlertDescription { "Session expired." }
                                }
                            }
                        }

                        Tile { title: "Typography",
                            div { class: "flex flex-col gap-1",
                                Heading { level: HeadingLevel::H1, "H1 Heading" }
                                Heading { level: HeadingLevel::H2, "H2 Heading" }
                                Heading { level: HeadingLevel::H3, "H3 Heading" }
                                Heading { level: HeadingLevel::H4, "H4 Heading" }
                                Text { "Body text" }
                                Text { variant: TextVariant::Small, "Small text" }
                                Text { variant: TextVariant::Muted, "Muted text" }
                                Text { variant: TextVariant::Code, "Code text" }
                            }
                        }

                        Tile { title: "Tooltips",
                            Cluster {
                                Tooltip {
                                    TooltipTrigger { Button { size: ButtonSize::Small, variant: ButtonVariant::Outline, "Top" } }
                                    TooltipContent { side: TooltipSide::Top, "Tooltip top" }
                                }
                                Tooltip {
                                    TooltipTrigger { Button { size: ButtonSize::Small, variant: ButtonVariant::Outline, "Bot" } }
                                    TooltipContent { side: TooltipSide::Bottom, "Tooltip bottom" }
                                }
                                Tooltip {
                                    TooltipTrigger { Button { size: ButtonSize::Small, variant: ButtonVariant::Outline, "L" } }
                                    TooltipContent { side: TooltipSide::Left, "Tooltip left" }
                                }
                                Tooltip {
                                    TooltipTrigger { Button { size: ButtonSize::Small, variant: ButtonVariant::Outline, "R" } }
                                    TooltipContent { side: TooltipSide::Right, "Tooltip right" }
                                }
                            }
                        }

                        Tile { title: "Cards",
                            Card {
                                CardHeader {
                                    CardTitle { "Project Alpha" }
                                    CardDescription { "Sample card." }
                                }
                                CardContent {
                                    div { class: "flex flex-col gap-1.5",
                                        KeyValueRow { label: "Tasks".to_string(), value: "42".to_string() }
                                        KeyValueRow { label: "Done".to_string(), value: "38".to_string(), bold: true }
                                        KeyValueRow { label: "Overdue".to_string(), value: "2".to_string() }
                                    }
                                }
                                CardFooter {
                                    Button { size: ButtonSize::Small, variant: ButtonVariant::Primary, "Action" }
                                    Button { size: ButtonSize::Small, variant: ButtonVariant::Ghost, "Cancel" }
                                }
                            }
                        }

                        Tile { title: "Layout · ListRow / Divider / Empty",
                            div { class: "flex flex-col gap-2",
                                ListRow {
                                    label: "Signal REAPER".to_string(),
                                    detail: "Running — PID 1234".to_string(),
                                    leading: rsx! { StatusDot { color: StatusDotColor::Success } },
                                    trailing: rsx! { Button { size: ButtonSize::Small, variant: ButtonVariant::Secondary, "Launch" } },
                                }
                                ListRow {
                                    label: "Audio Interface".to_string(),
                                    detail: "Disconnected".to_string(),
                                    tag: "USB".to_string(),
                                    leading: rsx! { StatusDot { color: StatusDotColor::Danger } },
                                }
                                Divider {}
                                EmptyState { message: "No items".to_string() }
                            }
                        }

                        Tile { title: "Accordion",
                            Accordion {
                                AccordionItem { index: 0,
                                    AccordionTrigger { "Accessible?" }
                                    AccordionContent { "Yes — WAI-ARIA patterns." }
                                }
                                AccordionItem { index: 1,
                                    AccordionTrigger { "Styled?" }
                                    AccordionContent { "shadcn v4 maia." }
                                }
                                AccordionItem { index: 2,
                                    AccordionTrigger { "Animated?" }
                                    AccordionContent { "CSS transitions." }
                                }
                            }
                        }

                        Tile { title: "Tabs",
                            Tabs { default_value: "account".to_string(),
                                TabList {
                                    TabTrigger { value: "account".to_string(), index: 0, "Account" }
                                    TabTrigger { value: "password".to_string(), index: 1, "Password" }
                                    TabTrigger { value: "settings".to_string(), index: 2, "Settings" }
                                }
                                TabContent { value: "account".to_string(), index: 0,
                                    p { class: "text-sm text-muted-foreground p-2", "Account panel." }
                                }
                                TabContent { value: "password".to_string(), index: 1,
                                    p { class: "text-sm text-muted-foreground p-2", "Password panel." }
                                }
                                TabContent { value: "settings".to_string(), index: 2,
                                    p { class: "text-sm text-muted-foreground p-2", "Settings panel." }
                                }
                            }
                        }

                        Tile { title: "Icons (Lucide · currentColor)",
                            div { class: "flex flex-col gap-2",
                                Cluster {
                                    lucide_dioxus::Check { size: 18 }
                                    lucide_dioxus::X { size: 18 }
                                    lucide_dioxus::Search { size: 18 }
                                    lucide_dioxus::House { size: 18 }
                                    lucide_dioxus::Settings { size: 18 }
                                    lucide_dioxus::Bell { size: 18 }
                                    lucide_dioxus::Heart { size: 18 }
                                    lucide_dioxus::Star { size: 18 }
                                    lucide_dioxus::ChevronRight { size: 18 }
                                    lucide_dioxus::ChevronDown { size: 18 }
                                }
                                Cluster {
                                    span { class: "text-destructive", lucide_dioxus::CircleAlert { size: 18 } }
                                    span { class: "text-primary", lucide_dioxus::Info { size: 18 } }
                                    span { class: "text-chart-2", lucide_dioxus::CircleCheck { size: 18 } }
                                    span { class: "text-chart-4", lucide_dioxus::TriangleAlert { size: 18 } }
                                    span { class: "text-muted-foreground", lucide_dioxus::Circle { size: 18 } }
                                }
                                Cluster {
                                    lucide_dioxus::Star { size: 12 }
                                    lucide_dioxus::Star { size: 16 }
                                    lucide_dioxus::Star { size: 20 }
                                    lucide_dioxus::Star { size: 28 }
                                }
                                Cluster {
                                    Button { size: ButtonSize::Small, variant: ButtonVariant::Primary,
                                        lucide_dioxus::Download { size: 14 }
                                        "Download"
                                    }
                                    Button { size: ButtonSize::Small, variant: ButtonVariant::Outline,
                                        lucide_dioxus::Pencil { size: 14 }
                                        "Edit"
                                    }
                                    Button { size: ButtonSize::Small, variant: ButtonVariant::Destructive,
                                        lucide_dioxus::Trash { size: 14 }
                                        "Delete"
                                    }
                                }
                            }
                        }

                        Tile { title: "Scoped Theme (doom-64)",
                            ThemeScope {
                                styles: theme_preset("doom-64").unwrap_or_else(default_theme_preset).styles,
                                mode: Some(ThemeMode::Dark),
                                class: "rounded-lg border border-border p-3",
                                div { class: "flex flex-col gap-2",
                                    p { class: "font-mono text-xs text-muted-foreground",
                                        "Scoped tokens: radius, font, color isolated."
                                    }
                                    Cluster {
                                        Button { size: ButtonSize::Small, "Primary" }
                                        Button { size: ButtonSize::Small, variant: ButtonVariant::Outline, "Outline" }
                                        Badge { "Badge" }
                                    }
                                }
                            }
                        }

                        Tile { title: "Tables",
                            TableContainer {
                                Table {
                                    TableHeader {
                                        TableRow {
                                            TableHead { "Name" }
                                            TableHead { "Status" }
                                            TableHead { "Due" }
                                        }
                                    }
                                    TableBody {
                                        TableRow {
                                            TableCell { "Auth bug" }
                                            TableCell { Badge { variant: BadgeVariant::Default, "WIP" } }
                                            TableCell { "Apr 11" }
                                        }
                                        TableRow {
                                            TableCell { "Dashboard" }
                                            TableCell { Badge { variant: BadgeVariant::Secondary, "Open" } }
                                            TableCell { "Apr 15" }
                                        }
                                    }
                                }
                            }
                        }

                        Tile { title: "Forms", span: TileSpan::Two,
                            ShowcaseForm {}
                        }

                        Tile { title: "Data Table", span: TileSpan::Two,
                            ShowcaseDataTable {}
                        }

                        Tile { title: "Spacing parity (Tailwind gap vs HStack inline-style)", span: TileSpan::Two,
                            div { class: "flex flex-col gap-2",
                                for gap_val in ["1", "2", "3", "4", "6", "8"] {
                                    {
                                        let gap_val = gap_val.to_string();
                                        let gap_class_h = format!("flex gap-{gap_val}");
                                        rsx! {
                                            div { class: "flex items-center gap-3",
                                                span { class: "text-[10px] font-mono w-10 text-muted-foreground", "g-{gap_val}" }
                                                div { class: gap_class_h,
                                                    div { class: "w-5 h-5 rounded bg-primary" }
                                                    div { class: "w-5 h-5 rounded bg-primary" }
                                                    div { class: "w-5 h-5 rounded bg-primary" }
                                                }
                                                span { class: "text-[10px] text-muted-foreground", "Tailwind" }
                                                HStack { gap: gap_val.clone(),
                                                    div { class: "w-5 h-5 rounded bg-chart-2" }
                                                    div { class: "w-5 h-5 rounded bg-chart-2" }
                                                    div { class: "w-5 h-5 rounded bg-chart-2" }
                                                }
                                                span { class: "text-[10px] text-muted-foreground", "HStack" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Showcase Helpers (internal) ──────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Default)]
enum TileSpan {
    #[default]
    One,
    Two,
}

#[component]
fn Tile(title: String, #[props(default)] span: TileSpan, children: Element) -> Element {
    let span_class = match span {
        TileSpan::One => "",
        TileSpan::Two => "sm:col-span-2 lg:col-span-3 2xl:col-span-4",
    };
    rsx! {
        section {
            class: format!(
                "rounded-lg border border-border bg-card text-card-foreground shadow-xs flex flex-col {span_class}",
            ),
            header { class: "flex items-center gap-2 px-3 py-1.5 border-b border-border",
                span { class: "text-[11px] font-semibold uppercase tracking-wider text-muted-foreground",
                    "{title}"
                }
            }
            div { class: "p-3", {children} }
        }
    }
}

/// Tight horizontal flex-wrap used to pack variants of one component family.
#[component]
fn Cluster(children: Element) -> Element {
    rsx! {
        div { class: "flex flex-wrap items-center gap-1.5", {children} }
    }
}

#[component]
fn ShowcaseTopBar(state: Signal<ThemeState>, renderer: String) -> Element {
    let current = state();
    let preset_name = current.preset.clone();
    let preset_label = theme_presets()
        .into_iter()
        .find(|p| p.name == preset_name)
        .map(|p| p.label)
        .unwrap_or_else(|| preset_name.clone());
    let mode = current.mode;
    let mut preset_value = use_signal(|| preset_name.clone());

    use_effect(move || {
        preset_value.set(preset_name.clone());
    });

    let mut state_signal = state;

    rsx! {
        header { class: "sticky top-0 z-50 flex items-center gap-3 h-12 px-3 border-b border-border bg-card text-card-foreground shadow-sm",
            div { class: "flex items-center gap-2",
                StatusDot { color: StatusDotColor::Success }
                span { class: "text-sm font-semibold tracking-tight", "architect-ui · {renderer}" }
                Badge { variant: BadgeVariant::Outline, "{preset_label}" }
                Badge { variant: BadgeVariant::Secondary,
                    if mode == ThemeMode::Dark { "Dark" } else { "Light" }
                }
            }

            Divider { orientation: DividerOrientation::Vertical, class: "h-6".to_string() }

            div { class: "flex items-center gap-2 min-w-[16rem]",
                span { class: "text-xs text-muted-foreground", "Preset" }
                div { class: "flex-1",
                    Select {
                        value: preset_value,
                        placeholder: "Theme…".to_string(),
                        on_change: move |value: String| {
                            if let Some(p) = theme_preset(&value) {
                                state_signal.write().set_preset(p);
                            }
                        },
                        SelectContent {
                            for (i, p) in theme_presets().into_iter().enumerate() {
                                SelectItem { value: p.name, index: i, "{p.label}" }
                            }
                        }
                    }
                }
            }

            ButtonGroup {
                Button {
                    size: ButtonSize::Small,
                    variant: if mode == ThemeMode::Light { ButtonVariant::Primary } else { ButtonVariant::Outline },
                    on_click: move |_| state_signal.write().set_mode(ThemeMode::Light),
                    lucide_dioxus::Sun { size: 14 }
                    "Light"
                }
                Button {
                    size: ButtonSize::Small,
                    variant: if mode == ThemeMode::Dark { ButtonVariant::Primary } else { ButtonVariant::Outline },
                    on_click: move |_| state_signal.write().set_mode(ThemeMode::Dark),
                    lucide_dioxus::Moon { size: 14 }
                    "Dark"
                }
            }

            div { class: "flex-1" }

            span { class: "text-[11px] font-mono text-muted-foreground hidden sm:block",
                "Compare against other renderers · resize to test responsive grid"
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct ShowcaseTask {
    id: u32,
    title: &'static str,
    status: &'static str,
    owner: &'static str,
    priority: u32,
}

fn showcase_tasks() -> Vec<ShowcaseTask> {
    vec![
        ShowcaseTask {
            id: 101,
            title: "Wire account settings",
            status: "In Progress",
            owner: "Ada",
            priority: 3,
        },
        ShowcaseTask {
            id: 102,
            title: "Review invoice export",
            status: "Queued",
            owner: "Grace",
            priority: 2,
        },
        ShowcaseTask {
            id: 103,
            title: "Ship mobile shell",
            status: "Blocked",
            owner: "Linus",
            priority: 4,
        },
        ShowcaseTask {
            id: 104,
            title: "Tighten data table API",
            status: "Done",
            owner: "Barbara",
            priority: 1,
        },
    ]
}

fn showcase_task_columns() -> Vec<DataTableColumn<ShowcaseTask>> {
    vec![
        DataTableColumn::new("title", "Task", |task: &ShowcaseTask| {
            task.title.to_string()
        })
        .cell_render(task_title_cell)
        .dynamic_cell_class(task_title_class),
        DataTableColumn::new("status", "Status", |task: &ShowcaseTask| {
            task.status.to_string()
        })
        .cell_render(task_status_cell),
        DataTableColumn::new("owner", "Owner", |task: &ShowcaseTask| {
            task.owner.to_string()
        }),
        DataTableColumn::new("priority", "Priority", |task: &ShowcaseTask| {
            task.priority.to_string()
        })
        .sort_value(|task| task.priority.into())
        .cell_class("text-right")
        .head_class("text-right"),
    ]
}

fn task_title_cell(context: DataTableCellContext<ShowcaseTask>) -> Element {
    rsx! {
        div { class: "flex flex-col gap-1",
            span { class: "font-medium", "{context.value}" }
            span { class: "text-xs text-muted-foreground", "TASK-{context.row.id}" }
        }
    }
}

fn task_status_cell(context: DataTableCellContext<ShowcaseTask>) -> Element {
    let variant = match context.value.as_str() {
        "Blocked" => BadgeVariant::Destructive,
        "Done" => BadgeVariant::Secondary,
        _ => BadgeVariant::Default,
    };

    rsx! {
        Badge { variant, "{context.value}" }
    }
}

fn task_title_class(context: DataTableCellContext<ShowcaseTask>) -> String {
    if context.selected {
        "bg-muted/40".to_string()
    } else {
        String::new()
    }
}

fn task_row_class(context: DataTableRowContext<ShowcaseTask>) -> String {
    if context.row.status == "Blocked" {
        "bg-destructive/5 hover:bg-destructive/10".to_string()
    } else {
        String::new()
    }
}

#[component]
fn ShowcaseDataTable() -> Element {
    let rows = use_signal(showcase_tasks);
    let columns = use_signal(showcase_task_columns);
    let options = use_signal(|| {
        DataTableOptions::default()
            .get_row_id(|task: &ShowcaseTask, _| task.id.to_string())
            .row_class(task_row_class)
    });
    let table = use_data_table_with_options(rows, columns, options);

    rsx! {
        div { class: "flex flex-col gap-3",
            DataTableToolbar { table: table.clone() }
            DataTableView {
                table: table.clone(),
                selectable: true,
                empty: "No tasks match the current filters.".to_string(),
            }
            DataTableFooter { table }
        }
    }
}

#[component]
fn ShowcaseForm() -> Element {
    let mut form = use_form();
    let mut initialized = use_signal(|| false);
    if !initialized() {
        form.register("name", "Ada Lovelace");
        form.register("email", "ada@example.com");
        initialized.set(true);
    }

    let name = form.field("name");
    let email = form.field("email");
    let state = form.state();
    let submitted = state.submit_count > 0;
    let name_error = if submitted && name.value().trim().is_empty() {
        Some("Name is required.".to_string())
    } else {
        None
    };
    let email_error = if submitted && !email.value().contains('@') {
        Some("Enter a valid email address.".to_string())
    } else {
        None
    };

    rsx! {
        Form {
            class: "max-w-xl".to_string(),
            on_submit: move |_| {
                form.submit();
                form.validate_field("name", &[FormRule::required("Name is required.")]);
                form.validate_field("email", &[FormRule::email("Enter a valid email address.")]);
            },
            Field {
                FieldLabel { required: true, "Name" }
                input {
                    class: "h-9 w-full rounded-lg border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                    value: "{name.value()}",
                    oninput: name.oninput(),
                    onblur: name.onblur(),
                }
                FieldDescription { "Tracked with FormApi dirty and touched state." }
                FormMessage { error: name_error }
            }
            Field {
                FieldLabel { required: true, "Email" }
                input {
                    class: "h-9 w-full rounded-lg border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                    value: "{email.value()}",
                    oninput: email.oninput(),
                    onblur: email.onblur(),
                }
                FormMessage { error: email_error }
            }
            div { class: "flex items-center justify-between gap-3",
                p { class: "text-sm text-muted-foreground",
                    "Dirty: {state.dirty()} | Touched: {state.touched()} | Submits: {state.submit_count}"
                }
                Button { variant: ButtonVariant::Primary, "Validate" }
            }
        }
    }
}
