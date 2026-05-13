# Changelog

All notable changes to this project will be documented in this file.

## v0.1.0

_Released on 2026-05-13_

### Features

- New subcommand `awswit exec PROFILE -- CMD ARGS` runs a command with
  `AWS_PROFILE` (and optionally `AWS_REGION`) set in the child only,
  without mutating the parent shell. Exit code is propagated. Records
  the use for frecency.
- New subcommand `awswit which` reports the active profile with its
  region / account / role / SSO status, including SSO token expiry
  pulled from `~/.aws/sso/cache/*.json`.
- New subcommand `awswit doctor` audits `~/.aws/config` and the SSO
  cache for missing `source_profile` chains, expired or absent SSO
  tokens, malformed `mfa_serial`, and role profiles without any
  credential source. Exits non-zero on errors.
- New subcommand `awswit prompt --format … --default …` prints the
  current profile (or a fallback) for shell-prompt embedding. Both
  `{}` and `%s` work as the placeholder.
- "Did you mean …?" suggestions on `ProfileNotFound`, ranked by
  Levenshtein distance and filtered to candidates that plausibly
  match the typo.

### Bug fixes

- **TUI: favorite toggles no longer get silently overwritten.** The
  picker previously saved on each `*` press, then `main` re-saved a
  stale snapshot afterwards, reverting the change. The picker now
  returns the updated history; `main` is the single save site.
- TUI: toggling a favorite now re-sorts the list immediately so the
  promoted/demoted profile moves into its new position visibly. The
  cursor follows the toggled profile.
- TUI: handles the "no profiles configured" case with a helpful
  empty-state hint instead of a blank panel.

### TUI improvements

- Standard readline-style keys: `Ctrl-A` (start of line), `Ctrl-E`
  (end of line), `Ctrl-W` (delete word backward) in the search bar.
- Preview pane now wraps long role ARNs and SSO start URLs, shows
  the "last used" relative timestamp, and falls back to the SSO
  start URL row for SSO profiles.

### Shell wrapper

- The init snippet now passes informational subcommands and flags
  (`exec` / `which` / `doctor` / `prompt` / `init` / `completions` /
  `-l` / `-h` / `-v`) through to the binary directly, so their
  output isn't accidentally `eval`'d. Only the switch path uses
  `--shell-export`.

### Breaking changes

- `awswit` now manages only `AWS_PROFILE` and `AWS_REGION`. The legacy
  `AWS_DEFAULT_PROFILE`, `AWS_DEFAULT_REGION`, and the awswit-private
  `AWSWIT_PROFILE` variables are no longer set; the modern AWS SDK does
  not need them, and removing them avoids leaving stale state in your
  environment.
- The shell wrapper protocol (`AWSWIT_UNSET=1` sentinel, line-by-line
  key/value parsing in `bash.sh` / `zsh.sh` / `fish.fish` /
  `powershell.ps1`) is gone. The init snippet is now ~5 lines that does
  `eval "$(command awswit --shell-export "$@")"`. **You must re-source
  your shell init**:
  ```sh
  eval "$(awswit init bash)"     # or zsh / fish / powershell
  ```
- The `~/.awswit/config.toml` file is no longer read. The three knobs
  it offered (`colors`, `fuzzy-match`, `region`) are now handled by:
  - `NO_COLOR=1` (de-facto standard) to disable color in the TUI;
  - `AWSWIT_NO_FUZZY=1` to require exact profile-name matches;
  - `AWS_REGION` env var or `--region` flag for region overrides.
- History moved from `~/.awswit/history.json` to
  `$XDG_DATA_HOME/awswit/history.json` (default
  `~/.local/share/awswit/history.json`). An existing file in the legacy
  location is migrated automatically on first run.

### Features

- `awswit -l` is now machine-readable when piped (tab-separated, no
  headers), human-readable in a terminal, or JSON with `--json`. This
  makes `awswit -l | awk` / `awswit -l --json | jq` actually pleasant.
- `awswit` warns on stderr when `AWS_VAULT` is set, since changing
  `AWS_PROFILE` inside an aws-vault session leaves the vault session
  credentials stranded in the environment.
- Fuzzy match substitution now writes a clear warning to stderr noting
  which profile it actually resolved to, and how to opt out.

### Improvements

- Frecency now uses continuous exponential decay (half-life = 72h)
  instead of four step-function buckets, so ranking changes smoothly
  with time instead of jumping at the bucket boundaries.
- History file is written with the user's umask (no more forced
  `0o600`) — the content is profile names and timestamps, not secrets.
- The pretty-printed list pads the profile column to the widest name
  instead of truncating to 20 characters.

### Internal

- Drop `~/.awswit/config.toml` entirely; remove the `AwswitConfig`
  module.
- Delete `StatusLine` / `crossterm::style::Stylize` — status now goes
  through `tracing` to stderr.
- Significant simplification of `src/shell/export.rs` and
  `src/init/*` (the four init scripts collapse from ~170 LOC of
  bespoke parsing to ~30 LOC of `eval` wrappers).

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
