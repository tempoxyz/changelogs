---
changelogs: patch
---

Added support for unified versioning in root changelog format by implicitly treating all workspace packages as a fixed group, merging duplicate version headings and deduplicating changelog entries. Added Rust workspace version inheritance support for reading and writing versions via `version.workspace = true`.
