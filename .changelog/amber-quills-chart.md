---
changelogs: minor
---

Added `--json` to `version` and `publish` for machine-readable output, and a `notes <tag>` command that prints a release's changelog section. The GitHub Action now uses them instead of parsing human output, so version PR bodies show per-crate changelogs and release notes come from the right CHANGELOG.md.
