---
changelogs: patch
---

Rust releases now update each bumped workspace package's version in `Cargo.lock`, so the lock file no longer stays one version behind `Cargo.toml` and builds with `--locked` pass on release commits.
