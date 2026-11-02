//! Deterministic journey-map geometry (`layout.json`).
//!
//! Only **flow nodes** (roots, edge endpoints, terminal states — formats.md)
//! get journey columns. Layering is the **longest path** from the roots over
//! the DAG obtained by dropping back edges, so the journey extends as far
//! right as possible:
//!
//! * `lane`  — index of the node's view in the manifest (inventory) order
//! * `depth` — longest-path layer (roots = 0); a flow node unreachable from
//!   every root keeps layer 0 — `sbk check flows` gates that, the layout
//!   merely stays deterministic and panic-free
//! * `order` — row within the lane (nodes sorted by depth, then state id)
//!
//! Duplicate `(from, to)` edges merge into one route with the labels joined
//! by `" / "`; `kind = "self"` terminals are not edges (their actions land
//! in the node's `self_actions`); edges that close a cycle are marked
//! `back: true`. Classification: DFS from the roots with edges sorted — an
//! edge is back when its target is on the DFS path (a self-loop always is);
//! `to = "back"` resolves to the DFS parent of `from`. Data variants are
//! listed separately with `variant_of` and a stack index under their node.

use crate::model::{Cell, EdgeRoute, Flows, Layout, NodePos, TerminalKind, VariantPos};
use std::collections::{BTreeMap, BTreeSet};

/// Label separator for duplicate-merged edges.
pub const LABEL_JOIN: &str = " / ";

/// One authored edge with its concrete (never `"back"`) target.
struct Wire {
    from: String,
    to: String,
    label: Option<String>,
    /// `true` for authored `to = "back"` (resolved to the DFS parent).
    authored_back: bool,
}

