//! The Appearance page (features `appearance` and `ui`): the theme
//! store's org, person and device layers as a settings section — mode,
//! theme cards drawn in their own colours, text size, density, corners,
//! contrast and font, each saying where its value comes from, with a reset
//! to the layer under it and "Reset to organization default".
//!
//! It reads and writes the store ([`architect_theme::use_theme`]), not
//! the settings' values: the store is the truth, and
//! [`crate::appearance::Persist`] keeps its layers.

// The `rsx!` loops iterate rows collected so a signal's read is let go
// before drawing.
#![allow(clippy::needless_collect)]

use architect_shell::{ShellTheme, try_use_shell};
use architect_theme::{
    Appearance, Layer, LayerKind, Layers, Resolved, Scheme, ThemeChoice, ThemeStore,
};
use dioxus::prelude::*;
use futures_util::StreamExt;
use lucide_dioxus::{Minus, Plus, RotateCcw};

use crate::appearance::{self, CORNERS, DENSITIES, FONTS, tokens};

/// Registers the Appearance section, drawn by [`AppearancePage`], while
/// the calling component is mounted. Under a theme store
/// (`architect_theme::use_theme_store`) and a settings provider.
pub fn use_appearance_section() {
    crate::ui::use_section_page(appearance::section, page);
}

fn page() -> Element {
    rsx! { AppearancePage {} }
}

fn theme() -> ShellTheme {
    try_use_shell().map(|s| s.theme).unwrap_or_default()
}

/// Who says a value now: a layer's name for a person.
const fn said_by(kind: Option<LayerKind>) -> &'static str {
    match kind {
        Some(LayerKind::Org) => "Your organization",
        Some(LayerKind::Person) => "You, on every device",
        Some(LayerKind::Device) => "This device",
        Some(LayerKind::App) => "The app",
        Some(LayerKind::Workspace) => "This workspace",
        Some(LayerKind::Pick | LayerKind::Preview) => "For now",
        Some(LayerKind::Builtin | LayerKind::Extension) | None => "The theme",
    }
}

/// The highest layer whose `get` says something, and what.
fn top<T>(layers: &Layers, get: impl Fn(&Layer) -> Option<T>) -> (Option<LayerKind>, Option<T>) {
    LayerKind::ALL
        .into_iter()
        .rev()
        .find_map(|k| get(&layers.get(k)).map(|v| (Some(k), Some(v))))
        .unwrap_or((None, None))
}

/// The store's layers as a signal that moves on every change to them.
fn use_layers(store: &ThemeStore) -> Signal<Layers> {
    let mut layers = use_signal(|| store.layers());
    let store = store.clone();
    use_hook(move || {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<Layers>();
        let subscription = store.subscribe_layers(move |l| {
            let _ = tx.unbounded_send(l.clone());
        });
        let now = store.layers();
        spawn(async move {
            let _subscription = subscription;
            if *layers.peek() != now {
                layers.set(now);
            }
            while let Some(mut next) = rx.next().await {
                while let Ok(later) = rx.try_recv() {
                    next = later;
                }
                if *layers.peek() != next {
                    layers.set(next);
                }
            }
        });
    });
    layers
}

