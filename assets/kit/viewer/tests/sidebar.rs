//! Pure sidebar model (references/canvas.md "Story list"): drill-down
//! groups per view, search over every state, programmatic location.

use std::collections::BTreeMap;
use storybook_kit::model::{Layout, NodePos, VariantPos};
use storybook_viewer::sidebar::{ViewGroup, build_model, humanize, search};

fn node(id: &str, lane: u32, depth: u32, order: u32, title: Option<&str>) -> NodePos {
    NodePos {
        state_id: id.to_string(),
        lane,
        depth,
        order,
        title: title.map(str::to_string),
        self_actions: Vec::new(),
        variants: Vec::new(),
        variant_count: 0,
        rect: None,
    }
}

fn variant(
    id: &str,
    of: &str,
    lane: u32,
    depth: u32,
    order: u32,
    title: Option<&str>,
) -> VariantPos {
    VariantPos {
        state_id: id.to_string(),
        variant_of: of.to_string(),
        lane,
        depth,
        order,
        title: title.map(str::to_string),
    }
}

/// Two views; the second has a node with two data variants.
fn model() -> Vec<ViewGroup> {
    let views = vec!["skills".to_string(), "skill_detail".to_string()];
    let layout = Layout {
        nodes: vec![
            node("skills/empty", 0, 0, 0, Some("Pusto")),
            node("skill_detail/dirty", 1, 0, 0, None),
        ],
        variants: vec![
            variant(
                "skill_detail/plain",
                "skill_detail/dirty",
                1,
                0,
                0,
                Some("Bez zmian"),
            ),
            variant("skill_detail/broken", "skill_detail/dirty", 1, 0, 1, None),
        ],
        edges: Vec::new(),
        engine: "builtin".to_string(),
        view_titles: BTreeMap::from([
            ("skills".to_string(), "Lista skilli".to_string()),
            ("skill_detail".to_string(), "Szczegóły skilla".to_string()),
        ]),
        dot_version: None,
        clusters: Vec::new(),
    };
    build_model(&views, &layout)
}

#[test]
fn groups_follow_inventory_order_with_counts() {
    let m = model();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].id, "skills");
    assert_eq!(m[0].title, "Lista skilli");
    // one node, no variants
    assert_eq!(m[0].count, 1);
    assert_eq!(m[1].id, "skill_detail");
    assert_eq!(m[1].count, 3); // node + two variants
    assert_eq!(m[1].nodes.len(), 1);
}

#[test]
fn variants_attach_to_their_node_in_stack_order() {
    let m = model();
    let detail = &m[1].nodes[0];
    assert_eq!(detail.state_id, "skill_detail/dirty");
    assert_eq!(detail.title, "dirty"); // story_title fallback
    assert_eq!(
        detail.variants,
        vec![
            ("skill_detail/plain".to_string(), "Bez zmian".to_string()),
            // authored title missing: story_title fallback
            ("skill_detail/broken".to_string(), "broken".to_string()),
        ]
    );
}

#[test]
fn view_title_falls_back_to_humanized_id() {
    let views = vec!["skill_detail".to_string()];
    let layout = Layout {
        nodes: vec![node("skill_detail/dirty", 0, 0, 0, None)],
        variants: Vec::new(),
        edges: Vec::new(),
        engine: "builtin".to_string(),
        view_titles: BTreeMap::new(),
        dot_version: None,
        clusters: Vec::new(),
    };
    let m = build_model(&views, &layout);
    assert_eq!(m[0].title, "Skill detail");
}

#[test]
fn humanize_capitalizes_and_spaces_ids() {
    assert_eq!(humanize("skill_detail"), "Skill detail");
    assert_eq!(humanize("skill-files"), "Skill files");
    assert_eq!(humanize("shell"), "Shell");
}

#[test]
fn nodes_within_a_view_follow_journey_order() {
    let views = vec!["v".to_string()];
    let layout = Layout {
        nodes: vec![
            node("v/b", 0, 1, 0, None),
            node("v/a", 0, 0, 0, None),
            node("v/c", 0, 1, 1, None),
        ],
        variants: Vec::new(),
        edges: Vec::new(),
        engine: "builtin".to_string(),
        view_titles: BTreeMap::new(),
        dot_version: None,
        clusters: Vec::new(),
    };
    let m = build_model(&views, &layout);
    let ids: Vec<&str> = m[0].nodes.iter().map(|n| n.state_id.as_str()).collect();
    assert_eq!(ids, vec!["v/a", "v/b", "v/c"]);
}

