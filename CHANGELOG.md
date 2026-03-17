# Changelog

## 0.6.2 (2026-03-17)

### Patch Changes

- Fixed config template placing `ignore` after `[changelog]` header causing it to be silently dropped. Respect `publish = false` in Cargo.toml by skipping unpublishable crates. Filter ignored packages during `publish` command. Added `SkipReason` enum to distinguish skip reasons in output. (by @Kartik, [#72](https://github.com/tempoxyz/changelogs/pull/72))

## 0.6.1 (2026-02-10)

### Patch Changes

- Truncated AI diff to 32KB to prevent "Prompt is too long" errors with AI commands that have smaller context windows. (by @jxom, [c3a9576](https://github.com/wevm/changelogs/commit/c3a9576))

## 0.6.0 (2026-02-04)

### Minor Changes

- Replaced auto-generate workflow with check action that comments on PRs with changelog status and optional AI-generated previews. (by @jxom, [#37](https://github.com/wevm/changelogs-rs/pull/37))

### Patch Changes

- Fixed PR number detection to find the *first* merge commit that brought a changelog file into the branch, rather than the most recent one. This prevents incorrect PR attribution on release branches where later merges could incorrectly claim authorship. (by @jxom, [#40](https://github.com/wevm/changelogs-rs/pull/40))

## `changelogs@0.5.2`

### Patch Changes

- Fixed per-crate changelog format to include package and version header in individual CHANGELOG.md files. Updated GitHub Action to look for changelogs in per-crate directories before falling back to root CHANGELOG.md. (by @jxom, [#33](https://github.com/wevm/changelogs/pull/33))

## `changelogs@0.5.1`

### Patch Changes

- Fixed token validation to check for empty strings instead of just missing environment variables. (by @jxom, [#31](https://github.com/wevm/changelogs/pull/31))

## `changelogs@0.5.0`

### Minor Changes

- Added support for creating git tags without registry tokens, allowing the tool to be used for version management even when package publishing is not configured. (by @jxom, [#29](https://github.com/wevm/changelogs/pull/29))

## `changelogs@0.4.2`

### Patch Changes

- Fixed release mode to run without requiring registry tokens, allowing tag creation and GitHub releases for all projects. (by @BrendanRyan, [#27](https://github.com/wevm/changelogs/pull/27))

## `changelogs@0.4.1`

### Patch Changes

- Fixed release notes formatting by adding blank line before GitHub extras section. (by @jxom, [#24](https://github.com/wevm/changelogs-rs/pull/24))
- Fixed error message when ecosystem is not detected to provide clear instructions with the --ecosystem flag, and added -e shorthand for the ecosystem argument. (by @jxom, [#24](https://github.com/wevm/changelogs-rs/pull/24))

## `changelogs@0.4.0`

### Minor Changes

- Added `changelogs up` command for self-update functionality and improved install script with automatic PATH configuration for multiple shells. (by @jxom, [#20](https://github.com/wevm/changelogs-rs/pull/20))

## `changelogs@0.3.0`

### Minor Changes

- Added Python ecosystem support with both PEP 621 and Poetry formats. (by @BrendanRyan, [#17](https://github.com/wevm/changelogs-rs/pull/17))

## `changelogs@0.2.1`

### Patch Changes

- Fixed action. (by @jxom, [411d91e](https://github.com/wevm/changelogs-rs/commit/411d91e))

## `changelogs@0.2.0`

### Minor Changes

- Added automatic installation of changelogs binary in GitHub Actions with caching support, and simplified action inputs by removing customizable version/publish commands in favor of hardcoded changelogs commands. (by @jxom, [#10](https://github.com/wevm/changelogs-rs/pull/10))

## `changelogs@0.1.1`

### Patch Changes

- Fixed GitHub releases to use changelog content from CHANGELOG.md instead of auto-generated notes. (by @jxom, [#7](https://github.com/wevm/changelogs-rs/pull/7))

## `changelogs@0.1.0`

### Minor Changes

- Added AI-assisted changelog generation. Users can now generate changelog entries from git diffs using the `--ai` flag with commands like `changelogs add --ai "claude -p"`. Includes a GitHub Action for automated PR changelog generation and configuration options for custom AI commands and instructions. (by @jxom, [e8740e8](https://github.com/wevm/changelogs-rs/commit/e8740e8))

### Patch Changes

- Updated README with improved workflow documentation, added mermaid diagram showing development and release process, documented AI changelog generation with GitHub Actions setup, and clarified the Release Candidate workflow. (by @jxom, [e8740e8](https://github.com/wevm/changelogs-rs/commit/e8740e8))

## `changelogs@0.0.1`

### Patch Changes

- Initial release (by @jxom, [59a7a96](https://github.com/wevm/changelogs-rs/commit/59a7a96))