pub fn layout(
    views_in_order: &[String],
    cells: &[Cell],
    flows: &Flows,
    titles: &BTreeMap<String, String>,
    view_titles: &BTreeMap<String, String>,
) -> Layout {
    let lane_of = |state: &str| -> u32 {
        cells
            .iter()
            .find(|c| c.state_id == state)
            .and_then(|c| views_in_order.iter().position(|v| v == &c.view))
            .map_or(u32::MAX, |i| i as u32)
    };

    // Flow nodes: known cells named by roots, edge endpoints or terminals.
    let known = |state: &str| cells.iter().any(|c| c.state_id == state);
    let mut flow: BTreeSet<String> = BTreeSet::new();
    for root in &flows.roots {
        if known(root) {
            flow.insert(root.clone());
        }
    }
    for edge in &flows.edges {
        if known(&edge.from) {
            flow.insert(edge.from.clone());
        }
        if edge.to != "back" && known(&edge.to) {
            flow.insert(edge.to.clone());
        }
    }
    for terminal in &flows.terminals {
        if known(&terminal.state) {
            flow.insert(terminal.state.clone());
        }
    }

    let wires = wires(flows);
    let Classified {
        parents,
        back_pairs,
        finish,
    } = classify(&flow, &wires, &flows.roots);

    // Longest-path layering over the DAG (back edges dropped), processing
    // nodes in reverse DFS finish order — a topological order of the DAG.
    let mut depths: BTreeMap<&str, u32> = flow.iter().map(|s| (s.as_str(), 0)).collect();
    for node in finish.iter().rev() {
        let d = depths[node.as_str()];
        for wire in &wires {
            if wire.from == *node
                && !back_pairs.contains(&(wire.from.clone(), wire.to.clone()))
                && flow.contains(&wire.to)
                && let Some(next) = depths.get_mut(wire.to.as_str())
                && *next < d + 1
            {
                *next = d + 1;
            }
        }
    }

    // Rows within a lane: nodes sorted by (depth, state id).
    let mut ordered: Vec<&String> = flow.iter().collect();
    ordered.sort_by(|a, b| (depths[a.as_str()], a.as_str()).cmp(&(depths[b.as_str()], b.as_str())));
    let mut rows: BTreeMap<&str, u32> = BTreeMap::new();
    let mut row_of_lane: BTreeMap<u32, u32> = BTreeMap::new();
    for state in &ordered {
        let lane = lane_of(state);
        let counter = row_of_lane.entry(lane).or_insert(0);
        rows.insert(state.as_str(), *counter);
        *counter += 1;
    }

    let nodes = ordered
        .iter()
        .map(|state| {
            let id = state.as_str();
            let mut self_actions: Vec<String> = Vec::new();
            for terminal in &flows.terminals {
                if terminal.state == **state
                    && terminal.kind == TerminalKind::Self_
                    && !self_actions.contains(&terminal.action)
                {
                    self_actions.push(terminal.action.clone());
                }
            }
            let variants: Vec<String> = flows
                .variants
                .get(id)
                .map(|list| {
                    list.iter()
                        .filter(|v| known(v) && !flow.contains(*v))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            NodePos {
                state_id: id.to_string(),
                lane: lane_of(id),
                depth: depths[id],
                order: rows[id],
                title: titles.get(id).cloned(),
                self_actions,
                variant_count: variants.len() as u32,
                variants,
                rect: None,
            }
        })
        .collect();

    let variants = variant_positions(flows, &flow, known, &depths, &lane_of, &rows, titles);

    Layout {
        nodes,
        variants,
        edges: merge_edges(&wires, &flow, &back_pairs, &parents),
        engine: crate::model::ENGINE_BUILTIN.to_string(),
        view_titles: views_in_order
            .iter()
            .filter_map(|v| view_titles.get(v).map(|t| (v.clone(), t.clone())))
            .collect(),
        dot_version: None,
        clusters: Vec::new(),
    }
}

/// Authored edges with concrete targets; `to = "back"` points at `from`
/// until [`classify`] supplies the DFS parent.
fn wires(flows: &Flows) -> Vec<Wire> {
    flows
        .edges
        .iter()
        .map(|e| Wire {
            from: e.from.clone(),
            to: if e.to == "back" {
                e.from.clone()
            } else {
                e.to.clone()
            },
            label: e.label.clone(),
            authored_back: e.to == "back",
        })
        .collect()
}

/// DFS from the sorted roots over sorted adjacency: DFS-tree parents, the
/// `(from, to)` pairs that close a cycle (target on the DFS path), and the
/// finish (post-order) list.
/// DFS-tree parents, cycle-closing `(from, to)` pairs and finish order.
struct Classified {
    parents: BTreeMap<String, String>,
    back_pairs: BTreeSet<(String, String)>,
    finish: Vec<String>,
}

fn classify(flow: &BTreeSet<String>, wires: &[Wire], roots: &[String]) -> Classified {
    // Sorted unique adjacency (from, to) over edges between flow nodes.
    let mut adjacency: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for wire in wires {
        if flow.contains(&wire.from) && flow.contains(&wire.to) && wire.from != wire.to {
            adjacency
                .entry(wire.from.as_str())
                .or_default()
                .insert(wire.to.as_str());
        }
    }

    let mut parents: BTreeMap<String, String> = BTreeMap::new();
    let mut back_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    // Self-loops close a cycle whichever way the DFS runs.
    for wire in wires {
        if wire.from == wire.to {
            back_pairs.insert((wire.from.clone(), wire.to.clone()));
        }
    }

    let mut on_path: BTreeSet<&str> = BTreeSet::new();
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    let mut finish: Vec<String> = Vec::new();
    for root in roots {
        if flow.contains(root) && !visited.contains(root.as_str()) {
            dfs(
                root,
                &adjacency,
                &mut on_path,
                &mut visited,
                &mut parents,
                &mut back_pairs,
                &mut finish,
            );
        }
    }
    Classified {
        parents,
        back_pairs,
        finish,
    }
}

/// Recursive DFS with the frame state of [`classify`].
fn dfs<'a>(
    node: &'a str,
    adjacency: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    on_path: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
    parents: &mut BTreeMap<String, String>,
    back_pairs: &mut BTreeSet<(String, String)>,
    finish: &mut Vec<String>,
) {
    visited.insert(node);
    on_path.insert(node);
    if let Some(nexts) = adjacency.get(node) {
        for next in nexts {
            if !visited.contains(*next) {
                parents.insert((*next).to_string(), node.to_string());
                dfs(
                    next, adjacency, on_path, visited, parents, back_pairs, finish,
                );
            } else if on_path.contains(*next) {
                back_pairs.insert((node.to_string(), (*next).to_string()));
            }
        }
    }
    on_path.remove(node);
    finish.push(node.to_string());
}

/// Duplicate-merged routes: one per `(from, to)` pair between flow nodes,
/// labels joined by [`LABEL_JOIN`] (unique, edge order), `back` per the DFS
/// classification; authored `back` edges resolve to the DFS parent.
fn merge_edges(
    wires: &[Wire],
    flow: &BTreeSet<String>,
    back_pairs: &BTreeSet<(String, String)>,
    parents: &BTreeMap<String, String>,
) -> Vec<EdgeRoute> {
    let mut groups: BTreeMap<(String, String), Vec<&Wire>> = BTreeMap::new();
    for wire in wires {
        let to = if wire.authored_back {
            parents
                .get(&wire.from)
                .cloned()
                .unwrap_or_else(|| wire.from.clone())
        } else {
            wire.to.clone()
        };
        if flow.contains(&wire.from) && flow.contains(&to) {
            groups
                .entry((wire.from.clone(), to))
                .or_default()
                .push(wire);
        }
    }
    groups
        .into_iter()
        .map(|((from, to), group)| {
            let authored_back = group.iter().any(|w| w.authored_back);
            let mut labels: Vec<&str> = Vec::new();
            for wire in &group {
                if let Some(label) = wire.label.as_deref()
                    && !labels.contains(&label)
                {
                    labels.push(label);
                }
            }
            EdgeRoute {
                back: authored_back || back_pairs.contains(&(from.clone(), to.clone())),
                from,
                to,
                label: if labels.is_empty() {
                    None
                } else {
                    Some(labels.join(LABEL_JOIN))
                },
                points: None,
                label_pos: None,
                arrow: None,
            }
        })
        .collect()
}

/// Data variants of flow nodes, in (lane, node row, stack index) order.
fn variant_positions(
    flows: &Flows,
    flow: &BTreeSet<String>,
    known: impl Fn(&str) -> bool,
    depths: &BTreeMap<&str, u32>,
    lane_of: &dyn Fn(&str) -> u32,
    rows: &BTreeMap<&str, u32>,
    titles: &BTreeMap<String, String>,
) -> Vec<VariantPos> {
    let mut nodes: Vec<&String> = flows.variants.keys().collect();
    nodes.sort_by_key(|node| (lane_of(node), depths[node.as_str()], rows[node.as_str()]));
    nodes
        .into_iter()
        .filter(|node| flow.contains(*node))
        .flat_map(|node| {
            flows
                .variants
                .get(node.as_str())
                .map_or(Vec::new(), |list| {
                    list.iter()
                        .filter(|v| known(v) && !flow.contains(*v))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .into_iter()
                .enumerate()
                .map(move |(i, state_id)| VariantPos {
                    title: titles.get(&state_id).cloned(),
                    state_id,
                    variant_of: node.clone(),
                    lane: lane_of(node),
                    depth: depths[node.as_str()],
                    order: i as u32,
                })
        })
        .collect()
}