/// The Appearance page. Mount it under a theme store; the settings page
/// draws it for the section [`use_appearance_section`] registers.
#[component]
pub fn AppearancePage() -> Element {
    let Some(handle) = architect_theme::try_use_theme() else {
        let t = theme();
        return rsx! { div { style: "color: {t.ink_3}; font-size: 13px;", "No theme store." } };
    };
    let store = handle.store();
    let layers = use_layers(&store);
    let mut target = use_signal(|| LayerKind::Person);
    let resolved = handle.resolved().read().clone();
    let layers = layers.read().clone();
    let kind = target();
    let locked = crate::ui::try_use_settings()
        .map(|h| h.values.read().org.locked.clone())
        .unwrap_or_default();
    let t = theme();
    let density = finite(resolved.number(tokens::DENSITY), 1.0);
    let scale = finite(resolved.number(tokens::TEXT_SCALE), 1.0);
    let ctx = Ctx {
        kind,
        density,
        scale,
    };
    let own = !layers.get(LayerKind::Person).is_empty() || !layers.get(LayerKind::Device).is_empty();
    let reset_store = store.clone();

    // Mode.
    let (mode_by, mode) = top(&layers, |l| l.appearance);
    let mode = mode.unwrap_or(Appearance::System);
    // Theme cards: every theme, drawn in its own colours.
    let (choice_by, _) = top(&layers, |l| l.choice.clone());
    let composed = store.composed();
    let cards = theme_cards(&store, &composed.choice);
    // Tokens.
    let token = |name: &str| top(&layers, |l| l.overrides.get(name).cloned());

    let target_names = [
        (LayerKind::Person, "All my devices"),
        (LayerKind::Device, "This device"),
    ];
    let pad = ctx.px(14.0);
    rsx! {
        div { style: "max-width: 760px; display: flex; flex-direction: column; gap: {ctx.px(22.0)}px; font-size: {ctx.text(13.0)}px;",
            // Where a change goes.
            div { style: "display: flex; align-items: center; gap: 12px; flex-wrap: wrap;",
                span { style: "color: {t.ink_2}; font-size: {ctx.text(12.0)}px;", "Changes apply to" }
                div { style: "display: flex; padding: 2px; gap: 2px; border-radius: 8px; background: {t.desk}; border: 1px solid {t.rule};",
                    for (k, label) in target_names {
                        Seg { key: "{label}", label: label.to_owned(), on: kind == k, onpick: move |()| target.set(k) }
                    }
                }
                span { style: "flex: 1;" }
                if own {
                    button {
                        style: "height: 28px; padding: 0 12px; border-radius: 7px; border: 1px solid {t.rule}; background: transparent; \
                                color: {t.ink_2}; font-size: 12px; cursor: pointer;",
                        onclick: move |_| {
                            let _ = appearance::reset_to_org(&reset_store);
                        },
                        "Reset to organization default"
                    }
                }
            }
            Group { title: "Mode",
                Row { title: "Mode", detail: "Follow the system, or always light or dark.", by: mode_by, kind, first: true, pad,
                    locked: locked.contains(appearance::MODE), clear: clear_mode(&store, kind),
                    div { style: "display: flex; gap: 8px;",
                        for (a, label, sample) in [
                            (Appearance::System, "System", None),
                            (Appearance::Light, "Light", Some(Scheme::Light)),
                            (Appearance::Dark, "Dark", Some(Scheme::Dark)),
                        ] {
                            ModeCard { key: "{label}", label: label.to_owned(), on: mode == a,
                                swatch: sample.map_or_else(|| resolved.clone(), |s| swatch_for(&store, composed.choice.pick(s))),
                                onpick: {
                                    let store = store.clone();
                                    move |()| { let _ = store.update(kind, |l| l.appearance = Some(a)); }
                                },
                            }
                        }
                    }
                }
            }
            for (scheme, title) in [(Scheme::Light, "Light theme"), (Scheme::Dark, "Dark theme")] {
                Group { key: "{title}", title: title.to_owned(),
                    div { style: "display: flex; flex-direction: column; gap: 10px; padding: {pad}px 16px;",
                        div { style: "display: flex; align-items: center; gap: 8px; color: {t.ink_3}; font-size: {ctx.text(12.0)}px;",
                            span { "{said_by(choice_by)}" }
                            if choice_by == Some(kind) {
                                ResetButton { label: "Reset the theme".to_owned(), onpress: clear_choice(&store, kind) }
                            }
                        }
                        div { style: "display: flex; flex-wrap: wrap; gap: 10px;",
                            for card in cards.iter().filter(|c| c.scheme == scheme).cloned() {
                                ThemeCard { key: "{card.id}", card: card.clone(), disabled: locked.contains(appearance::THEME),
                                    onpick: {
                                        let store = store.clone();
                                        let id = card.id;
                                        move |()| { let _ = store.pick_in(kind, &id); }
                                    },
                                }
                            }
                        }
                    }
                }
            }
            Group { title: "Text and spacing",
                TokenRow { ctx, store: store.clone(), name: tokens::TEXT_SCALE, title: "Text size",
                    detail: "How large text is everywhere.", first: true, said: token(tokens::TEXT_SCALE),
                    locked: locked.contains(&appearance::token_key(tokens::TEXT_SCALE)),
                    TextScale { now: scale, onpick: set_token(&store, kind, tokens::TEXT_SCALE) }
                }
                TokenRow { ctx, store: store.clone(), name: tokens::DENSITY, title: "Density",
                    detail: "How much room there is around things.", first: false, said: token(tokens::DENSITY),
                    locked: locked.contains(&appearance::token_key(tokens::DENSITY)),
                    Segmented {
                        options: DENSITIES.iter().map(|(v, l)| ((*l).to_owned(), v.to_string())).collect(),
                        now: density.to_string(),
                        onpick: set_token(&store, kind, tokens::DENSITY),
                    }
                }
                TokenRow { ctx, store: store.clone(), name: tokens::RADIUS, title: "Corners",
                    detail: "How round corners are.", first: false, said: token(tokens::RADIUS),
                    locked: locked.contains(&appearance::token_key(tokens::RADIUS)),
                    Segmented {
                        options: CORNERS.iter().map(|(v, l)| ((*l).to_owned(), (*v).to_owned())).collect(),
                        now: resolved.value(tokens::RADIUS).to_owned(),
                        onpick: set_token(&store, kind, tokens::RADIUS),
                    }
                }
                TokenRow { ctx, store: store.clone(), name: tokens::CONTRAST, title: "Contrast",
                    detail: "Quieter text and edges drawn stronger.", first: false, said: token(tokens::CONTRAST),
                    locked: locked.contains(&appearance::token_key(tokens::CONTRAST)),
                    Segmented {
                        options: vec![("Standard".to_owned(), "0".to_owned()), ("More".to_owned(), "1".to_owned())],
                        now: if resolved.number(tokens::CONTRAST) > 0.5 { "1".to_owned() } else { "0".to_owned() },
                        onpick: set_token(&store, kind, tokens::CONTRAST),
                    }
                }
                TokenRow { ctx, store: store.clone(), name: tokens::FONT, title: "Font",
                    detail: "The face everything is set in.", first: false, said: token(tokens::FONT),
                    locked: locked.contains(&appearance::token_key(tokens::FONT)),
                    Segmented {
                        options: FONTS.iter().map(|f| (f.label.to_owned(), f.stack.to_owned())).collect(),
                        now: resolved.value(tokens::FONT).to_owned(),
                        onpick: set_font(&store, kind),
                    }
                }
            }
        }
    }
}

