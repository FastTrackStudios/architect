//! Layer dependency planner — declare what each node *requires* and
//! *provides*, get back a valid build order or a precise diagnostic.
//!
//! Pure data (no vox, no async): a planning aid for wiring a multi-node
//! backend graph (`config → db → cache → service`). It topologically
//! sorts the nodes and reports cycles, conflicting providers, missing
//! providers, and duplicate ids — each with a fix suggestion — *before*
//! you build anything.
//!
//! ```ignore
//! let plan = LayerGraph::new()
//!     .add(LayerNode::new("config", [], ["config"]))
//!     .add(LayerNode::new("db", ["config"], ["db"]))
//!     .add(LayerNode::new("service", ["db"], ["service"]))
//!     .plan()?;
//! // plan.build_order == ["config", "db", "service"]
//! ```

use std::collections::HashMap;

/// One node: an id, the services it needs from others, and the services
/// it produces.
#[derive(Debug, Clone)]
pub struct LayerNode {
    pub id: String,
    pub requires: Vec<String>,
    pub provides: Vec<String>,
}

impl LayerNode {
    pub fn new<I, R, P>(id: I, requires: R, provides: P) -> Self
    where
        I: Into<String>,
        R: IntoIterator,
        R::Item: Into<String>,
        P: IntoIterator,
        P::Item: Into<String>,
    {
        Self {
            id: id.into(),
            requires: requires.into_iter().map(Into::into).collect(),
            provides: provides.into_iter().map(Into::into).collect(),
        }
    }
}

/// A graph of layer nodes awaiting a [`plan`](LayerGraph::plan).
#[derive(Debug, Clone, Default)]
pub struct LayerGraph {
    nodes: Vec<LayerNode>,
}

/// A validated build order — every node appears after the nodes that
/// provide what it requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerPlan {
    pub build_order: Vec<String>,
}

/// A human-facing explanation of a planning failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub suggestion: String,
}

/// Why a graph couldn't be planned. Each variant carries a
/// [`LayerDiagnostic`] via [`diagnostic`](LayerPlanError::diagnostic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerPlanError {
    DuplicateNode {
        id: String,
    },
    ConflictingProvider {
        service: String,
        first: String,
        second: String,
    },
    MissingProvider {
        node: String,
        service: String,
    },
    Cycle {
        nodes: Vec<String>,
    },
}

impl LayerPlanError {
    pub fn diagnostic(&self) -> LayerDiagnostic {
        match self {
            LayerPlanError::DuplicateNode { id } => LayerDiagnostic {
                code: "duplicate-node",
                message: format!("two nodes share the id `{id}`"),
                suggestion: "give each node a unique id".into(),
            },
            LayerPlanError::ConflictingProvider {
                service,
                first,
                second,
            } => LayerDiagnostic {
                code: "conflicting-provider",
                message: format!(
                    "service `{service}` is provided by both `{first}` and `{second}`"
                ),
                suggestion: "have exactly one node provide each service (or merge the two)".into(),
            },
            LayerPlanError::MissingProvider { node, service } => LayerDiagnostic {
                code: "missing-provider",
                message: format!("`{node}` requires `{service}`, but no node provides it"),
                suggestion: format!(
                    "add a node that provides `{service}` (or drop the requirement)"
                ),
            },
            LayerPlanError::Cycle { nodes } => LayerDiagnostic {
                code: "cycle",
                message: format!("dependency cycle among: {}", nodes.join(" → ")),
                suggestion: "break the cycle — one of these requirements must be removed".into(),
            },
        }
    }
}

impl core::fmt::Display for LayerPlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let d = self.diagnostic();
        write!(f, "[{}] {} — {}", d.code, d.message, d.suggestion)
    }
}

impl std::error::Error for LayerPlanError {}

impl LayerGraph {
    pub fn new() -> Self {
        Self::default()
    }

