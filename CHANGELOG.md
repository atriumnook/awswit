# Changelog

All notable changes to this project will be documented in this file.

## v0.0.2

_Released on 2026-04-27_

### Bug Fixes

- Stop validating credential file permissions and reading key material
- *(ci)* Rename artifacts to unique asset names before release upload

### CI

- Split release workflow into separate file

### Miscellaneous

- Collapse if-bodies into match arm guards (clippy 1.95)

## v0.0.1

_Released on 2026-03-15_

Initial release.

- Interactive TUI for AWS profile switching with fuzzy search and frecency sorting
- Non-interactive mode for scripting
- Shell completion support (bash, zsh, fish, powershell)
- Region override via flag, profile config, or awswit config
- fzf integration
