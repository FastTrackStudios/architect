//! Button — shadcn v4 maia style, replaces lumen-blocks wrapper.

use dioxus::prelude::*;

/// Button visual variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Outline,
    Ghost,
    Link,
    Destructive,
}

/// Button size.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ButtonSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Element used for the button root.
///
/// This is the Dioxus-native equivalent of the common shadcn `asChild` use
/// case: the component owns its styling and behavior, while callers can choose
/// the semantic root element when a real `<button>` is not appropriate.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum ButtonRenderAs {
    #[default]
    Button,
    Anchor {
        href: String,
    },
    Div,
    Span,
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    #[props(default)]
    pub variant: ButtonVariant,
    #[props(default)]
    pub size: ButtonSize,
    #[props(default)]
    pub render_as: ButtonRenderAs,
    #[props(default)]
    pub on_click: Option<Callback<MouseEvent>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = false)]
    pub loading: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    // shadcn v4 maia: cn-button base
    let base = "inline-flex items-center justify-center font-medium border border-transparent bg-clip-padding text-sm transition-colors focus-visible:outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] disabled:pointer-events-none disabled:opacity-50 [&_svg:not([class*='size-'])]:size-4";

    // shadcn v4 maia: cn-button-variant-*
    let variant = match props.variant {
        ButtonVariant::Primary => "bg-primary text-primary-foreground hover:bg-primary/80",
        ButtonVariant::Secondary => "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        ButtonVariant::Outline => {
            // Canonical shadcn outline: 1px solid `--border`, surface from
            // `--background`, text from `--foreground`. Avoid alpha
            // multipliers (`bg-input/30`) — blitz's compositor does not
            // alpha-blend backgrounds reliably, so they read as either
            // fully on or fully off depending on render order.
            "border-border bg-background text-foreground hover:bg-accent hover:text-accent-foreground"
        }
        ButtonVariant::Ghost => "text-foreground hover:bg-accent hover:text-accent-foreground",
        ButtonVariant::Destructive => {
            "bg-destructive/10 hover:bg-destructive/20 text-destructive dark:bg-destructive/20 dark:hover:bg-destructive/30 focus-visible:ring-destructive/20 focus-visible:border-destructive/40"
        }
        ButtonVariant::Link => "text-primary underline-offset-4 hover:underline border-none",
    };

    // shadcn v4 maia: cn-button-size-*
    let size = match props.size {
        ButtonSize::Small => "h-8 gap-1 px-3 rounded-lg",
        ButtonSize::Medium => "h-9 gap-1.5 px-3 rounded-lg",
        ButtonSize::Large => "h-10 gap-1.5 px-4 rounded-lg",
    };

    let disabled = props.disabled || props.loading;
    let class = crate::cn::merge_slice(&[base, variant, size, props.class.as_str()]);
    let children = rsx! {
        if props.loading {
            // Pure-CSS spinner (border-radius + one transparent edge),
            // not an SVG. Renders correctly across all renderers we
            // currently target. The earlier inline-SVG spinner trips
            // a Blitz-specific bug where small (<24px) inline SVGs
            // animated via Stylo CSS animations vanish from the paint
            // tree — see the `Diagnostics/svg-smoke` story for a
            // standalone repro and the upstream bug report.
            div {
                class: "size-4 rounded-full border-2 border-current border-t-transparent animate-spin",
            }
        }
        {props.children}
    };

    match props.render_as {
        ButtonRenderAs::Button => rsx! {
            button {
                class,
                // Emit `disabled` only when the button really is disabled.
                // `disabled: false` renders the attribute as `disabled="false"`,
                // and Blitz — like the HTML spec — treats the attribute's mere
                // presence as disabled, regardless of value. Combined with the
                // `disabled:pointer-events-none` in the base class that makes
                // every *enabled* button unclickable under Blitz (plugin
                // editors and the standalone shell alike). `None` omits the
                // attribute entirely.
                disabled: if disabled { Some("true") } else { None },
                r#type: "button",
                onclick: move |e| {
                    if let Some(cb) = &props.on_click {
                        cb.call(e);
                    }
                },
                {children}
            }
        },
        ButtonRenderAs::Anchor { href } => rsx! {
            a {
                class,
                href,
                role: "button",
                "aria-disabled": if disabled { "true" } else { "false" },
                tabindex: if disabled { "-1" } else { "0" },
                onclick: move |e| {
                    if disabled {
                        e.prevent_default();
                        return;
                    }
                    if let Some(cb) = &props.on_click {
                        cb.call(e);
                    }
                },
                {children}
            }
        },
        ButtonRenderAs::Div => rsx! {
            div {
                class,
                role: "button",
                "aria-disabled": if disabled { "true" } else { "false" },
                tabindex: if disabled { "-1" } else { "0" },
                onclick: move |e| {
                    if !disabled {
                        if let Some(cb) = &props.on_click {
                            cb.call(e);
                        }
                    }
                },
                {children}
            }
        },
        ButtonRenderAs::Span => rsx! {
            span {
                class,
                role: "button",
                "aria-disabled": if disabled { "true" } else { "false" },
                tabindex: if disabled { "-1" } else { "0" },
                onclick: move |e| {
                    if !disabled {
                        if let Some(cb) = &props.on_click {
                            cb.call(e);
                        }
                    }
                },
                {children}
            }
        },
    }
}