#[test]
fn search_matches_title_view_name_and_state_id_case_insensitively() {
    let m = model();
    // node title
    assert_eq!(search(&m, "pusto"), vec!["skills/empty".to_string()]);
    // variant authored title, mixed case query
    assert_eq!(
        search(&m, "BEZ ZMIAN"),
        vec!["skill_detail/plain".to_string()]
    );
    // view display name matches every state of the view
    assert_eq!(search(&m, "lista"), vec!["skills/empty".to_string()]);
    // raw state id
    assert_eq!(
        search(&m, "skill_detail/broken"),
        vec!["skill_detail/broken".to_string()]
    );
    // state id also matches on its classes part alone
    assert_eq!(
        search(&m, "broken"),
        vec!["skill_detail/broken".to_string()]
    );
    // view id matches too
    assert_eq!(search(&m, "SKILLS"), vec!["skills/empty".to_string()]);
    // empty query matches nothing
    assert!(search(&m, "").is_empty());
    // no hit
    assert!(search(&m, "nie ma takiego").is_empty());
}

// --- drill-down pages by data dimension -------------------------------------

mod pages_tree {
    use std::collections::BTreeMap;
    use storybook_viewer::sidebar::{
        Facets, NodeGroup, PageSpec, Target, ViewGroup, locate_leaf, page_path, pages,
    };

    fn facets() -> Facets {
        let mut f = Facets::default();
        f.axes.insert(
            "install".into(),
            vec![
                ("source".into(), vec!["none".into(), "some".into()]),
                (
                    "found".into(),
                    vec!["empty".into(), "one".into(), "many".into()],
                ),
                ("scopes".into(), vec!["one".into(), "many".into()]),
            ],
        );
        let mut a = |id: &str, s: &str, fo: &str, sc: &str| {
            f.assignment.insert(
                id.into(),
                BTreeMap::from([
                    ("source".into(), s.into()),
                    ("found".into(), fo.into()),
                    ("scopes".into(), sc.into()),
                ]),
            );
        };
        a("install/start", "none", "empty", "one");
        a("install/start-p", "none", "empty", "many");
        a("install/one", "some", "one", "one");
        a("install/many-1", "some", "many", "one");
        a("install/many-p", "some", "many", "many");
        f.labels.insert(
            "install".into(),
            BTreeMap::from([
                ("scopes".into(), "Zasięg".into()),
                ("scopes.many".into(), "globalny + projekt".into()),
                ("found".into(), "Znaleziono".into()),
            ]),
        );
        f
    }

    fn model() -> Vec<ViewGroup> {
        vec![ViewGroup {
            id: "install".into(),
            title: "Instalacja skilla".into(),
            count: 5,
            nodes: vec![
                NodeGroup {
                    state_id: "install/start".into(),
                    title: "Start".into(),
                    variants: vec![("install/start-p".into(), "Start z projektem".into())],
                },
                NodeGroup {
                    state_id: "install/one".into(),
                    title: "Jeden skill".into(),
                    variants: vec![
                        ("install/many-1".into(), "Wiele".into()),
                        ("install/many-p".into(), "Wiele z projektem".into()),
                    ],
                },
            ],
        }]
    }

