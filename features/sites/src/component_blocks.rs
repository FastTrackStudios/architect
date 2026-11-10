//! A WASI component's embed views, drawing its blocks at build time
//! (#488).
//!
//! In the app, an `[[embeds]] view` of a component is mounted as a
//! content-sized sub-document and given the block as its input (the
//! `architect-embed` event, `architect_extension_wasi::embed()` in the
//! guest). A site mounts it the same way, once per block, without a
//! window: the component runs in the extensions host's `wasi` runner (its
//! memory and time limited, the app reached only through the host
//! service, which here has nothing), the view's edits are applied to a
//! tree of [`Element`]s, and the tree is written as HTML by the same rules
//! as a page's ([`crate::elements_to_html`]).
//!
//! So a renderer written in Rust (Keyflow's engraver) draws a node's share
//! links and Sites without being linked into the node.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::contrib::{Claim, EmbedDecl, EmbedInput};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::manifest::Kind;
use architect_extensions::process::Processes;
use architect_extensions::wasi::WasiRunner;
use architect_extensions::wire::{EMBED_EVENT, Edit, EventData, Frame, Value, ViewEvent};
use architect_extensions::{Origin, Package};
use architect_pages::{Attr, Element};

use crate::blocks::{Block, BlockRenderer, Drawn};

/// How long a block's view has to draw it.
pub const DRAW_TIMEOUT: Duration = Duration::from_secs(10);

/// How many edits one block's drawing may take.
const MAX_EDITS: usize = 200_000;

/// A WASI component's embed views, as a [`BlockRenderer`] for the fences
/// they claim.
pub struct ComponentBlocks {
    id: String,
    /// Each embed drawn by a view that claims a fence.
    embeds: Vec<EmbedDecl>,
    processes: Processes,
    /// Kept: dropping it unloads the component.
    _host: Host,
    /// One block at a time: the component is one instance.
    drawing: Mutex<()>,
}