fn finite(v: f32, or: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v } else { or }
}

/// The page's own sizes under the person's density and text size.
#[derive(Clone, Copy, PartialEq)]
struct Ctx {
    kind: LayerKind,
    density: f32,
    scale: f32,
}

impl Ctx {
    fn px(self, base: f32) -> f32 {
        (base * self.density).round()
    }

    fn text(self, base: f32) -> f32 {
        (base * self.scale * 10.0).round() / 10.0
    }
}

fn set_token(store: &ThemeStore, kind: LayerKind, name: &'static str) -> EventHandler<String> {
    let store = store.clone();
    EventHandler::new(move |v: String| {
        let _ = store.set_token(kind, name, &v);
    })
}

fn set_font(store: &ThemeStore, kind: LayerKind) -> EventHandler<String> {
    let store = store.clone();
    EventHandler::new(move |v: String| {
        let _ = store.update(kind, |l| {
            l.overrides.insert(tokens::FONT.to_owned(), v.clone());
            l.overrides.insert(tokens::FONT_SANS.to_owned(), v);
        });
    })
}

fn clear_mode(store: &ThemeStore, kind: LayerKind) -> EventHandler<()> {
    let store = store.clone();
    EventHandler::new(move |()| {
        let _ = store.update(kind, |l| l.appearance = None);
    })
}

fn clear_choice(store: &ThemeStore, kind: LayerKind) -> EventHandler<()> {
    let store = store.clone();
    EventHandler::new(move |()| {
        let _ = store.update(kind, |l| l.choice = None);
    })
}

/// One theme as a card shows it: its colours, from its own resolve.
#[derive(Clone, PartialEq)]
struct Card {
    id: String,
    label: String,
    scheme: Scheme,
    on: bool,
    desk: String,
    sheet: String,
    rule: String,
    ink: String,
    ink_3: String,
    brand: String,
}

fn swatch_for(store: &ThemeStore, id: &str) -> Resolved {
    store.resolve_with(
        LayerKind::Preview,
        Layer::default().choice(ThemeChoice::One(id.to_owned())),
    )
}

fn theme_cards(store: &ThemeStore, choice: &ThemeChoice) -> Vec<Card> {
    store
        .themes()
        .into_iter()
        .filter_map(|info| {
            let scheme = info.scheme?;
            let r = swatch_for(store, &info.id);
            Some(Card {
                on: choice.pick(scheme) == info.id,
                desk: r.value("desk").to_owned(),
                sheet: r.value("sheet").to_owned(),
                rule: r.value("rule").to_owned(),
                ink: r.value("ink").to_owned(),
                ink_3: r.value("ink-3").to_owned(),
                brand: r.value("brand").to_owned(),
                id: info.id,
                label: info.label,
                scheme,
            })
        })
        .collect()
}