    fn page<'a>(ps: &'a [PageSpec], title: &str) -> &'a PageSpec {
        ps.iter().find(|p| p.title == title).unwrap()
    }

    #[test]
    fn root_lists_views_and_view_page_lists_nodes() {
        let ps = pages(&model(), &facets());
        assert_eq!(ps[0].title, "Views");
        assert_eq!(ps[0].rows[0].subtitle.as_deref(), Some("5 states"));
        assert_eq!(ps[0].rows[0].title, "Instalacja skilla");
        assert_eq!(ps[0].rows[0].target, Target::Page(1));
        let titles: Vec<&str> = ps[1].rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Start", "Jeden skill"]);
    }

    #[test]
    fn node_page_splits_by_first_differing_dimension_with_labels() {
        let ps = pages(&model(), &facets());
        let start = page(&ps, "Start");
        // only scopes differs: one level, two leaves, class order, labels
        let rows: Vec<(&str, &Target)> = start
            .rows
            .iter()
            .map(|r| (r.title.as_str(), &r.target))
            .collect();
        assert_eq!(
            rows,
            [
                ("Zasięg: one", &Target::State("install/start".into())),
                (
                    "Zasięg: globalny + projekt",
                    &Target::State("install/start-p".into())
                ),
            ]
        );
        assert_eq!(start.rows[1].subtitle.as_deref(), Some("Start z projektem"));
    }

    #[test]
    fn shared_value_becomes_a_further_page_and_constant_dims_are_skipped() {
        let ps = pages(&model(), &facets());
        let one = page(&ps, "Jeden skill");
        // source is "some" for all three -> skipped; found differs first
        assert_eq!(one.rows[0].title, "Znaleziono: one");
        assert_eq!(one.rows[0].target, Target::State("install/one".into()));
        let Target::Page(many) = one.rows[1].target else {
            panic!("many must be a page")
        };
        assert_eq!(ps[many].title, "Znaleziono: many");
        let leaf_titles: Vec<&str> = ps[many].rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(leaf_titles, ["Zasięg: one", "Zasięg: globalny + projekt"]);
    }

    #[test]
    fn node_without_variants_is_a_leaf_on_the_view_page() {
        let mut m = model();
        m[0].nodes[0].variants.clear();
        let ps = pages(&m, &facets());
        assert_eq!(ps[1].rows[0].target, Target::State("install/start".into()));
    }

    #[test]
    fn no_assignment_falls_back_to_plain_leaves() {
        let ps = pages(&model(), &Facets::default());
        let start = page(&ps, "Start");
        let titles: Vec<&str> = start.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Start", "Start z projektem"]);
    }

    #[test]
    fn locate_leaf_and_page_path_reach_deep_states() {
        let ps = pages(&model(), &facets());
        let (p, row) = locate_leaf(&ps, "install/many-p").unwrap();
        assert_eq!(ps[p].title, "Znaleziono: many");
        assert_eq!(row, 1);
        let titles: Vec<&str> = page_path(&ps, p)
            .iter()
            .map(|&i| ps[i].title.as_str())
            .collect();
        assert_eq!(
            titles,
            [
                "Views",
                "Instalacja skilla",
                "Jeden skill",
                "Znaleziono: many"
            ]
        );
        assert_eq!(locate_leaf(&ps, "install/nope"), None);
    }

    #[test]
    fn a_state_without_assignment_is_a_plain_leaf_never_a_question_mark() {
        let mut f = facets();
        // install/many-p lost its cells (stale cells.json)
        f.assignment.remove("install/many-p");
        let ps = pages(&model(), &f);
        assert!(
            ps.iter()
                .flat_map(|p| &p.rows)
                .all(|r| !r.title.contains('?'))
        );
        let one = page(&ps, "Jeden skill");
        let titles: Vec<&str> = one.rows.iter().map(|r| r.title.as_str()).collect();
        // known values split; the unassigned state follows as a plain leaf
        assert_eq!(
            titles,
            ["Znaleziono: one", "Znaleziono: many", "Wiele z projektem"]
        );
        // every state is reachable exactly once
        for id in [
            "install/start",
            "install/start-p",
            "install/one",
            "install/many-1",
            "install/many-p",
        ] {
            let n = ps
                .iter()
                .flat_map(|p| &p.rows)
                .filter(|r| r.target == Target::State(id.into()))
                .count();
            assert_eq!(n, 1, "{id}");
        }
    }

    #[test]
    fn unknown_values_sort_after_known_classes_in_order_seen() {
        let mut f = facets();
        f.assignment
            .get_mut("install/one")
            .unwrap()
            .insert("found".into(), "zzz".into());
        let ps = pages(&model(), &f);
        let one = page(&ps, "Jeden skill");
        let titles: Vec<&str> = one.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Znaleziono: many", "Znaleziono: zzz"]);
    }

    #[test]
    fn labels_fall_back_to_last_axis_segment_and_spaced_class() {
        use storybook_viewer::sidebar::{axis_label, class_label};
        let f = Facets::default();
        assert_eq!(axis_label(&f, "v", "vm.scope_kind"), "Scope kind");
        assert_eq!(axis_label(&f, "v", "found"), "Found");
        assert_eq!(class_label(&f, "v", "found", "many_items"), "many items");
        let labelled = facets();
        assert_eq!(axis_label(&labelled, "install", "scopes"), "Zasięg");
        assert_eq!(
            class_label(&labelled, "install", "scopes", "many"),
            "globalny + projekt"
        );
        // a label of another view does not leak
        assert_eq!(axis_label(&labelled, "other", "scopes"), "Scopes");
    }
}
