# libadwaita-storybook

An agent skill (`SKILL.md`) that gives any GTK4 / libadwaita app a **storybook**: a `storybook/` app in the host's own language that renders every state of every view from the app's real view-models, plus an endless-canvas **flow map** of the user journey (node = state, live thumbnails, Graphviz layout).

Works with Rust, Python, C, Vala and GJS hosts (neutral core + language adapters).

## What you get

- **`sbk` checker** (Rust, dev-only): census, contract check, extraction, state expansion, fixture generation and gates that prove completeness — every cell of a finite presentation model has a story, every finding has a disposition, every action has an outcome.
- **Viewer**: story list with width presets and light/dark, Events pane, and a flow map that reads left to right; data variants expand inline and the layout re-flows with animation, nothing ever overlaps.
- **Graphviz `dot`** (optional, recommended) for the flow-map layout; a builtin engine is the fallback.

## Install

Copy this directory into your agent's skills folder, e.g.

```sh
git clone https://github.com/kacperpaczos/libadwaita-storybook ~/.claude/skills/libadwaita-storybook
```

Then ask your agent for "a storybook for this GTK app". The skill copies `assets/kit/` into `storybook/kit/` of the host repo; see [SKILL.md](SKILL.md) for the steps and [references/](references/) for formats and the viewer spec.

## Requirements

Rust toolchain (for `sbk` and the Rust viewer), GTK 4.14+ and libadwaita 1.5+ development files, Graphviz (optional).

## Status

v0.1 — first public release; the Rust viewer is complete, other languages port it from `references/canvas.md`.

## License

MIT
