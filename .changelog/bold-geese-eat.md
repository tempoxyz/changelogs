---
changelogs: patch
---

Fixed config template placing `ignore` after `[changelog]` header causing it to be silently dropped. Respect `publish = false` in Cargo.toml by skipping unpublishable crates. Filter ignored packages during `publish` command. Added `SkipReason` enum to distinguish skip reasons in output.
