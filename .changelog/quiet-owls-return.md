---
changelogs: patch
---

`changelogs publish` is now safe to rerun: existing release tags are skipped instead of failing, packages publish in dependency order (dependents of a failed package are skipped), and registry checks look up the exact version on crates.io, PyPI and the Go proxy. In root format the release tag is only created once every package published.
