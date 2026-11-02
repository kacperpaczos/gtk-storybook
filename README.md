# gtk-storybook

An agent skill (`SKILL.md`) that turns a coding agent into a careful, repeatable process for one job: **show every state of a GTK app on one map**, using the app's own widgets and its own data types, not mock-ups.

The result is a separate `storybook/` app in the host's own language. It has three columns: a menu of every state, a zoomable map of how a user moves between screens, and a live preview of the selected state rendered by the real view code.

> **Supported today:** Rust (gtk-rs) apps on GTK 4.14+ / libadwaita 1.5+. Python, C, C++ and GTK3 are planned. See [Status and honest limits](#status-and-honest-limits).

## The problem, in plain words

Every screen in an app shows one of a few dozen situations: empty list, loading, error, one item, a thousand items, a name so long it wraps, a dialog half-filled. Designers, testers and reviewers want to see all of them at once. Usually nobody can. Some states only appear after ten clicks, a slow network or a failed disk, so they go unseen until a user reports them.

### Why web developers have this almost for free

Many React apps follow three habits that make the problem much easier:

1. **The screen is a function of data.** A *presentational* component receives props and draws itself from them. Hand it different props and you get a different state. (Components that keep local state, read a context or fetch data need extra setup in a story: providers, mocks, decorators. The habit is a discipline, not a guarantee.)
2. **The data is plain and serializable.** The state is a JSON-like object. You can write it in a file, save it, diff it and replay it.
3. **Every change goes through one door.** With Redux, a change is an *action* passed to a pure *reducer*: `reducer(state, action) -> state`. Redux DevTools can log, rewind and replay a session because of that single door.

Tools build on these habits. **Storybook** is easiest because of habit 1: a "story" is a component plus a set of props ("args"). **XState** goes one step further. You write the logic as an explicit state machine, so the states and transitions are data. A tool can draw that graph, compute shortest or bounded paths through it and generate tests that cover every transition.

### Why GTK apps do not

A typical GTK app (Rust, C, C++, Python, GTK3 or GTK4) is written differently, and there is nothing wrong with that:

- Widgets **hold their own state**. A label's text, a button's sensitivity or a stack's visible page is set imperatively from many places.
- The "data" is often **spread across objects**: GObject properties, fields on a window struct, a database handle, a subprocess.
- Changes happen **in signal handlers**. Each one mutates whatever it needs. There is no single reducer and no list of actions.

So there is nothing to hand to a Storybook-like tool and no machine to hand to an XState-like visualizer. Writing that machine by hand is a rewrite nobody will approve. Writing the screens by hand as mock-ups drifts away from the real app on the next commit.

### What this skill does about it

It rebuilds the missing pieces **beside** the app, inside `storybook/`, and lets tools, not the agent's memory, do everything that can be computed:

| Web habit | What the skill builds for a GTK app |
|---|---|
| props → screen | a **view-model (VM) contract**: each view is built from one plain, serializable value of the app's real types |
| state as JSON | **fixtures**: one realistic VM value per state, checked by loading it into the real type |
| list of all states | a finite **presentation model**: for each view, the data dimensions its code actually reads (for example *list: empty / one / many*, *network: ok / error*), grouped into equivalence classes |
| state machine graph | a **flow model**: entry screens, an outcome for every action of every reachable step, and back/self/leaf markers |
| Storybook + XState visualizer | a native **viewer**: menu, flow map with live thumbnails, preview |

The `sbk` checker is a deterministic Rust CLI. It derives the dimensions from the code and multiplies them into **cells** (one cell = one combination of classes, for example *list: many × network: error*). It generates fixtures and checks **completeness of the declared model**:
- every cell has a story;
- every **finding** (code the extractor could not resolve) has a **disposition**: covered by a story, or excluded with a reason;
- every action has an outcome;
- everything is reachable from an entry screen;
- the map layout is reproducible.

This is a proof about the model the agent declared, not about every behaviour the app could ever have. A view the agent never listed, or a transition written down wrongly, is not caught by arithmetic. That is why each judgement call (which views exist, which classes are equivalent, which values are realistic, where an action leads) is written into a file you can review, and checked by a **gate**: a check that must be green before the storybook counts as done. A red gate means "fix the model or the code", never "relax the checker".

## What you get

- **Story menu** (GNOME HIG drill-down): views → user steps → data dimensions → single states, plus search (Ctrl+F). Human names come from `states/<view>.toml`: `title`, `[titles]` and `[labels]`.
- **Flow map**: an endless canvas laid out by Graphviz `dot` (builtin engine as fallback). Each node is a live thumbnail of the real widget. The map is calm by default: no outlines, unlabelled edges. Hovering or selecting a state focuses it. Its transitions turn accent and show their action names, and unrelated parts dim. The `+N` chip expands a step's data variants inline, and the map re-flows with an animation so nothing overlaps.
- **Live preview** of the selected state at phone/tablet/desktop widths, in light or dark, with an Events pane that logs the actions the real view fires.
- **One window, three columns**: menu | map | preview. Clicking on the map or in the menu selects the same state in all three. The side columns can be hidden.
- **Gates**: `sbk verify` runs them all: contract, coverage, fixtures, flows, titles, fresh fingerprints (the code has not changed since extraction) and a deterministic `layout.json`.

The steps use these `sbk` commands: `probe` (census and tiers), `contract`, `scan` (extract axes, actions, findings), `expand` (cells), `gen` (fixtures), `check fixtures|coverage|flows|titles`, `scaffold` (the shell) and `verify`.

## Install

```sh
git clone https://github.com/kacperpaczos/gtk-storybook ~/.claude/skills/gtk-storybook
```

Any agent that reads `SKILL.md` skills works the same way: put the folder into its skills directory. Then ask: *"build a storybook for this GTK app"*. The skill copies [`assets/kit/`](assets/kit/) into `storybook/kit/` of the host repo and walks through the steps in [SKILL.md](SKILL.md). Formats and the viewer spec are in [references/](references/).

Once it is built, open the viewer from the host repo (Rust): `cargo run --manifest-path storybook/Cargo.toml`. Re-run the gates after changing the app: `cargo run --manifest-path storybook/kit/Cargo.toml --bin sbk -- verify`.

## Requirements

Rust toolchain (for `sbk` and the Rust viewer), GTK 4.14+ and libadwaita 1.5+ development files for the viewer, Graphviz (optional, recommended).

## Words used above

- **view-model (VM)**: a plain value holding everything one view displays; the view is built from it.
- **dimension / class**: something the view's code reads (a list's length) and the situations worth showing for it (empty, one, many). Classes that look the same may be declared **equivalent**.
- **flow node**: a step a user can actually reach; data combinations of the same step are its **variants** (the `+N` chip on the map).
- **back / self / leaf**: an action that returns to the previous screen, stays on the same one, or intentionally ends a journey.

