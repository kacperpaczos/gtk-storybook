---
name: libadwaita-storybook
description: Builds a `storybook/` UI-shell app for any GTK4/libadwaita project (Rust, Python, C, Vala, GJS) in the host's own language, rendering every state of every view from the app's real view-models plus an endless-canvas flow map from the first view to every leaf. Use when asked for a storybook, a map of all views/states/transitions, or UI-state coverage of a GTK app.
---

# GTK / libadwaita storybook

You give a GTK4 app a **storybook**: everything lives in `storybook/` in the host repo. It reuses the host's real view code, templates, CSS and resources, in the host's language and bindings, and renders each view from **view-model fixtures** — the app's real, coded presentation state, never invented screens. It shows a story list and a **flow map** (lanes per view, depth from the first view, live thumbnails).

**Completeness** means every cell of a declared, finite presentation model has a story, every finding about the code has a disposition, and every action has an outcome. The `sbk` checker proves it on a language-neutral IR; you supply judgement (which views, equivalence classes, guards, realistic values, transitions). A red gate is information: fix the model or the code, never the checker.

The only edits outside `storybook/` are the minimal contract refactors of step 2, each listed in the final report.

## Setup

Copy the kit from [`assets/kit/`](assets/kit/) to `storybook/kit/` and build the checker once: `cargo build --manifest-path storybook/kit/Cargo.toml --bin sbk` (`sbk` is a dev tool, never linked into the host). Graphviz is an optional (recommended) dependency: `sbk verify` uses `dot` for the flow-map layout when it is on `PATH` (`SBK_DOT=/path` overrides, `SBK_DOT=off` disables) and falls back to the builtin engine with a warning otherwise. Detect the host language and binding, then open its adapter section in [adapters](references/adapters.md); it names the extractor, codec, validator, shell template and purity gate you use below.

## Steps

1. **Census.** Write `storybook/views.toml`: one entry per view (entry point — function, template class or `set_vm` method — VM type, actions), from reading the host. Then `sbk probe` runs the language extractor and GtkBuilder/Blueprint extraction and grades each view against [the contract](references/contract.md): A conforms, B needs codec/visibility only, B2 needs an additive VM, C cannot be separated safely.
   *Done when* every view in the host appears in `views.toml` with a tier, and `sbk probe` lists no view you have not accounted for.

2. **Meet the contract, one view at a time.** A: nothing. B: add the VM codec / visibility. B2: introduce the VM beside the view and a `vm_from_domain` conversion at the existing call site — additive, behaviour-preserving. Moving effects out of a view into actions changes behaviour: do it only where the host's tests cover it or the user accepts a manual check. C: deliver the census and refactor list for that view and give it no stories.
   *Done when* `sbk contract --check` is green and the host's own build, lint and tests are green.

3. **Extract.** `sbk scan --write` produces `storybook/inventory.json` (rules: [discovery](references/discovery.md)): axes from the VM fields each view actually reads (code, template bindings, GObject property specs), actions, edge candidates, and a **finding** (stable ID, location, fingerprint, extractor + version) for everything the extractor cannot resolve.
   *Done when* `sbk scan --check` reports zero changes on a re-run.

4. **Model the states.** In `storybook/states/<view>.toml` give axes their classes, add evidenced guards and equivalence rules, give every finding a **disposition** (`covered-by` a story, or `excluded` with a reason that narrows the claim), and give every state a human `title` in `[titles]` (the flow-map node label). Formats: [formats](references/formats.md).
   *Done when* `sbk expand --check` is green and `sbk check titles` reports no missing titles: findings disposed, guards consistent, each view within its cell budget. Over budget stays red until you add guards or equivalences; sampling never counts as complete.

5. **Fixtures.** `sbk gen --write` synthesizes one deterministic fixture per cell; replace placeholders with realistic values (long text for overflow cells, real-looking names and paths). Any string still equal to a generated placeholder (`text-<hex>`) keeps `sbk check fixtures` red. Where the user runs the host with the capture probe, curate captured VMs into fixtures — each must still map to a cell.
   *Done when* `sbk check fixtures` is green **and** the host-language validator (`storybook/validate`, from the adapter) loads every fixture into the real VM type.

6. **Shell.** `sbk scaffold --lang <lang>` generates the storybook shell in the host language from the kit templates; fill its registry so each view id builds the host's real view from a fixture with recording actions.
   *Done when* the shell builds/imports cleanly with the host's toolchain and `sbk check coverage` is green.

7. **Flows.** In `storybook/flows.toml` set `roots` (first screen a user sees; conditional roots allowed), an edge for every action of every **flow node** — the user-reachable steps, which appear in `roots`, edges or terminals (candidates from step 3 plus your reading), `back` for history pops, `self` for actions that stay in place, `leaf` for intended ends. Mere data combinations of a step are **not** nodes: list them under `[variants]` of that step (same view).
   *Done when* `sbk check flows` is green: every cell is a flow node or a variant of exactly one, everything reachable from a root, no dangling targets, every action of every flow node has an outcome.

8. **Render proof** (needs a display; only when the user allows GUI). Run the shell with `--check-render`: every story renders, building twice gives identical widget trees (else: hidden widget-local state → finding), and distinct cells with equal render signatures are reported as suspects.
   *Done when* green, or reported as deferred to the user.

Finish with `sbk verify` (all gates, fresh fingerprints, deterministic `layout.json`) and report: language/adapter and its declared capabilities, tiers per view, host edits made, cells, findings by disposition, edges, deferred steps.

## The viewer

Run the shell (adapter gives the command): story list plus flow map, light/dark and width presets, double-click a node to interact with it full size. Spec: [canvas](references/canvas.md).