#[component]
fn Group(title: String, children: Element) -> Element {
    let t = theme();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px;",
            div { style: "color: {t.ink_2}; font-size: 12px; font-weight: 600; padding-left: 2px;", "{title}" }
            div { style: "display: flex; flex-direction: column; border-radius: 10px; background: {t.sheet}; \
                          border: 1px solid {t.rule}; overflow: hidden;",
                {children}
            }
        }
    }
}

#[component]
fn Row(
    title: String,
    detail: String,
    by: Option<LayerKind>,
    kind: LayerKind,
    first: bool,
    pad: f32,
    locked: bool,
    clear: EventHandler<()>,
    children: Element,
) -> Element {
    let t = theme();
    let rule = if first { "transparent" } else { &*t.rule };
    let (fade, reach) = if locked { ("0.5", "none") } else { ("1", "auto") };
    let said = if locked {
        "Set by your organization"
    } else {
        said_by(by)
    };
    rsx! {
        div { style: "display: flex; align-items: center; gap: 14px; padding: {pad}px 16px; border-top: 1px solid {rule};",
            div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px;",
                div { style: "display: flex; align-items: center; gap: 8px;",
                    span { style: "font-weight: 550; color: {t.ink};", "{title}" }
                    if by == Some(kind) && !locked {
                        span { style: "width: 6px; height: 6px; border-radius: 3px; background: {t.accent};", title: "Set here" }
                        ResetButton { label: format!("Reset {title}"), onpress: clear }
                    }
                }
                span { style: "font-size: 12px; line-height: 1.5; color: {t.ink_3};", "{detail}" }
                span { style: "font-size: 11px; color: {t.ink_3};", "{said}" }
            }
            div { style: "flex-shrink: 0; opacity: {fade}; pointer-events: {reach};", {children} }
        }
    }
}

/// A token's row: who sets it, and a reset that takes this layer's away.
#[component]
fn TokenRow(
    ctx: Ctx,
    store: ThemeStore,
    name: &'static str,
    title: &'static str,
    detail: &'static str,
    first: bool,
    said: (Option<LayerKind>, Option<String>),
    locked: bool,
    children: Element,
) -> Element {
    let kind = ctx.kind;
    let clear = EventHandler::new(move |()| {
        let _ = store.update(kind, |l| {
            l.overrides.remove(name);
            if name == tokens::FONT {
                l.overrides.remove(tokens::FONT_SANS);
            }
        });
    });
    rsx! {
        Row { title, detail, by: said.0, kind, first, pad: ctx.px(14.0), locked, clear, {children} }
    }
}

#[component]
fn ResetButton(label: String, onpress: EventHandler<()>) -> Element {
    let t = theme();
    rsx! {
        button {
            style: "width: 20px; height: 20px; border: none; border-radius: 5px; background: transparent; \
                    display: flex; align-items: center; justify-content: center; cursor: pointer;",
            title: "Back to what is under it",
            "aria-label": "{label}",
            onclick: move |_| onpress.call(()),
            RotateCcw { size: 12, color: t.ink_3.to_string() }
        }
    }
}

#[component]
fn Seg(label: String, on: bool, onpick: EventHandler<()>) -> Element {
    let t = theme();
    let (fill, ink) = if on {
        (&*t.pill, &*t.ink)
    } else {
        ("transparent", &*t.ink_3)
    };
    rsx! {
        button {
            style: "height: 24px; padding: 0 10px; border: none; border-radius: 6px; background: {fill}; color: {ink}; \
                    font-size: 12px; font-weight: 550; cursor: pointer;",
            "aria-pressed": "{on}",
            onclick: move |_| onpick.call(()),
            "{label}"
        }
    }
}

/// Labelled options, the one whose value is `now` pressed.
#[component]
fn Segmented(options: Vec<(String, String)>, now: String, onpick: EventHandler<String>) -> Element {
    let t = theme();
    let same = |a: &str, b: &str| match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() < 1e-6,
        _ => a.trim() == b.trim(),
    };
    rsx! {
        div { style: "display: flex; padding: 2px; gap: 2px; border-radius: 8px; background: {t.desk}; border: 1px solid {t.rule};",
            for (label, value) in options {
                Seg { key: "{label}", on: same(&value, &now), label, onpick: move |()| onpick.call(value.clone()) }
            }
        }
    }
}

