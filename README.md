# sfc — Symfony Companion CLI

Rust CLI for post-build analysis and optimization of Symfony projects.

> **WIP** — This project is under active development.

## Install

```bash
cargo install --path .
```

## Commands

```bash
sfc analyze [PATH]              # Static audit of the prod build
sfc optimize [-O 1|2] [PATH]   # Strip dead/unreachable services from cache
sfc preload [PATH]              # Generate smart preload.php
sfc init [PATH]                 # Bootstrap sfc.toml
```

Run `sfc analyze` after `cache:warmup --env=prod`. Never touches source code.

## TODO

- [ ] `sfc report` — GitHub annotations output format
- [ ] Optimize level 3 — flatten decorator chains
- [ ] Reduce remaining ~12 false positives in dead service detection
- [ ] `src/` scanning in preload
- [ ] CI pipeline
- [ ] Release binaries
