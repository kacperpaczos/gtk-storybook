//! `probe::probe` — host tiers A/B/C against the contract.

mod common;

use common::{edit, minihost_copy};
use std::fs;
use storybook_kit::probe::{Tier, probe};
use storybook_kit::scan::scan;

fn tier_of(dir: &std::path::Path) -> (Tier, Vec<(String, Tier)>) {
    let report = probe(&scan(dir));
    (
        report.tier,
        report
            .views
            .iter()
            .map(|v| (v.id.clone(), v.tier))
            .collect(),
    )
}

#[test]
fn conforming_host_is_tier_a() {
    let (tier, views) = tier_of(&minihost_copy("probe-a"));
    assert_eq!(tier, Tier::A);
    assert_eq!(
        views,
        vec![
            ("deep".into(), Tier::A),
            ("corners".into(), Tier::A),
            ("pairy".into(), Tier::A),
            ("pm".into(), Tier::A),
            ("pf".into(), Tier::A),
            ("pv".into(), Tier::A),
            ("alt".into(), Tier::A),
            ("create".into(), Tier::A),
            ("settings".into(), Tier::A),
            ("skill_detail".into(), Tier::A),
        ]
    );
    let report = probe(&scan(&minihost_copy("probe-a2")));
    assert!(report.views.iter().all(|v| v.misses.is_empty()));
}

#[test]
fn missing_derives_or_pub_is_tier_b() {
    let dir = minihost_copy("probe-b");
    edit(
        &dir.join("src/create.rs"),
        "#[derive(Clone, PartialEq, Serialize, Deserialize)]\npub struct CreateVm",
        "#[derive(Clone, Serialize, Deserialize)]\npub struct CreateVm",
    );
    let (tier, views) = tier_of(&dir);
    assert_eq!(tier, Tier::B);
    assert!(views.contains(&("create".into(), Tier::B)));
    let report = probe(&scan(&dir));
    let create = report.views.iter().find(|v| v.id == "create").unwrap();
    // tier B covers the codec contract AND the Clone+PartialEq capability
    assert!(
        create.misses.iter().any(|m| m.contains("Clone+PartialEq")),
        "{:?}",
        create.misses
    );
    // dropping every derive keeps tier B with all three misses
    edit(
        &dir.join("src/create.rs"),
        "#[derive(Clone, Serialize, Deserialize)]\npub struct CreateVm",
        "#[derive(Clone)]\npub struct CreateVm",
    );
    let report = probe(&scan(&dir));
    let create = report.views.iter().find(|v| v.id == "create").unwrap();
    assert!(create.misses.iter().any(|m| m.contains("Serialize")));
    assert!(create.misses.iter().any(|m| m.contains("Deserialize")));
    assert!(create.misses.iter().any(|m| m.contains("Clone+PartialEq")));
}

#[test]
fn non_pub_vm_is_tier_b() {
    let dir = minihost_copy("probe-b2");
    edit(
        &dir.join("src/create.rs"),
        "pub struct CreateVm",
        "struct CreateVm",
    );
    let report = probe(&scan(&dir));
    let create = report.views.iter().find(|v| v.id == "create").unwrap();
    assert_eq!(create.tier, Tier::B);
    assert!(create.misses.iter().any(|m| m.contains("not pub")));
}

#[test]
fn non_pub_module_is_tier_b() {
    let dir = minihost_copy("probe-b3");
    edit(&dir.join("src/lib.rs"), "pub mod create;", "mod create;");
    let report = probe(&scan(&dir));
    let create = report.views.iter().find(|v| v.id == "create").unwrap();
    assert_eq!(create.tier, Tier::B);
    assert!(create.misses.iter().any(|m| m == "module not pub"));
}

#[test]
fn io_inside_a_view_is_tier_c() {
    let dir = minihost_copy("probe-c");
    edit(
        &dir.join("src/settings.rs"),
        "    let _ = vm.marker;\n    let _ = &vm.prompt;",
        "    let _ = vm.marker;\n    let _ = std::fs::read_to_string(\"x\");\n    let _ = &vm.prompt;",
    );
    let (tier, views) = tier_of(&dir);
    assert_eq!(tier, Tier::C);
    assert!(views.contains(&("settings".into(), Tier::C)));
}

#[test]
fn host_without_views_is_tier_c() {
    let dir = minihost_copy("probe-c2");
    fs::remove_file(dir.join("src/create.rs")).unwrap();
    fs::remove_file(dir.join("src/settings.rs")).unwrap();
    fs::remove_file(dir.join("src/skill_detail.rs")).unwrap();
    fs::remove_file(dir.join("src/corners.rs")).unwrap();
    fs::remove_dir_all(dir.join("nested")).unwrap();
    edit(
        &dir.join("src/lib.rs"),
        "pub mod corners;\npub mod create;\npub mod settings;\npub mod skill_detail;",
        "",
    );
    let (tier, views) = tier_of(&dir);
    assert_eq!(tier, Tier::C);
    assert!(views.is_empty());
}

#[test]
fn use_of_std_io_modules_is_tier_c() {
    let dir = minihost_copy("probe-c3");
    edit(
        &dir.join("src/skill_detail.rs"),
        "use crate::gtk::{Button, Widget};",
        "use crate::gtk::{Button, Widget};\nuse std::fs;",
    );
    let (tier, views) = tier_of(&dir);
    assert_eq!(tier, Tier::C);
    assert!(views.contains(&("skill_detail".into(), Tier::C)));
}
