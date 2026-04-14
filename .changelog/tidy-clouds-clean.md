---
changelogs: patch
---

Fixed changelog directory lookup to support hyphenated package names by stripping the first prefix segment as a fallback (e.g., `tempo-alloy` -> `alloy`). Also improved release notes extraction to match both backtick-wrapped tag headings and plain version headings.