/// Text size, 80 % to 140 % in steps of 5.
#[component]
fn TextScale(now: f32, onpick: EventHandler<String>) -> Element {
    let t = theme();
    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    let pct = (now * 100.0).round() as i32;
    let to = move |p: i32| {
        let p = p.clamp(80, 140);
        format!("{}", f64::from(p) / 100.0)
    };
    rsx! {
        div { style: "display: flex; align-items: center; height: 30px; border-radius: 8px; border: 1px solid {t.rule}; background: {t.desk};",
            button {
                style: "width: 30px; height: 28px; border: none; background: transparent; display: flex; align-items: center; justify-content: center; cursor: pointer;",
                "aria-label": "Smaller",
                disabled: pct <= 80,
                onclick: move |_| onpick.call(to(pct.saturating_sub(5))),
                Minus { size: 14, color: t.ink_2.to_string() }
            }
            span { style: "min-width: 56px; text-align: center; font-size: 13px; font-variant-numeric: tabular-nums; color: {t.ink};", "{pct} %" }
            button {
                style: "width: 30px; height: 28px; border: none; background: transparent; display: flex; align-items: center; justify-content: center; cursor: pointer;",
                "aria-label": "Larger",
                disabled: pct >= 140,
                onclick: move |_| onpick.call(to(pct.saturating_add(5))),
                Plus { size: 14, color: t.ink_2.to_string() }
            }
        }
    }
}

/// Light, dark or system, each drawn in the theme it would show.
#[component]
fn ModeCard(label: String, on: bool, swatch: Resolved, onpick: EventHandler<()>) -> Element {
    let t = theme();
    let edge = if on { &*t.accent } else { &*t.rule };
    let (desk, ink, brand) = (swatch.value("desk"), swatch.value("ink-3"), swatch.value("brand"));
    rsx! {
        button {
            style: "width: 104px; display: flex; flex-direction: column; gap: 6px; padding: 6px; border-radius: 10px; \
                    border: 1.5px solid {edge}; background: {t.desk}; text-align: left; cursor: pointer;",
            "aria-pressed": "{on}",
            onclick: move |_| onpick.call(()),
            div { style: "height: 40px; border-radius: 6px; background: {desk}; display: flex; flex-direction: column; gap: 4px; padding: 8px;",
                div { style: "width: 60%; height: 4px; border-radius: 2px; background: {ink};" }
                div { style: "width: 30%; height: 4px; border-radius: 2px; background: {brand};" }
            }
            span { style: "font-size: 12px; font-weight: 600; color: {t.ink}; padding: 0 2px 2px;", "{label}" }
        }
    }
}

/// A theme, drawn in its own colours: its desk, a sidebar, text, its
/// brand.
#[component]
fn ThemeCard(card: Card, disabled: bool, onpick: EventHandler<()>) -> Element {
    let t = theme();
    let edge = if card.on { &*t.accent } else { &*t.rule };
    let width = if card.on { "2px" } else { "1px" };
    let reach = if disabled { "none" } else { "auto" };
    rsx! {
        button {
            style: "width: 128px; display: flex; flex-direction: column; gap: 6px; padding: 5px; border-radius: 10px; \
                    border: {width} solid {edge}; background: {t.desk}; text-align: left; cursor: pointer; pointer-events: {reach};",
            "aria-pressed": "{card.on}",
            title: "{card.id}",
            onclick: move |_| onpick.call(()),
            div { style: "height: 52px; border-radius: 6px; overflow: hidden; display: flex; background: {card.desk}; border: 1px solid {card.rule};",
                div { style: "width: 30%; background: {card.sheet}; border-right: 1px solid {card.rule};" }
                div { style: "flex: 1; display: flex; flex-direction: column; gap: 5px; padding: 8px;",
                    div { style: "width: 80%; height: 4px; border-radius: 2px; background: {card.ink};" }
                    div { style: "width: 55%; height: 4px; border-radius: 2px; background: {card.ink_3};" }
                    div { style: "width: 28px; height: 8px; border-radius: 4px; background: {card.brand};" }
                }
            }
            span { style: "font-size: 12px; font-weight: 550; color: {t.ink}; padding: 0 2px 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                "{card.label}"
            }
        }
    }
}
