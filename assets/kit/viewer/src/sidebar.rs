//! Pure sidebar model (references/canvas.md "Story list"): the drill-down
//! structure the GTK sidebar renders — one group per view, its flow nodes
//! in journey order, each node carrying its data variants. Search and
//! programmatic location are plain functions so the GUI layer stays thin
//! and headless tests cover the behaviour.

use std::collections::BTreeMap;

use storybook_kit::model::Layout;

use crate::story_list::story_title;

/// One view level of the sidebar: the level-1 row and the level-2 page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewGroup {
    pub id: String,
    /// Display name: authored `states/<view>.toml` `title`, else
    /// [`humanize`] of the id.
    pub title: String,
    /// States on the page: nodes + their variants.
    pub count: usize,
    pub nodes: Vec<NodeGroup>,
}

/// One flow node of a view page, with its data variants behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeGroup {
    pub state_id: String,
    /// Authored `layout.json` title, else [`story_title`].
    pub title: String,
    /// `(state_id, title)` of the node's variants in stack order;
    /// titles fall back to [`story_title`] like the node's.
    pub variants: Vec<(String, String)>,
}

/// `"skill_detail"` -> `"Skill detail"`: separators become spaces, the
/// first letter is capitalized. Fallback display name for views.
pub fn humanize(id: &str) -> String {
    let spaced = id.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Build the sidebar model: views in inventory order, each view's flow
/// nodes in journey order (lane/depth/order), variants attached to their
/// node in stack order.
pub fn build_model(views: &[String], layout: &Layout) -> Vec<ViewGroup> {
    let mut nodes_by_lane: Vec<Vec<NodeGroup>> = vec![Vec::new(); views.len()];
    let mut ordered: Vec<&storybook_kit::model::NodePos> = layout.nodes.iter().collect();
    ordered.sort_by_key(|n| (n.lane, n.depth, n.order));
    for node in ordered {
        let title = node
            .title
            .clone()
            .unwrap_or_else(|| story_title(&node.state_id));
        let mut variants: Vec<&storybook_kit::model::VariantPos> = layout
            .variants
            .iter()
            .filter(|v| v.variant_of == node.state_id)
            .collect();
        variants.sort_by_key(|v| v.order);
        let variants = variants
            .into_iter()
            .map(|v| {
                (
                    v.state_id.clone(),
                    v.title.clone().unwrap_or_else(|| story_title(&v.state_id)),
                )
            })
            .collect();
        let group = NodeGroup {
            state_id: node.state_id.clone(),
            title,
            variants,
        };
        if let Some(lane) = nodes_by_lane.get_mut(node.lane as usize) {
            lane.push(group);
        }
    }
    views
        .iter()
        .enumerate()
        .map(|(lane, id)| {
            let nodes = nodes_by_lane.get(lane).cloned().unwrap_or_default();
            ViewGroup {
                id: id.clone(),
                title: layout
                    .view_titles
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| humanize(id)),
                count: nodes.iter().map(|n| 1 + n.variants.len()).sum(),
                nodes,
            }
        })
        .collect()
}

/// Every state whose title, view name (display name or id) or state id
/// contains `query`, case-insensitively, in model order. An empty query
/// matches nothing (the sidebar shows the pages instead).
pub fn search(model: &[ViewGroup], query: &str) -> Vec<String> {
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for view in model {
        let view_names = [view.title.to_lowercase(), view.id.to_lowercase()];
        let mut push = |state_id: &str, title: &str| {
            let title = title.to_lowercase();
            let id = state_id.to_lowercase();
            if title.contains(&needle)
                || id.contains(&needle)
                || view_names.iter().any(|n| n.contains(&needle))
            {
                hits.push(state_id.to_string());
            }
        };
        for node in &view.nodes {
            push(&node.state_id, &node.title);
            for (state_id, title) in &node.variants {
                push(state_id, title);
            }
        }
    }
    hits
}

/// `(view index, flat row index)` of a state for programmatic selection:
/// the row is counted over the view page's rows — each node one row,
/// directly followed by its variant rows. `None` for an unknown id.
pub fn locate(model: &[ViewGroup], state_id: &str) -> Option<(usize, usize)> {
    for (view_idx, view) in model.iter().enumerate() {
        let mut row = 0;
        for node in &view.nodes {
            if node.state_id == state_id {
                return Some((view_idx, row));
            }
            row += 1;
            for (variant, _) in &node.variants {
                if variant == state_id {
                    return Some((view_idx, row));
                }
                row += 1;
            }
        }
    }
    None
}

// --- drill-down pages: views -> flow nodes -> data dimensions -> states ----

/// Data facts the dimension tree is built from.
#[derive(Debug, Clone, Default)]
pub struct Facets {
    /// view id -> its axes in order, each `(axis path, class order)`
    /// (`inventory.json`).
    pub axes: BTreeMap<String, Vec<(String, Vec<String>)>>,
    /// state id -> axis path -> class (`cells.json` assignments).
    pub assignment: BTreeMap<String, BTreeMap<String, String>>,
    /// view id -> label key -> human label (`states/<view>.toml`
    /// `[labels]`): key `"<axis>"` names a dimension, `"<axis>.<class>"`
    /// one of its values.
    pub labels: BTreeMap<String, BTreeMap<String, String>>,
}

/// What activating a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Push page `index` (a further level).
    Page(usize),
    /// Show this state (a leaf).
    State(String),
}

/// One row of a drill-down page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSpec {
    pub title: String,
    pub subtitle: Option<String>,
    pub target: Target,
}

