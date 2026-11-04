//! Contribution points (feature `host`).
//!
//! What extensions declare (`[[points]]`), what others add to them (`[[contribute."<point>"]]`,
//! the `[[sections]]` sugar, the app's own items), each item checked
//! against its point's schema, and the set as its owner is told it.
//!
//! An item that does not meet the schema is left out and is a problem
//! (`APP_STATUS`, [`Points::problems`]), never a failed load: the rest of
//! the contributor loads. Items for a point nobody declares wait, unchecked,
//! until one does (the owner may load after its contributors).
//!
//! The order is the app's items first, then each contributor's in the
//! order it first contributed (a reload keeps its place).

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::interop::{APP, PointDecl, validate};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One item of a point, as its owner is told it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    /// Who contributed it: an extension's id, or `app`.
    pub from: String,
    pub value: Value,
}

/// An item left out: it does not meet its point's schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Rejected {
    pub point: String,
    pub from: String,
    /// Its place among `from`'s items for the point.
    pub index: usize,
    pub why: String,
}

/// A point declared, and who owns it.
#[derive(Clone, Debug, PartialEq)]
pub struct Declared {
    pub owner: String,
    pub decl: PointDecl,
    pub schema: Value,
}

#[derive(Default)]
struct State {
    declared: BTreeMap<String, Declared>,
    /// By contributor, in the order each first contributed: its items by
    /// point.
    given: Vec<(String, BTreeMap<String, Vec<Value>>)>,
}

/// The points and their items. Cheap to share inside the host.
#[derive(Default)]
pub struct Points(Mutex<State>);

impl std::fmt::Debug for Points {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.0);
        f.debug_struct("Points")
            .field("declared", &s.declared.keys().collect::<Vec<_>>())
            .field("contributors", &s.given.len())
            .finish()
    }
}

impl Points {
    /// `owner`'s points, in place of the ones it had: the points whose
    /// items' set or schema changed (to tell their owners).
    pub fn declare(&self, owner: &str, points: &[PointDecl]) -> Vec<String> {
        let mut s = lock(&self.0);
        let mut changed: Vec<String> = s
            .declared
            .iter()
            .filter(|(_, d)| d.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        s.declared.retain(|_, d| d.owner != owner);
        for p in points {
            // Another's point of the same name keeps it.
            if s.declared.contains_key(&p.id) {
                continue;
            }
            s.declared.insert(
                p.id.clone(),
                Declared {
                    owner: owner.to_owned(),
                    decl: p.clone(),
                    schema: p.schema_json(),
                },
            );
            changed.push(p.id.clone());
        }
        drop(s);
        changed.sort();
        changed.dedup();
        changed
    }

    /// `from`'s items, by point, in place of what it gave: the points
    /// whose set changed.
    pub fn put(&self, from: &str, items: BTreeMap<String, Vec<Value>>) -> Vec<String> {
        let mut s = lock(&self.0);
        let before = s
            .given
            .iter()
            .find(|(f, _)| f == from)
            .map(|(_, i)| i.clone())
            .unwrap_or_default();
        let mut changed: Vec<String> = before
            .keys()
            .chain(items.keys())
            .filter(|p| before.get(*p) != items.get(*p))
            .cloned()
            .collect();
        changed.sort();
        changed.dedup();
        match s.given.iter_mut().find(|(f, _)| f == from) {
            Some((_, kept)) => *kept = items,
            None => s.given.push((from.to_owned(), items)),
        }
        changed
    }

    /// Everything `id` declared and gave goes: the points whose set
    /// changed, and the ones it owned (their owner is gone).
    pub fn withdraw(&self, id: &str) -> Vec<String> {
        let mut s = lock(&self.0);
        let mut changed: Vec<String> = s
            .given
            .iter()
            .filter(|(f, _)| f == id)
            .flat_map(|(_, i)| i.keys().cloned())
            .collect();
        s.given.retain(|(f, _)| f != id);
        changed.extend(
            s.declared
                .iter()
                .filter(|(_, d)| d.owner == id)
                .map(|(p, _)| p.clone()),
        );
        s.declared.retain(|_, d| d.owner != id);
        drop(s);
        changed.sort();
        changed.dedup();
        changed
    }

    /// Point `point`, when one is declared.
    #[must_use]
    pub fn declared(&self, point: &str) -> Option<Declared> {
        lock(&self.0).declared.get(point).cloned()
    }

    /// Every point declared.
    #[must_use]
    pub fn all(&self) -> Vec<Declared> {
        lock(&self.0).declared.values().cloned().collect()
    }

    /// Point `point`'s items that meet its schema, in order (the app's
    /// first); none when it is not declared.
    #[must_use]
    pub fn items(&self, point: &str) -> Vec<Item> {
        let s = lock(&self.0);
        let Some(declared) = s.declared.get(point) else {
            return Vec::new();
        };
        ordered(&s.given)
            .flat_map(|(from, given)| {
                given
                    .get(point)
                    .into_iter()
                    .flatten()
                    .filter(|v| validate(&declared.schema, v).is_ok())
                    .map(move |v| Item {
                        from: from.clone(),
                        value: v.clone(),
                    })
            })
            .collect()
    }

    /// Every item left out, for a declared point: its point, who gave
    /// it, where, and why.
    #[must_use]
    pub fn rejected(&self) -> Vec<Rejected> {
        let s = lock(&self.0);
        let mut out = Vec::new();
        for (from, given) in ordered(&s.given) {
            for (point, items) in given {
                let Some(declared) = s.declared.get(point) else {
                    continue;
                };
                for (index, v) in items.iter().enumerate() {
                    if let Err(why) = validate(&declared.schema, v) {
                        out.push(Rejected {
                            point: point.clone(),
                            from: from.clone(),
                            index,
                            why,
                        });
                    }
                }
            }
        }
        drop(s);
        out
    }

    /// Items given for points nobody declares (yet): by point, who gave
    /// how many.
    #[must_use]
    pub fn waiting(&self) -> BTreeMap<String, Vec<(String, usize)>> {
        let s = lock(&self.0);
        let mut out: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
        for (from, given) in &s.given {
            for (point, items) in given {
                if !s.declared.contains_key(point) && !items.is_empty() {
                    out.entry(point.clone())
                        .or_default()
                        .push((from.clone(), items.len()));
                }
            }
        }
        out
    }

    /// What the problems list says of [`Self::rejected`].
    #[must_use]
    pub fn problems(&self) -> Vec<architect::action::Problem> {
        self.rejected()
            .into_iter()
            .map(|r| {
                architect::action::Problem::new(
                    "extension",
                    format!(
                        "{}: its item {} for {} was left out: {}",
                        r.from, r.index, r.point, r.why
                    ),
                )
                .field(format!("contribute.{}[{}]", r.point, r.index))
            })
            .collect()
    }
}

/// The contributors, the app first, then in the order they came.
fn ordered(
    given: &[(String, BTreeMap<String, Vec<Value>>)],
) -> impl Iterator<Item = &(String, BTreeMap<String, Vec<Value>>)> {
    given
        .iter()
        .filter(|(f, _)| f == APP)
        .chain(given.iter().filter(|(f, _)| f != APP))
}