impl std::fmt::Debug for ComponentBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentBlocks")
            .field("id", &self.id)
            .field(
                "embeds",
                &self.embeds.iter().map(|e| &e.id).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl ComponentBlocks {
    /// The component extension whose package is `dir`, loaded, when it
    /// has an `[[embeds]] view` that claims a fence; `None` when it is not
    /// a component or draws no fence.
    ///
    /// # Errors
    ///
    /// Its package does not read, or its component does not load.
    pub fn load(dir: &Path) -> Result<Option<Self>, String> {
        let package = Package::read(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let manifest = &package.manifest;
        if manifest.kind != Kind::Wasm || manifest.component().is_none() {
            return Ok(None);
        }
        let embeds: Vec<EmbedDecl> = manifest
            .contributes
            .embeds
            .iter()
            .filter(|e| e.view.is_some() && !e.claims.fence.is_empty())
            .cloned()
            .collect();
        if embeds.is_empty() {
            return Ok(None);
        }
        let processes = Processes::default();
        let runner = WasiRunner::new().with_processes(processes.clone());
        let host = Host::new(
            ActionList::new(),
            SharedSections::new(architect_settings::Registry::new()),
        )
        .with_runner(runner);
        host.load(&package, Origin::Installed)
            .map_err(|e| format!("{}: {e}", manifest.id))?;
        Ok(Some(Self {
            id: manifest.id.clone(),
            embeds,
            processes,
            _host: host,
            drawing: Mutex::new(()),
        }))
    }

    /// The extension's id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The fences it draws.
    #[must_use]
    pub fn fences(&self) -> Vec<String> {
        self.embeds
            .iter()
            .flat_map(|e| e.claims.fence.iter().cloned())
            .collect()
    }
}

impl BlockRenderer for ComponentBlocks {
    fn name(&self) -> &str {
        &self.id
    }

    fn claims(&self, claim: &Claim) -> bool {
        self.embeds.iter().any(|e| e.claims(claim))
    }

    fn render(&self, block: &Block<'_>) -> Result<Drawn, String> {
        let embed = self
            .embeds
            .iter()
            .find(|e| e.claims(block.claim))
            .ok_or_else(|| format!("nothing of {} claims {}", self.id, block.claim))?;
        let view = embed.view.as_deref().unwrap_or_default();
        let process = self
            .processes
            .get(&self.id)
            .ok_or_else(|| format!("{} is not running", self.id))?;
        let _one = self
            .drawing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut session = process.open(view)?;
        let mut tree = Tree::new();
        let deadline = Instant::now() + DRAW_TIMEOUT;
        tree.apply(next(&mut session.frames, deadline)?)?;
        let input = EmbedInput {
            claim: block.claim.to_string(),
            source: block.source.to_owned(),
            props: serde_json::json!({ "info": block.info, "page": block.page }),
        };
        session
            .events
            .unbounded_send(ViewEvent {
                element: 0,
                name: EMBED_EVENT.to_owned(),
                bubbles: false,
                data: EventData::Form(serde_json::to_string(&input).map_err(|e| e.to_string())?),
            })
            .map_err(|_| format!("{} stopped", self.id))?;
        // Every event is answered with a frame (wire 5), empty or not.
        tree.apply(next(&mut session.frames, deadline)?)?;
        Ok(Drawn::Elements(tree.into_elements()))
    }
}

/// The next frame, by `deadline`.
fn next(
    frames: &mut futures::channel::mpsc::UnboundedReceiver<Frame>,
    deadline: Instant,
) -> Result<Frame, String> {
    loop {
        match frames.try_recv() {
            Ok(frame) => return Ok(frame),
            Err(futures::channel::mpsc::TryRecvError::Closed) => {
                return Err("its view closed".to_owned());
            }
            Err(_) if Instant::now() >= deadline => {
                return Err(format!(
                    "it drew nothing in {} s",
                    DRAW_TIMEOUT.as_secs()
                ));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

/// A view's document without a window: the remote-view wire's edits
/// applied to plain nodes, as `architect_extensions::remote::RemoteDocument`
/// applies them to Blitz's.
struct Tree {
    nodes: Vec<Node>,
    stack: Vec<usize>,
    ids: HashMap<u64, usize>,
    edits: usize,
}

#[derive(Clone, Default)]
struct Node {
    /// `#text` for text.
    tag: String,
    text: String,
    attrs: Vec<(String, String)>,
    /// CSS properties set one by one (Dioxus's `style` namespace).
    style: Vec<(String, String)>,
    children: Vec<usize>,
    parent: Option<usize>,
}

/// The root: the view's element 0.
const ROOT: usize = 0;

impl Tree {
    fn new() -> Self {
        Self {
            nodes: vec![Node {
                tag: "#root".to_owned(),
                ..Node::default()
            }],
            stack: vec![ROOT],
            ids: HashMap::from([(0, ROOT)]),
            edits: 0,
        }
    }

    fn apply(&mut self, frame: Frame) -> Result<(), String> {
        let edits = match frame {
            Frame::Edits { edits } => edits,
            Frame::Failed(why) => return Err(why),
        };
        self.edits += edits.len();
        if self.edits > MAX_EDITS {
            return Err(format!("a drawing of over {MAX_EDITS} edits"));
        }
        edits.into_iter().try_for_each(|e| self.edit(e))
    }

    fn top(&self) -> Result<usize, String> {
        self.stack
            .last()
            .copied()
            .ok_or_else(|| "an edit on an empty stack".to_owned())
    }

    fn pop_one(&mut self) -> Result<usize, String> {
        if self.stack.len() <= 1 {
            return Err("an edit took more nodes than there are".to_owned());
        }
        self.top().inspect(|_| {
            self.stack.pop();
        })
    }

    fn pop(&mut self, m: u32) -> Result<Vec<usize>, String> {
        let m = usize::try_from(m).map_err(|e| e.to_string())?;
        let keep = self
            .stack
            .len()
            .checked_sub(m)
            .filter(|k| *k >= 1)
            .ok_or("an edit took more nodes than there are")?;
        Ok(self.stack.split_off(keep))
    }

    fn set_top(&mut self, node: usize) -> Result<(), String> {
        if self.stack.len() <= 1 {
            return Err("an edit reached outside the view".to_owned());
        }
        if let Some(top) = self.stack.last_mut() {
            *top = node;
        }
        Ok(())
    }

    fn add(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// `node` out of its parent's children.
    fn detach(&mut self, node: usize) {
        if let Some(parent) = self.nodes[node].parent.take() {
            self.nodes[parent].children.retain(|&c| c != node);
        }
    }

    /// Whether `node` is `inside` or below it.
    fn within(&self, mut node: usize, inside: usize) -> bool {
        loop {
            if node == inside {
                return true;
            }
            match self.nodes[node].parent {
                Some(p) => node = p,
                None => return false,
            }
        }
    }

    /// Puts `kids` under `parent` at `at` (its index among the children).
    fn insert(&mut self, parent: usize, at: usize, kids: &[usize]) -> Result<(), String> {
        if kids
            .iter()
            .any(|&k| self.within(parent, k) || k == ROOT)
        {
            return Err("an edit would put a node inside itself".to_owned());
        }
        for &kid in kids {
            self.detach(kid);
        }
        let at = at.min(self.nodes[parent].children.len());
        for (i, &kid) in kids.iter().enumerate() {
            self.nodes[kid].parent = Some(parent);
            self.nodes[parent].children.insert(at + i, kid);
        }
        Ok(())
    }

    fn place(&self, anchor: usize) -> Result<(usize, usize), String> {
        let parent = self.nodes[anchor]
            .parent
            .filter(|_| anchor != ROOT)
            .ok_or("an edit reached outside the view")?;
        let index = self.nodes[parent]
            .children
            .iter()
            .position(|&c| c == anchor)
            .ok_or("a node not among its parent's children")?;
        Ok((parent, index))
    }

    fn copy(&mut self, node: usize) -> usize {
        let mut fresh = self.nodes[node].clone();
        fresh.parent = None;
        let kids = std::mem::take(&mut fresh.children);
        let at = self.add(fresh);
        for kid in kids {
            let c = self.copy(kid);
            self.nodes[c].parent = Some(at);
            self.nodes[at].children.push(c);
        }
        at
    }

    fn drop_node(&mut self, node: usize) -> Result<(), String> {
        if self.within(ROOT, node) {
            return Err("an edit reached outside the view".to_owned());
        }
        self.detach(node);
        // Its ids, and those of everything below it, name nothing now.
        let gone: Vec<u64> = self
            .ids
            .iter()
            .filter(|&(_, &n)| n != ROOT && self.within(n, node))
            .map(|(&id, _)| id)
            .collect();
        for id in gone {
            self.ids.remove(&id);
        }
        Ok(())
    }

    fn edit(&mut self, edit: Edit) -> Result<(), String> {
        match edit {
            Edit::PushId { id } => {
                let node = *self.ids.get(&id).ok_or_else(|| format!("no element {id}"))?;
                self.stack.push(node);
            }
            Edit::SetId { id } => {
                let node = self.top()?;
                if node == ROOT && id != 0 {
                    return Err("an edit reached outside the view".to_owned());
                }
                self.ids.insert(id, node);
            }
            Edit::Child { index } => {
                let node = self.top()?;
                let child = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.nodes[node].children.get(i).copied())
                    .ok_or("a child that is not there")?;
                self.set_top(child)?;
            }
            Edit::Pop => {
                self.pop_one()?;
            }
            Edit::CreateElement { tag, .. } => {
                let node = self.add(Node {
                    tag,
                    ..Node::default()
                });
                self.stack.push(node);
            }
            Edit::CreateText { value } => {
                let node = self.add(Node {
                    tag: "#text".to_owned(),
                    text: value,
                    ..Node::default()
                });
                self.stack.push(node);
            }
            Edit::Clone => {
                let node = self.top()?;
                let copy = self.copy(node);
                self.set_top(copy)?;
            }
            Edit::AppendChildren { m } => {
                let kids = self.pop(m)?;
                let parent = self.top()?;
                let end = self.nodes[parent].children.len();
                self.insert(parent, end, &kids)?;
            }
            Edit::ReplaceWith { m } => {
                let kids = self.pop(m)?;
                let anchor = self.pop_one()?;
                let (parent, at) = self.place(anchor)?;
                self.insert(parent, at, &kids)?;
                self.drop_node(anchor)?;
            }
            Edit::InsertAfter { m } | Edit::InsertBefore { m } => {
                let after = matches!(edit, Edit::InsertAfter { .. });
                let kids = self.pop(m)?;
                let anchor = self.top()?;
                let (parent, at) = self.place(anchor)?;
                self.insert(parent, if after { at + 1 } else { at }, &kids)?;
            }
            Edit::SetAttribute {
                name,
                namespace,
                value,
            } => {
                let node = self.top()?;
                let value = match value {
                    Value::None => None,
                    Value::Bool(false) if name == "checked" => None,
                    Value::Text(t) => Some(t),
                    Value::Float(f) => Some(f.to_string()),
                    Value::Int(i) => Some(i.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                };
                let n = &mut self.nodes[node];
                let list = if namespace.as_deref() == Some("style") {
                    &mut n.style
                } else {
                    &mut n.attrs
                };
                list.retain(|(k, _)| *k != name);
                if let Some(value) = value {
                    list.push((name, value));
                }
            }
            Edit::SetText { value } => {
                let node = self.top()?;
                self.nodes[node].text = value;
            }
            // Nothing listens here: the drawing is written once.
            Edit::Listen { .. } | Edit::Unlisten { .. } => {}
            Edit::Remove => {
                let node = self.pop_one()?;
                self.drop_node(node)?;
            }
        }
        Ok(())
    }

    /// The root's children, as a page's element tree.
    fn into_elements(self) -> Vec<Element> {
        self.nodes[ROOT]
            .children
            .iter()
            .map(|&c| self.element(c))
            .collect()
    }

    fn element(&self, at: usize) -> Element {
        let n = &self.nodes[at];
        if n.tag == "#text" {
            return Element {
                tag: "#text".to_owned(),
                text: Some(n.text.clone()),
                ..Element::default()
            };
        }
        let mut attrs: Vec<Attr> = n
            .attrs
            .iter()
            .filter(|(k, _)| k != "style" || n.style.is_empty())
            .map(|(name, value)| Attr {
                name: name.clone(),
                value: value.clone(),
            })
            .collect();
        if !n.style.is_empty() {
            let mut css: String = n
                .attrs
                .iter()
                .find(|(k, _)| k == "style")
                .map(|(_, v)| format!("{}; ", v.trim_end_matches(';')))
                .unwrap_or_default();
            for (k, v) in &n.style {
                css.push_str(&format!("{k}: {v}; "));
            }
            attrs.push(Attr {
                name: "style".to_owned(),
                value: css.trim_end().to_owned(),
            });
        }
        Element {
            tag: n.tag.clone(),
            attrs,
            text: (!n.text.is_empty()).then(|| n.text.clone()),
            children: n.children.iter().map(|&c| self.element(c)).collect(),
            ..Element::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements_to_html;

    fn text(t: &str) -> Value {
        Value::Text(t.to_owned())
    }

    /// The edits Dioxus writes for an SVG with two bars, then an update
    /// that replaces the second bar's time and removes a node: applied
    /// headlessly, written as the page's HTML.
    #[test]
    fn a_views_edits_become_its_drawing() {
        let mut tree = Tree::new();
        let attr = |name: &str, value: &str| Edit::SetAttribute {
            name: name.to_owned(),
            namespace: None,
            value: text(value),
        };
        let first = vec![
            Edit::CreateElement {
                tag: "svg".to_owned(),
                namespace: Some("http://www.w3.org/2000/svg".to_owned()),
            },
            attr("viewBox", "0 0 20 10"),
            attr("data-follow-playhead", ""),
            Edit::CreateElement {
                tag: "rect".to_owned(),
                namespace: None,
            },
            Edit::SetId { id: 3 },
            attr("data-at", "0"),
            Edit::AppendChildren { m: 1 },
            Edit::CreateElement {
                tag: "rect".to_owned(),
                namespace: None,
            },
            Edit::SetId { id: 4 },
            attr("data-at", "1"),
            Edit::SetAttribute {
                name: "fill".to_owned(),
                namespace: Some("style".to_owned()),
                value: text("red"),
            },
            Edit::Listen {
                name: "click".to_owned(),
            },
            Edit::AppendChildren { m: 1 },
            Edit::CreateText {
                value: "gone".to_owned(),
            },
            Edit::SetId { id: 5 },
            Edit::AppendChildren { m: 1 },
            Edit::AppendChildren { m: 1 },
        ];
        tree.apply(Frame::Edits { edits: first }).unwrap();
        let update = vec![
            Edit::PushId { id: 4 },
            attr("data-at", "2.5"),
            Edit::Pop,
            Edit::PushId { id: 5 },
            Edit::Remove,
            Edit::PushId { id: 3 },
            Edit::CreateText {
                value: "C".to_owned(),
            },
            Edit::InsertBefore { m: 1 },
            Edit::Pop,
        ];
        tree.apply(Frame::Edits { edits: update }).unwrap();
        assert_eq!(
            elements_to_html(&tree.into_elements()),
            "<svg viewBox=\"0 0 20 10\" data-follow-playhead=\"\">C<rect data-at=\"0\"></rect><rect data-at=\"2.5\" style=\"fill: red;\"></rect></svg>"
        );
    }

    #[test]
    fn edits_that_reach_outside_are_refused() {
        let mut tree = Tree::new();
        assert!(tree.apply(Frame::Edits { edits: vec![Edit::Pop] }).is_err());
        let mut tree = Tree::new();
        assert!(
            tree.apply(Frame::Edits {
                edits: vec![Edit::PushId { id: 0 }, Edit::Remove]
            })
            .is_err()
        );
        let mut tree = Tree::new();
        assert!(
            tree.apply(Frame::Failed("it trapped".to_owned()))
                .is_err()
        );
    }
}
