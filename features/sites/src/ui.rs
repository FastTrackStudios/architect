//! The site's layout: pure components.
//!
//! Every component here is a function of its props alone: the built
//! [`Site`] (shared, read-only) and which page or tag it draws. No
//! signals, effects, hooks, handlers or I/O. That is Task's `ssg-ui` rule,
//! kept: a component that cannot disagree with itself renders the same on
//! the server and on a client, which is what pre-rendering a route and
//! hydrating it later needs (#325's islands). What a reader does in the
//! browser (search, the theme toggle, the phone menu) is
//! `assets/site.js`, over the finished markup.
//!
//! The sidebar follows the navigator's look (folders that open, notes
//! under them, the current note marked), with `<details>` so folders open
//! without a script. Class names are `site-` prefixed; `assets/site.css`
//! styles them with the theme's tokens.

use std::rc::Rc;

use dioxus::prelude::*;

use crate::site::{NavNode, Page, Site};

/// What every page links: the fingerprinted stylesheet, script and
/// search index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assets {
    pub css: String,
    pub js: String,
    pub search: String,
}

/// What a document shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// A page, by index into [`Site::pages`].
    Page(usize),
    /// Every tag.
    Tags,
    /// One tag's pages, by index into [`Site::tags`].
    Tag(usize),
    /// A path the site does not have.
    NotFound,
}

/// `view` of `site`, as a whole HTML document.
#[must_use]
pub fn render(site: &Rc<Site>, view: View, assets: &Rc<Assets>) -> String {
    let head = dioxus_ssr::render_element(rsx! {
        Head { site: site.clone(), view, assets: assets.clone() }
    });
    let body = dioxus_ssr::render_element(rsx! {
        Body { site: site.clone(), view }
    });
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>{head}</head>\n<body>{body}</body>\n</html>\n"
    )
}

/// Before first paint: the reader's saved theme, so a dark page never
/// flashes light. The page's one inline script; a Content-Security-Policy
/// allows it by [`THEME_BOOT_HASH`].
pub const THEME_BOOT: &str = "try{var t=localStorage.getItem('site-theme');if(t)document.documentElement.dataset.theme=t}catch(e){}";

/// [`THEME_BOOT`]'s hash as a CSP source (`script-src 'self'
/// 'sha256-…'`): the one inline script a site's page runs.
pub const THEME_BOOT_HASH: &str = "'sha256-deQKU2EOWf3ZNuWj0YOr9gSGJkEEn27H0aM2lBzWZlA='";

const ICON_MENU: &str =
    r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16"/></svg>"#;
const ICON_THEME: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>"#;
const ICON_SEARCH: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/></svg>"#;
const ICON_NOTE: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/></svg>"#;
const ICON_FOLDER: &str =
    r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h6l2 2h10v11H3z"/></svg>"#;
const ICON_MARK: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 3 20h18z"/><path d="M8 14h8"/></svg>"#;

/// A document's `<head>`: its title, description, theme and assets.
#[component]
pub fn Head(site: Rc<Site>, view: View, assets: Rc<Assets>) -> Element {
    let (title, description) = match view {
        View::Page(i) => site.pages.get(i).map_or_else(
            || (site.title.clone(), None),
            |p| {
                let title = if i == 0 {
                    site.title.clone()
                } else {
                    format!("{} · {}", p.title, site.title)
                };
                let description = p
                    .description
                    .clone()
                    .or_else(|| (i == 0).then(|| site.description.clone()).flatten());
                (title, description)
            },
        ),
        View::Tags => (format!("Tags · {}", site.title), None),
        View::Tag(t) => (
            format!(
                "#{} · {}",
                site.tags
                    .get(t)
                    .map(|(n, _)| n.as_str())
                    .unwrap_or_default(),
                site.title
            ),
            None,
        ),
        View::NotFound => (format!("Not found · {}", site.title), None),
    };
    rsx! {
        meta { charset: "utf-8" }
        meta { name: "viewport", content: "width=device-width, initial-scale=1" }
        meta { name: "color-scheme", content: "light dark" }
        title { "{title}" }
        if let Some(description) = description {
            meta { name: "description", content: "{description}" }
        }
        meta { name: "site-search", content: "{assets.search}" }
        meta { name: "site-base", content: "{site.base}" }
        script { dangerous_inner_html: THEME_BOOT }
        link { rel: "stylesheet", href: "{assets.css}" }
        script { src: "{assets.js}", defer: true }
        if let Some(live) = &site.live_reload {
            meta { name: "site-live", content: "{live}" }
        }
    }
}

