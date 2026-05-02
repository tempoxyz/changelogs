---
changelogs: patch
---

Two release-mode fixes:

- **Go ecosystem.** `is_published` now treats `v0.0.0` as already published, so a Go module with no prior `vX.Y.Z` tag and no staged changelog entries does not get bootstrap-tagged on every push to the release branch. The previous behavior unconditionally created and pushed a `v0.0.0` tag the first time the release workflow ran on an integrated repo.
- **GitHub Action.** The "Create GitHub releases" step's previous-tag lookup uses `git tag --list … | grep -Fxv -- "$tag" | head -1`. `grep -Fxv` exits 1 when every input line matches the excluded pattern (i.e. when the only tag is `$tag` itself), and under `set -e -o pipefail` that aborted the whole step before any release was created. Both occurrences are now guarded with `{ grep -Fxv … || true; }`, so the lookup degrades to "no previous tag" instead of failing the step.
