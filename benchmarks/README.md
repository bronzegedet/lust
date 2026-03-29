# Benchmarks

These are small Lust programs for measuring the current VM/data-processing path.

They are not a formal benchmark harness yet. They are meant to answer practical
questions like:

- is `lustgex_match(...)` actually slow?
- is `lustgex_capture(...)` slower than manual string parsing?
- how expensive is `json_parse(...)`?
- how much does map-heavy aggregation cost on the VM?

## Current Benchmarks

- `benchmarks/manual_split_scan.lust`
  Scans generated report rows using string helpers and `split("|")`.
- `benchmarks/lustgex_match_scan.lust`
  Scans the same generated report rows using `lustgex_match(...)`.
- `benchmarks/lustgex_capture_scan.lust`
  Extracts fields from the same generated report rows using `lustgex_capture(...)`.
- `benchmarks/json_parse_loop.lust`
  Repeatedly parses a medium JSON payload and reads nested values.
- `benchmarks/map_grouping.lust`
  Builds nested `Map` state and aggregates generated department records.

## Running

Use release mode for timings:

```bash
env CARGO_TARGET_DIR=/tmp/lust-target cargo build --release --bin lust
/tmp/lust-target/release/lust run benchmarks/manual_split_scan.lust 20000
/tmp/lust-target/release/lust run benchmarks/lustgex_match_scan.lust 20000
/tmp/lust-target/release/lust run benchmarks/lustgex_capture_scan.lust 20000
/tmp/lust-target/release/lust run benchmarks/json_parse_loop.lust 5000
/tmp/lust-target/release/lust run benchmarks/map_grouping.lust 20000
```

If you want phase timing too:

```bash
env LUST_PROFILE=1 /tmp/lust-target/release/lust run benchmarks/lustgex_capture_scan.lust 20000
```

## Output Shape

Each benchmark prints a tiny summary:

- benchmark name
- iterations / rows
- a checksum or count

The checksum is there to keep the benchmark honest and make sure the work
actually happened.
