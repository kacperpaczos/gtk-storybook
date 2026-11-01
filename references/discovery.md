# Discovery rules (`sbk probe` / `sbk scan` / `sbk expand`)

## Sources, in order of authority
1. **`storybook/views.toml`** (agent-authored census): the views that exist and their entry points.
2. **GObject introspection** of VM classes (param specs, GEnum/GFlags values) and **GtkBuilder XML** (templates, `bind-source`, `GtkStack`/`AdwViewStack` page names, `AdwBreakpoint` conditions, template child ids). Blueprint is compiled with the host's `blueprint-compiler` to XML first; provenance maps back to the `.blp` line.
3. **Language extractor** (adapter): which VM fields each view reads, actions, edge candidates. Static analysis is strong in Rust, Vala, C with a compilation database; advisory in Python and GJS, where the authority becomes the union of runtime-observed VM shapes from captured or generated fixtures checked against the declared axes.

Every extractor declares its **capabilities** in `inventory.json`; whatever it cannot resolve is a finding, never silently omitted.

## Projection
Only fields a view actually reads (code, bindings, expressions) become axes; unread fields are listed (they suggest an invented model).

## Axis classes
| Type | Classes |
|---|---|
| bool | `false`, `true` |
| nullable (Option, None, NULL, `?`) — only with type evidence | `none`, `some` |
| enum / GEnum | one class per value; payloads partitioned recursively |
| GFlags | the valid combinations the view distinguishes (declared in states file; default each single flag + none) |
| list / GListModel items | `empty`, `one`, `many` (fixture size 120 — a chosen class, not exhaustive) |
| operation phase (only when the VM models one) | `idle`, `pending`, `ok`, `err` |
| string / number | `any` until the agent declares boundary classes (`empty`, `short`, `overflow`, `invalid`…) |
| page / visible-child name | one class per page |
| nested record | its read fields, recursively |

Cells = product of a view's axes, minus guard-excluded cells, merged by equivalence rules. Default budget 64 cells per view; over budget is red.

## Findings
`id` (stable hash of view + path + kind), `location` (file:line, .blp mapped), `fingerprint` (hash of the source span or compiled XML node), `extractor` + `version`, `kind`:
`unresolved-type`, `macro`, `widget-local-state`, `non-vm-binding`, `vm-signal`, `dynamic-registration`, `unread-field`, `feature-gated`, `edge-candidate`, `extractor-limit`.
A changed fingerprint reopens its disposition.
