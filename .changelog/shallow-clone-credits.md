---
changelogs: patch
---

The GitHub Action now fetches full history before versioning, and `changelogs version` warns when it runs in a shallow clone. In a shallow clone every changelog entry was credited to the newest commit, its pull request and its author.
