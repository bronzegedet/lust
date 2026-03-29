# Lust Stdlib

This folder is for Lust-authored standard-library modules.

The intended split is:

- `lust_src/std/`
  - pure Lust modules
  - string/data helpers
  - parsing utilities
  - DSL/compiler-like helpers such as `lustgex`

- Rust host/runtime modules
  - files
  - env/process
  - draw
  - audio
  - other OS/crate-backed capabilities

Initial modules:

- [`dispatch.lust`](dispatch.lust)
- [`helpers.lust`](helpers.lust)
- [`lustgex.lust`](lustgex.lust)
- [`path.lust`](path.lust)
- [`strings.lust`](strings.lust)