/// One drill-down page; page 0 is the root ("Widoki").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSpec {
    pub title: String,
    pub parent: Option<usize>,
    pub rows: Vec<RowSpec>,
}

/// Human name of a dimension: authored label, else the humanized axis path.
pub fn axis_label(facets: &Facets, view: &str, axis: &str) -> String {
    facets
        .labels
        .get(view)
        .and_then(|l| l.get(axis))
        .cloned()
        .unwrap_or_else(|| humanize(axis))
}

/// Human name of a dimension value: authored label, else the class id.
pub fn class_label(facets: &Facets, view: &str, axis: &str, class: &str) -> String {
    facets
        .labels
        .get(view)
        .and_then(|l| l.get(&format!("{axis}.{class}")))
        .cloned()
        .unwrap_or_else(|| class.replace(['-', '_'], " "))
}

/// Build every drill-down page. Root lists the views; a view page lists
/// its flow nodes in journey order (a node without variants is a leaf);
/// a node page splits the node and its variants by the first dimension
/// (axis order) whose value differs among them, one row per value in class
/// order — a value held by one state is a leaf ("Zasięg: globalny"), by
/// several a further page split the same way. Dimensions constant within
/// a group never get a level.
pub fn pages(model: &[ViewGroup], facets: &Facets) -> Vec<PageSpec> {
    let mut out = vec![PageSpec {
        title: "Widoki".to_string(),
        parent: None,
        rows: Vec::new(),
    }];
    for view in model {
        let view_page = out.len();
        out.push(PageSpec {
            title: view.title.clone(),
            parent: Some(0),
            rows: Vec::new(),
        });
        out[0].rows.push(RowSpec {
            title: view.title.clone(),
            subtitle: Some(format!("Liczba stanów: {}", view.count)),
            target: Target::Page(view_page),
        });
        for node in &view.nodes {
            if node.variants.is_empty() {
                out[view_page].rows.push(RowSpec {
                    title: node.title.clone(),
                    subtitle: None,
                    target: Target::State(node.state_id.clone()),
                });
                continue;
            }
            let mut states = vec![(node.state_id.clone(), node.title.clone())];
            states.extend(node.variants.iter().cloned());
            let node_page = out.len();
            out.push(PageSpec {
                title: node.title.clone(),
                parent: Some(view_page),
                rows: Vec::new(),
            });
            out[view_page].rows.push(RowSpec {
                title: node.title.clone(),
                subtitle: Some(format!("Liczba stanów: {}", states.len())),
                target: Target::Page(node_page),
            });
            split(&mut out, node_page, &view.id, &states, facets);
        }
    }
    out
}

/// Fill page `page` with `states` split by their first differing dimension.
fn split(
    out: &mut Vec<PageSpec>,
    page: usize,
    view: &str,
    states: &[(String, String)],
    facets: &Facets,
) {
    let value = |state: &str, axis: &str| -> String {
        facets
            .assignment
            .get(state)
            .and_then(|a| a.get(axis))
            .cloned()
            .unwrap_or_else(|| "?".to_string())
    };
    let axes = facets.axes.get(view).cloned().unwrap_or_default();
    let differing = axes.iter().find(|(axis, _)| {
        let first = value(&states[0].0, axis);
        states.iter().any(|(s, _)| value(s, axis) != first)
    });
    let Some((axis, class_order)) = differing else {
        // nothing tells them apart: plain leaves
        for (state, title) in states {
            out[page].rows.push(RowSpec {
                title: title.clone(),
                subtitle: None,
                target: Target::State(state.clone()),
            });
        }
        return;
    };
    // group by value: class order first, unknown values after, in order seen
    let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (state, title) in states {
        let v = value(state, axis);
        match groups.iter_mut().find(|(g, _)| *g == v) {
            Some((_, members)) => members.push((state.clone(), title.clone())),
            None => groups.push((v, vec![(state.clone(), title.clone())])),
        }
    }
    let rank = |v: &str| class_order.iter().position(|c| c == v).unwrap_or(usize::MAX);
    groups.sort_by_key(|(v, _)| rank(v));
    let dim = axis_label(facets, view, axis);
    for (v, members) in groups {
        let label = format!("{dim}: {}", class_label(facets, view, axis, &v));
        if let [(state, title)] = members.as_slice() {
            out[page].rows.push(RowSpec {
                title: label,
                subtitle: Some(title.clone()),
                target: Target::State(state.clone()),
            });
        } else {
            let child = out.len();
            out.push(PageSpec {
                title: label.clone(),
                parent: Some(page),
                rows: Vec::new(),
            });
            out[page].rows.push(RowSpec {
                title: label,
                subtitle: Some(format!("Liczba stanów: {}", members.len())),
                target: Target::Page(child),
            });
            split(out, child, view, &members, facets);
        }
    }
}

/// `(page, row)` of the leaf row showing `state_id`; `None` if absent.
pub fn locate_leaf(pages: &[PageSpec], state_id: &str) -> Option<(usize, usize)> {
    pages.iter().enumerate().find_map(|(p, page)| {
        page.rows
            .iter()
            .position(|r| r.target == Target::State(state_id.to_string()))
            .map(|row| (p, row))
    })
}

/// Page indices from the root down to `page` (inclusive).
pub fn page_path(pages: &[PageSpec], page: usize) -> Vec<usize> {
    let mut path = vec![page];
    let mut current = page;
    while let Some(parent) = pages.get(current).and_then(|p| p.parent) {
        path.push(parent);
        current = parent;
    }
    path.reverse();
    path
}
