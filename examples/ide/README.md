# IDE Examples

Primary scripts:

- `lusty.lust`: main Lust IDE script used by `lusty`.
- `live_ui_test.lust`: quick live UI controls test script.

Run in GUI host:

```bash
cargo run --bin lusty -- examples/ide/lusty.lust
```

or

```bash
cargo run --bin lusty -- examples/ide/live_ui_test.lust
```

Run in terminal fallback:

```bash
cargo run --bin lust -- ide examples/ide/lusty.lust
```
