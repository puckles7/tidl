# Streamline local/remote handling for opening a file in an external editor

**Status:** done.

The old split between General → "Server editor command" and This Device →
"Local editor", plus the four target states in the preview, is replaced by one
editors model: git sources always open from the user's own clone on this
device; filesystem sources open externally only when the browser is on the tidl
host; tidl's managed clone is never offered for manual external editing.

- **Design:** [spec](../docs/superpowers/specs/2026-11-06-editors-tab-design.md)
- **Plan:** [plan](../docs/superpowers/plans/2026-11-06-editors-tab.md)

## Resolution

- Renamed Settings → "This Device" to **Editors**; it owns the single editor
  choice, the local clone mapping, and an Advanced section for the custom URI
  template and browser-location override.
- Removed `Server editor command` from General. Each editor preset now supplies
  a default server-side CLI command, with an optional "Command on the tidl
  host" override under Advanced (launcher names vary across platforms and
  packaging), so the editor is configured once.
- `OpenTarget` collapsed to `LocalClone` / `HostCopy` / `ConfigureClone` /
  `Unavailable`. Git sources never open tidl's managed clone; the server-clone
  pull/commit recovery UI was removed.
- Preview actions: `Open in <editor> →`, `Open on tidl host →`, a disabled
  `Open on tidl host` with a hover explanation for remote filesystem sources,
  and an inline source-scoped `Set up clone…` prompt (no more settings detour).
- Clone keys now reconcile SSH/SCP/HTTPS spellings (`host/owner/repo`) and
  legacy `localStorage` keys are migrated on load. Paths are checked for being
  absolute; the `Test` link opens the clone root with the chosen editor.
- Browser location stays auto-detected (loopback heuristic) with a manual
  override under Advanced; it now only affects filesystem sources.
