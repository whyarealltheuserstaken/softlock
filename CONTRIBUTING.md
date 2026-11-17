# Contributing

## Open-core boundary

This repository is the free Apache-2.0 core. Planned paid features are never added to this repository. That includes allowlists, rules, fleet management, remote triggers, and the command-line tool.

Do not stub, design, or document those features here. `docs/SPEC.md` records them as out of scope.

## Developer Certificate of Origin

No contributor agreement is required while the maintainer is the only author. A DCO sign-off (Signed-off-by) becomes required once outside pull requests start.

## Continuous integration

Pull requests run platform jobs only when shared crates, `Cargo.lock`, platform-specific trees, or workflow files change; pushes to `main` run the full matrix. Required checks aggregate into `ci ok` and `universal ok`, which pass when every needed job succeeded or was skipped.

## Tests

Behavior changes at the seams in `docs/SPEC.md` (Testing Decisions) are test-first: one failing seam test, then the code that makes it pass. `cargo test --workspace` runs seam 1. Seam 3 is the Windows probe harness in `crates/softlock-seam3`, run on `windows-2022` and `windows-2025`. `npm run test:e2e` in `crates/softlock-app` runs seam 4.

## Platforms

macOS and Linux are not implemented. Their backends compile, report every capability as unsupported, and the settings app shows the unsupported-platform message on those systems. Design notes for the later phases live in `docs/platforms/macos.md` and `docs/platforms/linux.md`.
