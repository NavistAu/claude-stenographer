# Contributing

Outside contributions are welcome. This document covers the branch model, how to build and test locally, the documentation style this repo uses, and what bar a new configuration flag needs to clear.

## Branch model

- Feature branches (`feat/…`, `fix/…`, `docs/…`) start from `develop` and target `develop` in their pull request.
- `develop` → `main` is the only path to `main`.
- Every push to `main` is a release: CI builds and publishes binaries, and `plugin/.claude-plugin/plugin.json`'s version is what ships. Don't open a PR directly against `main`.

If you don't have push access, fork the repo, branch from `develop` on your fork, and open your pull request back against this repo's `develop` branch.

## Build and test

```sh
mise install     # pins rust 1.95, see mise.toml
cargo build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

`cargo test` runs both the unit tests and `tests/integration.rs`, the authoritative suite that builds the real binary and drives it as a subprocess. The integration suite can take about 2 minutes on a first run.

CI runs the same three checks (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`) on every PR, so running them locally first catches failures before CI does.

## Documentation style: two registers

This README deliberately mixes two different registers, and a PR that touches docs needs to preserve the split rather than "fixing" it toward one style:

- **Controlled, terse language** in the Terms, Requirements, Installation, Configuration reference, Error and log messages, and Troubleshooting sections. These are reference material — a flag's default, an error string's meaning, a step in an install sequence. Precision and scannability matter more than voice here.
- **Ordinary prose** everywhere else — the opening, "Why it's worth having," "When to use it," "Design constraints." These sections explain motivation and trade-offs, which reads better as normal writing than as a table.

If your PR touches README.md, match the register of the section you're editing. Don't convert prose sections into tables, and don't loosen the terse sections into narrative.

## Adding configuration surface

New flags and options are not free — every one is a thing to document, test, and support indefinitely. Before adding one, have a concrete use case the existing surface can't already express (an existing flag, an existing default, or a combination of the two). "Might be useful" or "matches what other tools do" isn't sufficient by itself. If you're not sure whether your use case clears this bar, open an issue describing it before writing the PR.

## CHANGELOG discipline

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add your entry to `CHANGELOG.md` as part of the same PR that makes the change — not deferred to release time. If your change affects what gets indexed or how search results are scoped (the escalation ladder, project-boundary resolution, dense/lexical fusion), call that out explicitly in the entry regardless of how small it is; those are the changes most likely to silently affect what a query can and can't find.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Reports go to `foss+conduct@navist.com.au`.
