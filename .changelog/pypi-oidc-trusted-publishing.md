---
changelogs: minor
---

Add support for [PyPI Trusted Publishing](https://docs.pypi.org/trusted-publishers/)
(OIDC). When `pypi-token` is empty and the workflow has `id-token: write`,
the action mints a short-lived PyPI API token by exchanging the GitHub
OIDC ID token at PyPI's `_/oidc/mint-token` endpoint, removing the need
for a long-lived static API token. Existing static-token usage is
unchanged.
