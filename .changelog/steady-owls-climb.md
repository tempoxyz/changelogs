---
changelogs: minor
---

Added a `draft-releases` input and `releaseTags` output to the release action so binaries can be attached before a release becomes immutable. The action now fails when a GitHub release cannot be created, pushes tags even if publishing fails partway, and the check action fails clearly when the base ref is missing, ignores deleted changelog entries, and builds the CLI from its own ref.
