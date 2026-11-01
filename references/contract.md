# Host contract (language-neutral)

**Given a presentation state (the view-model), an environment (theme, width, locale) and action ports, constructing or updating the real view deterministically produces its observable UI without invoking production effects** (no I/O, clock, timers, randomness, network, processes, backend singletons).

A storybook-ready view offers four capabilities, in whatever shape the language makes natural:

| Capability | Meaning |
|---|---|
| create / apply | build the real view (function, constructor, template class) and apply a VM to it |
| snapshot / restore | the VM round-trips through a canonical codec (JSON, sorted keys, versioned) |
| action interception | every user intent leaves through an action port the storybook can record |
| disposal | the view can be dropped without side effects |

## The view-model
- **Complete presentation state**: drafts and entered text, validation messages, selection, which page/dialog is open, operation phase (idle / pending / ok / err) when the view shows one, items of lists.
- **Plain data**: no widgets, callbacks or backend handles inside. A GObject whose only members are data properties qualifies.
- **Lists**: the VM carries plain items; the view wraps them in its `GListModel`. Selection, filter and sort state are VM fields.

## GObject and templates
- Properties on the VM object are fields. Signals the view emits for user intent are actions; signals on the VM are findings.
- `bind-source` / `bind-property` / `GtkExpression` lookups that read VM properties are reads (projection works from the template). A binding or closure over any non-VM object is a finding.
- Template classes stay as they are: the storybook instantiates the real class and sets the VM.
- **Widget-local state** (entry buffers, expander state, visible-child set by code, selection held only by a widget): move it into the VM, or give its finding a disposition. Scroll position and focus are the usual `excluded` cases.

## Tiers (per view)
| Tier | Meaning | Agent action |
|---|---|---|
| A | conforms | none |
| B | VM exists; codec or visibility missing | add codec / exports only |
| B2 | no VM, but the view's inputs are plain values | add the VM beside the view and a `vm_from_domain` conversion at the existing call site (additive) |
| C | construction interleaves awaited I/O, mutates shared domain objects others observe, reaches backends through unstubbable globals, or is generated from unenumerable data | census + refactor list; no stories for this view |
