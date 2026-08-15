//! Sidebar — app sidebar layout system, shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Which side the sidebar sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarSide {
    #[default]
    Left,
    Right,
}

/// Shared state provided by [`SidebarProvider`].
///
/// `compact` is the "rail" / icon-only state — when true, sidebars
/// configured with [`SidebarCollapsible::Icon`] shrink to a narrow
/// icon strip and any [`SidebarLabel`] children render nothing.
#[derive(Clone, Copy, PartialEq)]
pub struct SidebarContext {
    pub compact: Signal<bool>,
    pub side: SidebarSide,
}

/// Convenience hook — returns the current sidebar `compact` flag
/// from the surrounding [`SidebarProvider`]. Use this to hide
/// labels, group titles, or anything else that should disappear in
/// rail mode.
pub fn use_sidebar_compact() -> Signal<bool> {
    use_context::<SidebarContext>().compact
}

// ---------------------------------------------------------------------------
// SidebarProvider
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarProviderProps {
    /// Start in compact (icon-rail) mode.
    #[props(default = false)]
    pub default_compact: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarProvider(props: SidebarProviderProps) -> Element {
    let compact = use_signal(|| props.default_compact);
    let ctx = SidebarContext {
        compact,
        side: SidebarSide::Left,
    };
    use_context_provider(|| ctx);

    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex min-h-screen w-full", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/// Visual variant for the sidebar panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarVariant {
    #[default]
    Default,
    Floating,
}

/// Collapsible behaviour.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarCollapsible {
    #[default]
    None,
    Icon,
}