/// A document's `<body>`: the header, the navigator, the view, and the
/// page's contents.
#[component]
pub fn Body(site: Rc<Site>, view: View) -> Element {
    let active = match view {
        View::Page(i) => Some(i),
        View::Tags | View::Tag(_) | View::NotFound => None,
    };
    rsx! {
        a { class: "site-skip", href: "#site-main", "Skip to content" }
        Header { site: site.clone() }
        div { class: "site-body",
            nav { id: "site-nav", class: "site-nav", aria_label: "Pages",
                Sidebar { site: site.clone(), active }
            }
            div { class: "site-scrim", "data-site-close": "" }
            main { id: "site-main", class: "site-main",
                match view {
                    View::Page(i) => rsx! { PageView { site: site.clone(), index: i } },
                    View::Tags => rsx! { TagIndex { site: site.clone() } },
                    View::Tag(t) => rsx! { TagPage { site: site.clone(), tag: t } },
                    View::NotFound => rsx! { NotFound { site: site.clone() } },
                }
                Footer { site: site.clone() }
            }
            if let Some(i) = active {
                Toc { site, index: i }
            }
        }
    }
}

/// The bar across the top: the menu (phone), the site, search, the theme.
#[component]
fn Header(site: Rc<Site>) -> Element {
    rsx! {
        header { class: "site-header",
            button {
                class: "site-icon-button site-menu",
                r#type: "button",
                aria_label: "Pages",
                aria_controls: "site-nav",
                aria_expanded: "false",
                dangerous_inner_html: ICON_MENU,
            }
            a { class: "site-brand", href: "{site.base}",
                span { class: "site-mark", dangerous_inner_html: ICON_MARK }
                span { "{site.title}" }
            }
            div { class: "site-search", role: "search",
                span { class: "site-search-icon", dangerous_inner_html: ICON_SEARCH }
                input {
                    id: "site-search",
                    r#type: "search",
                    placeholder: "Search",
                    autocomplete: "off",
                    spellcheck: "false",
                    aria_label: "Search the site",
                    aria_controls: "site-results",
                }
                kbd { class: "site-search-key", "/" }
                div { id: "site-results", class: "site-results", role: "listbox", hidden: true }
            }
            a { class: "site-header-link", href: "{site.base}tags/", "Tags" }
            button {
                class: "site-icon-button site-theme",
                r#type: "button",
                aria_label: "Switch between light and dark",
                dangerous_inner_html: ICON_THEME,
            }
        }
    }
}

/// The navigation tree: the home page's children, the current page's
/// folders open.
#[component]
fn Sidebar(site: Rc<Site>, active: Option<usize>) -> Element {
    let open: Vec<String> = active
        .and_then(|i| site.pages.get(i))
        .map(|p| p.crumbs.iter().map(|c| c.url.clone()).collect())
        .unwrap_or_default();
    let home = site.nav.page;
    rsx! {
        ul { class: "site-tree",
            li {
                a {
                    class: if active == Some(home) { "site-tree-row site-tree-home is-active" } else { "site-tree-row site-tree-home" },
                    href: "{site.base}",
                    span { class: "site-tree-icon", dangerous_inner_html: ICON_NOTE }
                    span { class: "site-tree-label", "Home" }
                }
            }
            for child in site.nav.children.iter() {
                Branch { key: "{child.page}", site: site.clone(), node: child.clone(), active, open: open.clone() }
            }
        }
    }
}

/// One node of the tree: a note, or a folder with what is in it.
#[component]
fn Branch(site: Rc<Site>, node: NavNode, active: Option<usize>, open: Vec<String>) -> Element {
    let Some(page) = site.pages.get(node.page) else {
        return rsx! {};
    };
    let is_active = active == Some(node.page);
    let row = if is_active {
        "site-tree-row is-active"
    } else {
        "site-tree-row"
    };
    if node.children.is_empty() {
        return rsx! {
            li {
                a { class: row, href: "{page.url}", aria_current: if is_active { "page" } else { "false" },
                    span { class: "site-tree-icon", dangerous_inner_html: if page.is_section { ICON_FOLDER } else { ICON_NOTE } }
                    span { class: "site-tree-label", "{page.title}" }
                }
            }
        };
    }
    let expanded = is_active || open.contains(&page.url);
    rsx! {
        li {
            details { class: "site-tree-folder", open: expanded,
                summary { class: row,
                    span { class: "site-tree-chevron" }
                    span { class: "site-tree-icon", dangerous_inner_html: ICON_FOLDER }
                    a { class: "site-tree-label", href: "{page.url}", aria_current: if is_active { "page" } else { "false" }, "{page.title}" }
                    span { class: "site-tree-count", "{node.children.len()}" }
                }
                ul {
                    for child in node.children.iter() {
                        Branch { key: "{child.page}", site: site.clone(), node: child.clone(), active, open: open.clone() }
                    }
                }
            }
        }
    }
}

