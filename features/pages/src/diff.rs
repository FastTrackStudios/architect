//! A page's element tree, drawn as the remote-view edit stream.
//!
//! [`Mirror`] keeps what was drawn last, each node under an element id of
//! its own; [`Mirror::update`] diffs a new tree against it and answers
//! with the [`Edit`]s that move the document from one to the other — a
//! process view's `RemoteDocument` applies them, so a page and a process
//! view are drawn the same way. Keyed children keep their nodes as a list
//! reorders; an attribute or a text that changed is set in place; a
//! handler that changed only its function makes no edit at all.
//!
//! An event names the element it happened on; [`Mirror::handler`] finds
//! the handler for it there or on an ancestor (the page bubbles it
//! through its own tree, as a Dioxus guest does).

use std::collections::HashMap;

use architect_extensions::wire::{Edit, Value};

use crate::api::{Attr, Element, Listener};

/// The tag of a text node.
pub const TEXT: &str = "#text";

/// One node as drawn.
#[derive(Clone, Debug)]
struct Node {
    id: u64,
    tag: String,
    key: Option<String>,
    text: Option<String>,
    attrs: Vec<Attr>,
    on: Vec<Listener>,
    children: Vec<Node>,
}

/// What a page drew last, to diff the next against. The root is the
/// view's own element (id 0); a page's top-level nodes are its children.
#[derive(Debug)]
pub struct Mirror {
    root: Vec<Node>,
    next: u64,
    /// Element id → its parent's (0 for the root's children).
    parents: HashMap<u64, u64>,
    /// Element id → its handlers.
    handlers: HashMap<u64, Vec<Listener>>,
}

impl Default for Mirror {
    fn default() -> Self {
        Self::new()
    }
}

