---
changelogs: patch
---

Added Go module ecosystem support. Discovers single-module Go projects via `go.mod`, persists the bumped version inline as a `// changelogs:version X.Y.Z` comment between the version and publish CI runs, edits `require` blocks for dependency updates, queries `proxy.golang.org` for published-version checks, and emits `vX.Y.Z` git tags. Auto-detection (`go.mod` at the repository root) and the `go` / `golang` ecosystem aliases are wired through.