/// A page: where it is, its title, its tags, its prose, what is in it (a
/// folder's), what links here, and the pages either side.
#[component]
fn PageView(site: Rc<Site>, index: usize) -> Element {
    let Some(page) = site.pages.get(index) else {
        return rsx! {};
    };
    let minutes = (page.words / 220).max(1);
    // A note whose description is its opening paragraph says it once.
    let lede = page.description.clone().filter(|d| {
        let opening = page
            .sections
            .first()
            .map(|s| s.text.as_str())
            .unwrap_or_default();
        !opening.starts_with(d.trim())
    });
    rsx! {
        article { class: "site-article",
            if !page.crumbs.is_empty() {
                nav { class: "site-crumbs", aria_label: "Breadcrumbs",
                    for (i, crumb) in page.crumbs.iter().enumerate() {
                        if i > 0 { span { class: "site-crumb-sep", "/" } }
                        a { href: "{crumb.url}", "{crumb.title}" }
                    }
                }
            }
            h1 { class: "site-title", "{page.title}" }
            if let Some(description) = lede {
                p { class: "site-lede", "{description}" }
            }
            div { class: "site-meta",
                if page.words > 0 { span { "{minutes} min read" } }
                for tag in page.tags.iter() {
                    a { key: "{tag}", class: "site-tag", href: "{site.tag_url(tag)}", "#{tag}" }
                }
            }
            div { class: "site-prose", dangerous_inner_html: "{page.html}" }
            if page.is_section && !page.children.is_empty() {
                Listing { site: site.clone(), pages: page.children.clone(), heading: "In this section" }
            }
            if !page.backlinks.is_empty() {
                Listing { site: site.clone(), pages: page.backlinks.clone(), heading: "Linked from" }
            }
            PrevNext { site: site.clone(), index }
        }
    }
}

/// A list of pages as cards: a title, a line about each.
#[component]
fn Listing(site: Rc<Site>, pages: Vec<usize>, heading: String) -> Element {
    rsx! {
        section { class: "site-listing",
            h2 { class: "site-listing-title", "{heading}" }
            ul {
                for i in pages.iter() {
                    if let Some(page) = site.pages.get(*i) {
                        li { key: "{i}",
                            a { href: "{page.url}",
                                span { class: "site-listing-name", "{page.title}" }
                                if let Some(description) = &page.description {
                                    span { class: "site-listing-line", "{description}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The pages before and after this one in reading order.
#[component]
fn PrevNext(site: Rc<Site>, index: usize) -> Element {
    let previous = site.previous(index).cloned();
    let next = site.next(index).cloned();
    rsx! {
        nav { class: "site-prevnext", aria_label: "Previous and next",
            if let Some(Page { url, title, .. }) = previous {
                a { class: "site-prev", href: "{url}", span { "Previous" } strong { "{title}" } }
            }
            if let Some(Page { url, title, .. }) = next {
                a { class: "site-next", href: "{url}", span { "Next" } strong { "{title}" } }
            }
        }
    }
}

/// The page's own contents: its second- and third-level headings.
#[component]
fn Toc(site: Rc<Site>, index: usize) -> Element {
    let headings: Vec<_> = site
        .pages
        .get(index)
        .map(|p| {
            p.headings
                .iter()
                .filter(|h| (2..=3).contains(&h.level))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if headings.len() < 2 {
        return rsx! { aside { class: "site-toc" } };
    }
    rsx! {
        aside { class: "site-toc", aria_label: "On this page",
            p { class: "site-toc-title", "On this page" }
            ul {
                for h in headings.iter() {
                    li { key: "{h.id}", class: if h.level == 3 { "is-sub" } else { "" },
                        a { href: "#{h.id}", "{h.text}" }
                    }
                }
            }
        }
    }
}

/// Every tag, with how many pages carry it.
#[component]
fn TagIndex(site: Rc<Site>) -> Element {
    rsx! {
        article { class: "site-article",
            h1 { class: "site-title", "Tags" }
            if site.tags.is_empty() {
                p { class: "site-lede", "No page has a tag yet." }
            }
            div { class: "site-tag-cloud",
                for (tag, pages) in site.tags.iter() {
                    a { key: "{tag}", class: "site-tag", href: "{site.tag_url(tag)}",
                        "#{tag}"
                        span { class: "site-tag-count", "{pages.len()}" }
                    }
                }
            }
        }
    }
}

/// One tag's pages.
#[component]
fn TagPage(site: Rc<Site>, tag: usize) -> Element {
    let Some((name, pages)) = site.tags.get(tag).cloned() else {
        return rsx! {};
    };
    rsx! {
        article { class: "site-article",
            nav { class: "site-crumbs", aria_label: "Breadcrumbs",
                a { href: "{site.base}tags/", "Tags" }
            }
            h1 { class: "site-title", "#{name}" }
            Listing { site: site.clone(), pages, heading: "Pages" }
        }
    }
}

/// A path the site does not have.
#[component]
fn NotFound(site: Rc<Site>) -> Element {
    rsx! {
        article { class: "site-article",
            h1 { class: "site-title", "Not found" }
            p { class: "site-lede", "There is no page here. Search for it, or start from the home page." }
            p { a { href: "{site.base}", "{site.title}" } }
        }
    }
}

/// The foot of every page.
#[component]
fn Footer(site: Rc<Site>) -> Element {
    rsx! {
        footer { class: "site-footer",
            span { "{site.title}" }
            span { "·" }
            span { "Published with architect" }
        }
    }
}
