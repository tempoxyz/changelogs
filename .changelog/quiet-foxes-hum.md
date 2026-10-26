---
changelogs: patch
---

Renamed the `publish --tag` option to `--registry`, which is what it sets; `--tag` keeps working as a hidden alias. `version` now looks up each changelog entry's commit and pull request once instead of once per package it releases, so large workspaces render changelogs with far fewer git calls.