impl Mirror {
    /// Nothing drawn yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: Vec::new(),
            next: 1,
            parents: HashMap::new(),
            handlers: HashMap::new(),
        }
    }

    /// The edits that draw `tree` (the page's top-level nodes) where the
    /// last was; empty when nothing changed.
    pub fn update(&mut self, tree: &[Element]) -> Vec<Edit> {
        let mut edits = Vec::new();
        let mut old = std::mem::take(&mut self.root);
        self.children(0, &mut old, tree, &mut edits);
        self.root = old;
        edits
    }

    /// How many nodes are drawn.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parents.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }

    /// The handler for `event` on element `id`, or the nearest ancestor
    /// that has one.
    #[must_use]
    pub fn handler(&self, id: u64, event: &str) -> Option<u32> {
        let mut at = id;
        loop {
            if let Some(found) = self
                .handlers
                .get(&at)
                .and_then(|on| on.iter().find(|l| l.event == event))
            {
                return Some(found.handler);
            }
            at = *self.parents.get(&at)?;
            if at == 0 {
                return None;
            }
        }
    }

    fn id(&mut self) -> u64 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }

    /// Makes `el` and everything below it, leaving it on the stack.
    fn create(&mut self, parent: u64, el: &Element, edits: &mut Vec<Edit>) -> Node {
        let id = self.id();
        self.parents.insert(id, parent);
        if el.tag == TEXT {
            edits.push(Edit::CreateText {
                value: el.text.clone().unwrap_or_default(),
            });
            edits.push(Edit::SetId { id });
            return Node {
                id,
                tag: TEXT.to_owned(),
                key: el.key.clone(),
                text: el.text.clone(),
                attrs: Vec::new(),
                on: Vec::new(),
                children: Vec::new(),
            };
        }
        edits.push(Edit::CreateElement {
            tag: el.tag.clone(),
            namespace: namespace(&el.tag),
        });
        edits.push(Edit::SetId { id });
        for a in &el.attrs {
            edits.push(set(a));
        }
        for l in distinct(&el.on) {
            edits.push(Edit::Listen {
                name: l.event.clone(),
            });
        }
        if !el.on.is_empty() {
            self.handlers.insert(id, el.on.clone());
        }
        let children: Vec<Node> = el
            .children
            .iter()
            .map(|c| self.create(id, c, edits))
            .collect();
        if !children.is_empty() {
            edits.push(Edit::AppendChildren {
                m: count(children.len()),
            });
        }
        Node {
            id,
            tag: el.tag.clone(),
            key: el.key.clone(),
            text: None,
            attrs: el.attrs.clone(),
            on: el.on.clone(),
            children,
        }
    }

    /// Forgets `node` and everything below it (its document nodes go
    /// with the edit that removes it).
    fn forget(&mut self, node: &Node) {
        self.parents.remove(&node.id);
        self.handlers.remove(&node.id);
        for c in &node.children {
            self.forget(c);
        }
    }

    /// Moves `old` to `new` in place, or replaces it when it is another
    /// kind of node.
    fn patch(&mut self, parent: u64, old: &mut Node, new: &Element, edits: &mut Vec<Edit>) {
        if old.tag != new.tag {
            edits.push(Edit::PushId { id: old.id });
            let made = self.create(parent, new, edits);
            edits.push(Edit::ReplaceWith { m: 1 });
            self.forget(old);
            *old = made;
            return;
        }
        if old.tag == TEXT {
            if old.text != new.text {
                edits.push(Edit::PushId { id: old.id });
                edits.push(Edit::SetText {
                    value: new.text.clone().unwrap_or_default(),
                });
                edits.push(Edit::Pop);
                old.text.clone_from(&new.text);
            }
            old.key.clone_from(&new.key);
            return;
        }
        let mut own = Vec::new();
        for a in &new.attrs {
            if !old.attrs.contains(a) {
                own.push(set(a));
            }
        }
        for a in &old.attrs {
            if !new.attrs.iter().any(|n| n.name == a.name) {
                own.push(Edit::SetAttribute {
                    name: a.name.clone(),
                    namespace: None,
                    value: Value::None,
                });
            }
        }
        let (was, now) = (distinct(&old.on), distinct(&new.on));
        for l in &now {
            if !was.iter().any(|w| w.event == l.event) {
                own.push(Edit::Listen {
                    name: l.event.clone(),
                });
            }
        }
        for l in &was {
            if !now.iter().any(|n| n.event == l.event) {
                own.push(Edit::Unlisten {
                    name: l.event.clone(),
                });
            }
        }
        if !own.is_empty() {
            edits.push(Edit::PushId { id: old.id });
            edits.append(&mut own);
            edits.push(Edit::Pop);
        }
        old.attrs.clone_from(&new.attrs);
        old.on.clone_from(&new.on);
        old.key.clone_from(&new.key);
        if new.on.is_empty() {
            self.handlers.remove(&old.id);
        } else {
            self.handlers.insert(old.id, new.on.clone());
        }
        let id = old.id;
        self.children(id, &mut old.children, &new.children, edits);
    }

    /// Moves `parent`'s children from `old` to `new`: keyed ones matched
    /// by key, the rest by place; what is left over removed, what is new
    /// made, and each put where `new` says.
    fn children(
        &mut self,
        parent: u64,
        old: &mut Vec<Node>,
        new: &[Element],
        edits: &mut Vec<Edit>,
    ) {
        let mut pool: Vec<Option<Node>> = std::mem::take(old).into_iter().map(Some).collect();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        for (i, n) in pool.iter().enumerate() {
            if let Some(k) = n.as_ref().and_then(|n| n.key.clone()) {
                by_key.entry(k).or_insert(i);
            }
        }
        // Each new child's match among the old, by key or by place.
        let matched: Vec<Option<usize>> = new
            .iter()
            .enumerate()
            .map(|(i, el)| match &el.key {
                Some(k) => by_key.remove(k),
                None => pool
                    .get(i)
                    .and_then(Option::as_ref)
                    .filter(|n| n.key.is_none())
                    .map(|_| i),
            })
            .collect();
        // What nothing matched goes.
        for (i, slot) in pool.iter_mut().enumerate() {
            if matched.contains(&Some(i)) {
                continue;
            }
            if let Some(gone) = slot.take() {
                edits.push(Edit::PushId { id: gone.id });
                edits.push(Edit::Remove);
                self.forget(&gone);
            }
        }
        // The document's order now, to place each child against.
        let mut order: Vec<u64> = pool.iter().flatten().map(|n| n.id).collect();
        let mut out = Vec::with_capacity(new.len());
        for (at, (el, found)) in new.iter().zip(&matched).enumerate() {
            let node = match found.and_then(|i| pool.get_mut(i)?.take()) {
                Some(mut node) => {
                    let was = node.id;
                    self.patch(parent, &mut node, el, edits);
                    // Replaced by another kind of node: in its place.
                    if let Some(slot) = order.iter_mut().find(|id| **id == was) {
                        *slot = node.id;
                    }
                    if order.get(at) != Some(&node.id) {
                        order.retain(|id| *id != node.id);
                        let anchor = order.get(at).copied();
                        edits.push(Edit::PushId {
                            id: anchor.unwrap_or(parent),
                        });
                        edits.push(Edit::PushId { id: node.id });
                        edits.push(into(anchor));
                        edits.push(Edit::Pop);
                        order.insert(at.min(order.len()), node.id);
                    }
                    node
                }
                None => {
                    let anchor = order.get(at).copied();
                    edits.push(Edit::PushId {
                        id: anchor.unwrap_or(parent),
                    });
                    let made = self.create(parent, el, edits);
                    edits.push(into(anchor));
                    edits.push(Edit::Pop);
                    order.insert(at.min(order.len()), made.id);
                    made
                }
            };
            out.push(node);
        }
        *old = out;
    }
}

/// Puts the node on top of the stack before `anchor` (on the stack below
/// it), or — none — last in the parent below it.
const fn into(anchor: Option<u64>) -> Edit {
    match anchor {
        Some(_) => Edit::InsertBefore { m: 1 },
        None => Edit::AppendChildren { m: 1 },
    }
}

/// An element's attribute, as an edit.
fn set(a: &Attr) -> Edit {
    Edit::SetAttribute {
        name: a.name.clone(),
        namespace: None,
        value: Value::Text(a.value.clone()),
    }
}

/// SVG's elements are made in its namespace.
fn namespace(tag: &str) -> Option<String> {
    matches!(
        tag,
        "svg" | "path" | "circle" | "rect" | "line" | "polyline" | "polygon" | "g" | "ellipse"
    )
    .then(|| "http://www.w3.org/2000/svg".to_owned())
}

/// One listener per event name: the document needs one `Listen` for it.
fn distinct(on: &[Listener]) -> Vec<&Listener> {
    let mut out: Vec<&Listener> = Vec::new();
    for l in on {
        if !out.iter().any(|o| o.event == l.event) {
            out.push(l);
        }
    }
    out
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
