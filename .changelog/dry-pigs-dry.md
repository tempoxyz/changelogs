---
changelogs: patch
---

Updated the CI release workflow to pass `--locked` to `cargo install`, ensuring the build uses the committed `Cargo.lock` file for reproducible installs.