    // Builder-style append, not arithmetic — the `add` name reads well at
    // the call site (`.add(LayerNode::new(...))`).
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, node: LayerNode) -> Self {
        self.nodes.push(node);
        self
    }

    /// Topologically sort the nodes into a build order, or return the
    /// first problem found (duplicate id → conflicting provider →
    /// missing provider → cycle).
    pub fn plan(&self) -> Result<LayerPlan, LayerPlanError> {
        // 1. Unique ids.
        let mut seen = HashMap::new();
        for node in &self.nodes {
            if seen.insert(node.id.as_str(), ()).is_some() {
                return Err(LayerPlanError::DuplicateNode {
                    id: node.id.clone(),
                });
            }
        }

        // 2. Provider map: service → node id (no two providers).
        let mut provider: HashMap<&str, &str> = HashMap::new();
        for node in &self.nodes {
            for svc in &node.provides {
                if let Some(first) = provider.insert(svc.as_str(), node.id.as_str()) {
                    return Err(LayerPlanError::ConflictingProvider {
                        service: svc.clone(),
                        first: first.to_string(),
                        second: node.id.clone(),
                    });
                }
            }
        }

        // 3. Edges: a node depends on the provider of each required
        //    service. Missing provider is an error. Build indegrees.
        let mut indegree: HashMap<&str, usize> =
            self.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for node in &self.nodes {
            for req in &node.requires {
                let Some(&dep) = provider.get(req.as_str()) else {
                    return Err(LayerPlanError::MissingProvider {
                        node: node.id.clone(),
                        service: req.clone(),
                    });
                };
                if dep == node.id.as_str() {
                    continue; // self-provision is not a dependency
                }
                dependents.entry(dep).or_default().push(node.id.as_str());
                *indegree
                    .get_mut(node.id.as_str())
                    .expect("node in indegree") += 1;
            }
        }

        // 4. Kahn's algorithm. Process ready nodes in declared order for
        //    a deterministic plan.
        let mut order: Vec<&str> = self
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .filter(|id| indegree[id] == 0)
            .collect();
        let mut build_order = Vec::with_capacity(self.nodes.len());
        let mut idx = 0;
        while idx < order.len() {
            let id = order[idx];
            idx += 1;
            build_order.push(id.to_string());
            if let Some(deps) = dependents.get(id) {
                for &d in deps {
                    let e = indegree.get_mut(d).expect("dependent in indegree");
                    *e -= 1;
                    if *e == 0 {
                        order.push(d);
                    }
                }
            }
        }

        if build_order.len() != self.nodes.len() {
            let mut nodes: Vec<String> = self
                .nodes
                .iter()
                .map(|n| n.id.as_str())
                .filter(|id| indegree[id] > 0)
                .map(|s| s.to_string())
                .collect();
            nodes.sort();
            return Err(LayerPlanError::Cycle { nodes });
        }

        Ok(LayerPlan { build_order })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topological_order() {
        let plan = LayerGraph::new()
            .add(LayerNode::new("service", ["db", "cache"], ["service"]))
            .add(LayerNode::new("db", ["config"], ["db"]))
            .add(LayerNode::new("cache", ["config"], ["cache"]))
            .add(LayerNode::new("config", [] as [&str; 0], ["config"]))
            .plan()
            .expect("valid graph");
        // config before db/cache; db+cache before service
        let pos = |id: &str| plan.build_order.iter().position(|x| x == id).unwrap();
        assert!(pos("config") < pos("db"));
        assert!(pos("config") < pos("cache"));
        assert!(pos("db") < pos("service"));
        assert!(pos("cache") < pos("service"));
    }

    #[test]
    fn missing_provider() {
        let err = LayerGraph::new()
            .add(LayerNode::new("svc", ["db"], ["svc"]))
            .plan()
            .unwrap_err();
        assert_eq!(
            err,
            LayerPlanError::MissingProvider {
                node: "svc".into(),
                service: "db".into()
            }
        );
        assert_eq!(err.diagnostic().code, "missing-provider");
    }

    #[test]
    fn conflicting_provider() {
        let err = LayerGraph::new()
            .add(LayerNode::new("a", [] as [&str; 0], ["db"]))
            .add(LayerNode::new("b", [] as [&str; 0], ["db"]))
            .plan()
            .unwrap_err();
        assert!(matches!(err, LayerPlanError::ConflictingProvider { .. }));
        assert_eq!(err.diagnostic().code, "conflicting-provider");
    }

    #[test]
    fn duplicate_node() {
        let err = LayerGraph::new()
            .add(LayerNode::new("a", [] as [&str; 0], ["x"]))
            .add(LayerNode::new("a", [] as [&str; 0], ["y"]))
            .plan()
            .unwrap_err();
        assert_eq!(err, LayerPlanError::DuplicateNode { id: "a".into() });
    }

    #[test]
    fn cycle() {
        let err = LayerGraph::new()
            .add(LayerNode::new("a", ["b_out"], ["a_out"]))
            .add(LayerNode::new("b", ["a_out"], ["b_out"]))
            .plan()
            .unwrap_err();
        match err {
            LayerPlanError::Cycle { nodes } => {
                assert_eq!(nodes, vec!["a".to_string(), "b".to_string()])
            }
            other => panic!("expected cycle, got {other:?}"),
        }
    }
}
