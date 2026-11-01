# Language adapters

Each adapter supplies five things; the checker, formats and layout stay shared. Support status is published honestly in the kit README.

| | Rust (gtk-rs) | Python (PyGObject) | C (GObject) | Vala | GJS |
|---|---|---|---|---|---|
| **Status** | supported | first test | planned | planned | planned |
| **View shape** | `fn build(&Vm, &Actions) -> gtk::Widget` (+ `update`) | `def build(vm, actions) -> Gtk.Widget` or template class with `set_vm(vm)` | `GtkWidget *x_build(const XVm *, const XActions *)` or template class with `x_set_vm()` | `Widget build(Vm, Actions)` or template class `set_vm` | `build(vm, actions)` or `GObject.registerClass` template with `setVm` |
| **VM + codec** | struct + serde | `@dataclass` (+ `asdict`/from-dict) or GObject with data properties | struct + `to_json`/`from_json` (json-glib) or data-only GObject | class/struct + json-glib | plain object / GObject + `JSON` |
| **Actions** | struct of `Rc<dyn Fn>` | object/Protocol of callables, or view signals | struct of callbacks, or view signals / `GActionGroup` | delegates or signals | object of functions or signals |
| **Extractor** | `syn` (static, authoritative) | `ast` + runtime reflection (advisory static) | GIR + libclang with compile_commands.json | valac GIR + source | GIR + tree-sitter (advisory) |
| **Validator** (`storybook/validate`) | `cargo run --bin validate` decoding fixtures into VM types | `python -m storybook.validate` constructing dataclasses/GObjects | small C program via json-glib | small Vala program | `gjs -m storybook/validate.js` |
| **Purity gate** | clippy `disallowed-methods` in the view crate | AST import/call scan of view modules (`os`, `subprocess`, `socket`, `Gio.File*`, backend modules) | link/symbol scan of view TUs (`g_file_*`, `g_subprocess_*`, `soup_*`, backend headers) | same as C on generated C | AST scan |
| **Shell** | `storybook/Cargo.toml` (own workspace) + viewer crate | `storybook/__main__.py` + viewer package | `storybook/meson.build` | `storybook/meson.build` | `storybook/main.js` |

## Capture probe (optional, user-run)
A dev-only hook the host imports from `storybook/capture/` that serializes the VM whenever it is applied, on navigation (`notify::visible-child`, navigation `pushed`/`popped`) and on a hotkey, as NDJSON `{view, vm, trigger, location}` into `storybook/fixtures/captured/`. Redaction is allow-list based (`storybook/redact.toml`: field paths kept verbatim; everything else string-scrubbed for emails, tokens, home paths, the user's name). The probe observes only and is never enabled by default. Captured fixtures must map to a cell or become a finding.