## Status and honest limits

**Unreleased changes after v0.1.**

**Works today.** The full pipeline and the Rust/GTK4 viewer for Rust (gtk-rs) hosts, used on a real app (SkillCraft, Rust + libadwaita: 10 views, 20 flow nodes). The extractor reads Rust only. `references/adapters.md` describes how the other languages would plug in. None of them works yet.

**Not there yet.** These are planned, in this order:

1. **Host untouched.** Today step 2 may edit the host to meet the VM contract. Most edits are additive (for example a VM type beside a view). Moving side effects out of a view changes behaviour, so it happens only where the host's tests cover it or you accept a manual check. Every edit is listed in the final report. The goal is zero edits outside `storybook/`: the storybook imports the real host code and runs it with fakes for files, network, subprocesses and root.
2. **Computed transitions.** Today the agent writes `flows.toml` and the gates check it. The goal is the missing third habit, an `update(state, message) -> (state, effects)` function obtained from the host's handlers. With it a tool explores the graph, `flows.toml` is generated, and `sbk paths` lists the shortest path to every state.
3. **More hosts.** First-class C, C++ (gtkmm) and Python adapters, GTK3 as well as GTK4. The first external test target is [linuxmint/mintupdate](https://github.com/linuxmint/mintupdate) (Python, GTK3).

## License

MIT
