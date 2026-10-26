---
changelogs: patch
---

Fixed release planning so fixed groups always end on one version (including after dependent bumps), prerelease bumps start a new rc cycle when the change outgrows the current one, and Cargo dependency rewrites cover renamed and target-specific dependencies while keeping requirement operators. `changelogs version` now refuses to run when an entry references an unknown package or has an invalid bump value, and changelog links and authors are resolved from the workspace repository, with GitHub handles taken from noreply emails.
