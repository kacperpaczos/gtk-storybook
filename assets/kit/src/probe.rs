//! Host-contract probe: tier A (conforms), B (VM separation exists, but
//! derives or visibility missing), C (no usable VM separation: I/O inside
//! views, or no views at all). See `references/contract.md`.

use crate::scan::ScanReport;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    A,
    B,
    C,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewProbe {
    pub id: String,
    pub module: String,
    pub vm_type: String,
    pub actions_type: String,
    pub tier: Tier,
    pub misses: Vec<String>,
}

/// `probe.json`: the tier plus the view census.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeReport {
    pub tier: Tier,
    pub views: Vec<ViewProbe>,
}

pub fn probe(report: &ScanReport) -> ProbeReport {
    let views: Vec<ViewProbe> = report
        .inventory
        .views
        .iter()
        .map(|view| {
            let mut misses = Vec::new();
            match report.types.get(&view.vm_type) {
                None => misses.push("vm type not found".to_string()),
                Some(info) => {
                    if !info.public {
                        misses.push("vm not pub".to_string());
                    }
                    if !info.serde {
                        misses.push("vm missing Serialize".to_string());
                    }
                    if !info.de {
                        misses.push("vm missing Deserialize".to_string());
                    }
                    if !info.clone_eq {
                        misses.push("vm missing Clone+PartialEq".to_string());
                    }
                }
            }
            match report.types.get(&view.actions_type) {
                None => misses.push("actions type not found".to_string()),
                Some(info) => {
                    if !info.public {
                        misses.push("actions not pub".to_string());
                    }
                }
            }
            // module ids are crate-qualified: `<crate>::<path>…`; the
            // first segment names the crate, the second its root module
            let mut parts = view.module.split("::");
            let krate = parts.next().unwrap_or_default();
            if let Some(root_mod) = parts.next()
                && report
                    .root_modules_pub
                    .get(krate)
                    .and_then(|mods| mods.get(root_mod))
                    == Some(&false)
            {
                misses.push("module not pub".to_string());
            }
            let tier = if report.io_modules.contains(&view.module) {
                Tier::C
            } else if misses.is_empty() {
                Tier::A
            } else {
                Tier::B
            };
            ViewProbe {
                id: view.id.clone(),
                module: view.module.clone(),
                vm_type: view.vm_type.clone(),
                actions_type: view.actions_type.clone(),
                tier,
                misses,
            }
        })
        .collect();
    ProbeReport {
        tier: overall_tier(&views),
        views,
    }
}

fn overall_tier(views: &[ViewProbe]) -> Tier {
    if views.is_empty() || views.iter().any(|v| v.tier == Tier::C) {
        Tier::C
    } else if views.iter().any(|v| v.tier == Tier::B) {
        Tier::B
    } else {
        Tier::A
    }
}

/// Problems for the `contract --check` gate (empty = green).
pub fn contract_problems(report: &ProbeReport) -> Vec<String> {
    let mut problems = Vec::new();
    if report.views.is_empty() {
        problems.push(
            "no views found: no pub fn build(vm: &Vm, actions: &Actions) -> gtk::Widget"
                .to_string(),
        );
    }
    for view in &report.views {
        if view.tier == Tier::C {
            problems.push(format!("view {}: module has I/O effects (tier C)", view.id));
        }
        for miss in &view.misses {
            problems.push(format!("view {id}: {miss}", id = view.id));
        }
    }
    problems
}