#[derive(Props, Clone, PartialEq)]
pub struct SidebarProps {
    #[props(default)]
    pub side: SidebarSide,
    #[props(default)]
    pub variant: SidebarVariant,
    #[props(default)]
    pub collapsible: SidebarCollapsible,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Sidebar(props: SidebarProps) -> Element {
    let mut ctx = use_context::<SidebarContext>();
    // Update side in context so children/trigger know which side we are on.
    ctx.side = props.side;

    let compact = (ctx.compact)();
    let is_icon = props.collapsible == SidebarCollapsible::Icon;

    let width = if is_icon && compact { "w-14" } else { "w-64" };

    let variant_cls = match props.variant {
        SidebarVariant::Default => {
            let border = match props.side {
                SidebarSide::Left => "border-r",
                SidebarSide::Right => "border-l",
            };
            format!(
                "bg-sidebar text-sidebar-foreground border-sidebar-border flex flex-col h-full {border}"
            )
        }
        SidebarVariant::Floating => {
            "bg-sidebar text-sidebar-foreground rounded-lg ring-1 ring-sidebar-border shadow-sm m-2 flex flex-col h-[calc(100%-1rem)]".to_string()
        }
    };

    let state = if compact { "compact" } else { "expanded" };
    let variant_attr = match props.variant {
        SidebarVariant::Default => "default",
        SidebarVariant::Floating => "floating",
    };
    let collapsible_attr = match props.collapsible {
        SidebarCollapsible::None => "none",
        SidebarCollapsible::Icon => "icon",
    };

    rsx! {
        aside {
            class: crate::cn::merge_slice(&["transition-[width] duration-200 ease-linear overflow-hidden", variant_cls.as_str(), width, props.class.as_str()]),
            "data-state": state,
            "data-variant": variant_attr,
            "data-collapsible": collapsible_attr,
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarHeader
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarHeaderProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarHeader(props: SidebarHeaderProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-2 p-2", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarContent
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarContentProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarContent(props: SidebarContentProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex-1 flex flex-col gap-2 overflow-y-auto overflow-x-hidden", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarFooter
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarFooterProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarFooter(props: SidebarFooterProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-2 p-2", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarSeparator
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarSeparatorProps {
    #[props(default)]
    pub class: String,
}

#[component]
pub fn SidebarSeparator(props: SidebarSeparatorProps) -> Element {
    rsx! {
        hr {
            class: crate::cn::merge_slice(&["bg-sidebar-border mx-2 h-px border-none", props.class.as_str()]),
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarGroup
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarGroupProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarGroup(props: SidebarGroupProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["relative flex flex-col p-2", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarGroupLabel
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarGroupLabelProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarGroupLabel(props: SidebarGroupLabelProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["text-sidebar-foreground/70 flex h-8 items-center rounded-md px-3 text-xs font-medium [&>svg]:size-4", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarGroupContent
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarGroupContentProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarGroupContent(props: SidebarGroupContentProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-1 text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarMenu
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarMenuProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarMenu(props: SidebarMenuProps) -> Element {
    rsx! {
        ul {
            class: crate::cn::merge_slice(&["flex flex-col gap-1", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarMenuItem
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarMenuItemProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarMenuItem(props: SidebarMenuItemProps) -> Element {
    rsx! {
        li {
            class: crate::cn::merge_slice(&["relative", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarMenuButton
// ---------------------------------------------------------------------------

/// Size of a [`SidebarMenuButton`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarMenuButtonSize {
    Small,
    #[default]
    Default,
    Large,
}

/// Visual variant of a [`SidebarMenuButton`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarMenuButtonVariant {
    #[default]
    Default,
    Outline,
}

#[derive(Props, Clone, PartialEq)]
pub struct SidebarMenuButtonProps {
    #[props(default = false)]
    pub is_active: bool,
    #[props(default)]
    pub size: SidebarMenuButtonSize,
    #[props(default)]
    pub variant: SidebarMenuButtonVariant,
    #[props(default)]
    pub on_click: Option<Callback<()>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarMenuButton(props: SidebarMenuButtonProps) -> Element {
    let base = "flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors cursor-pointer select-none";

    let variant_cls = match props.variant {
        SidebarMenuButtonVariant::Default => {
            "hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        }
        SidebarMenuButtonVariant::Outline => {
            "bg-background hover:bg-sidebar-accent hover:text-sidebar-accent-foreground ring-1 ring-sidebar-border"
        }
    };

    let active_cls = if props.is_active {
        "bg-sidebar-accent text-sidebar-accent-foreground font-medium"
    } else {
        ""
    };

    let size_cls = match props.size {
        SidebarMenuButtonSize::Small => "h-8 text-xs",
        SidebarMenuButtonSize::Default => "h-9 text-sm",
        SidebarMenuButtonSize::Large => "h-14 text-sm",
    };

    let data_active = props.is_active.then_some("true");

    rsx! {
        button {
            class: crate::cn::merge_slice(&[base, variant_cls, active_cls, size_cls, props.class.as_str()]),
            r#type: "button",
            "data-active": data_active,
            onclick: move |_| {
                if let Some(cb) = &props.on_click {
                    cb.call(());
                }
            },
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarMenuBadge
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarMenuBadgeProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarMenuBadge(props: SidebarMenuBadgeProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge_slice(&["text-sidebar-foreground flex h-5 min-w-5 items-center justify-center rounded-md px-1 text-xs font-medium", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarTrigger
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarTriggerProps {
    #[props(default)]
    pub class: String,
}

#[component]
pub fn SidebarTrigger(props: SidebarTriggerProps) -> Element {
    let mut ctx = use_context::<SidebarContext>();

    rsx! {
        button {
            class: crate::cn::merge_slice(&["inline-flex items-center justify-center size-8 rounded-lg hover:bg-sidebar-accent transition-colors", props.class.as_str()]),
            r#type: "button",
            onclick: move |_| {
                let current = (ctx.compact)();
                ctx.compact.set(!current);
            },
            svg {
                class: "size-4",
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                // Hamburger / panel-left icon
                rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
                path { d: "M9 3v18" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarLabel — auto-hides in compact mode
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarLabelProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Wrap any text/label that should disappear when the sidebar is in
/// compact (icon-only / rail) mode. Reads
/// [`SidebarContext::compact`] from context.
#[component]
pub fn SidebarLabel(props: SidebarLabelProps) -> Element {
    let compact = (use_context::<SidebarContext>().compact)();
    if compact {
        return rsx! {};
    }
    rsx! {
        span {
            class: crate::cn::merge_slice(&["truncate", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SidebarInset
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SidebarInsetProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn SidebarInset(props: SidebarInsetProps) -> Element {
    rsx! {
        main {
            class: crate::cn::merge_slice(&["flex-1 bg-background", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Collapsible sidebar with header, nav groups, footer, and an inset content area.
#[story(category = "Sidebar", name = "default")]
pub fn sidebar_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            SidebarProvider {
                Sidebar { collapsible: SidebarCollapsible::Icon,
                    SidebarHeader {
                        div { class: "flex items-center gap-2 px-2",
                            div { class: "size-8 rounded-md bg-primary text-primary-foreground flex items-center justify-center text-xs font-bold", "FT" }
                            span { class: "text-sm font-semibold", "FastTrack" }
                        }
                    }
                    SidebarSeparator {}
                    SidebarContent {
                        SidebarGroup {
                            SidebarGroupLabel { "Workspace" }
                            SidebarGroupContent {
                                SidebarMenu {
                                    SidebarMenuItem {
                                        SidebarMenuButton { is_active: true, "Dashboard" }
                                    }
                                    SidebarMenuItem {
                                        SidebarMenuButton { "Projects" }
                                    }
                                    SidebarMenuItem {
                                        SidebarMenuButton {
                                            "Inbox"
                                            SidebarMenuBadge { "4" }
                                        }
                                    }
                                }
                            }
                        }
                        SidebarGroup {
                            SidebarGroupLabel { "Settings" }
                            SidebarGroupContent {
                                SidebarMenu {
                                    SidebarMenuItem {
                                        SidebarMenuButton { "Account" }
                                    }
                                    SidebarMenuItem {
                                        SidebarMenuButton { "Billing" }
                                    }
                                }
                            }
                        }
                    }
                    SidebarFooter {
                        SidebarTrigger {}
                    }
                }
                SidebarInset {
                    div { class: "p-6", "Main content area." }
                }
            }
        }
    }
}
