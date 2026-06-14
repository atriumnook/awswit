# awswit

awswit is an interactive AWS profile switcher with fuzzy search, frecency
sorting, and first-class scripting support — built so picking the right
profile out of dozens never gets in the way.

[![CI](https://github.com/atriumnook/awswit/workflows/CI/badge.svg)](https://github.com/atriumnook/awswit/actions)
[![Crates.io](https://img.shields.io/crates/v/awswit.svg)](https://crates.io/crates/awswit)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[日本語](README_ja.md)

## Quick Start

### Install

Pre-built binaries are attached to each [release](https://github.com/atriumnook/awswit/releases).

```bash
# Linux / macOS
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/atriumnook/awswit/releases/latest/download/awswit-installer.sh | sh

# Windows (PowerShell)
irm https://github.com/atriumnook/awswit/releases/latest/download/awswit-installer.ps1 | iex

# From source
cargo install awswit
```

### Shell setup

Add the snippet for your shell to your rc file. It defines an `awswit`
shell function that wraps the binary so a selection actually updates the
current shell's `AWS_PROFILE`.

```bash
# ~/.bashrc
eval "$(awswit init bash)"

# ~/.zshrc
eval "$(awswit init zsh)"

# ~/.config/fish/config.fish
awswit init fish | source

# $PROFILE (PowerShell)
awswit init powershell | Invoke-Expression
```

### Usage at a glance

```text
awswit                       # interactive picker — sets AWS_PROFILE in the shell
awswit prod                  # switch directly
awswit pick                  # interactive picker — prints selection to stdout
awswit which                 # what's active right now?
awswit doctor                # audit ~/.aws/config and SSO tokens
awswit exec prod -- aws s3 ls   # one-off command, no shell mutation
awswit -l                    # list profiles (TSV if piped, table on TTY)
awswit -l --json             # list profiles as JSON
awswit prompt                # current profile, for shell-prompt embedding
awswit -u                    # unset AWS_PROFILE and AWS_REGION
```

In the TUI: type to fuzzy-filter, `↑/↓` (or `Ctrl-k`/`Ctrl-j`) to
navigate, `Enter` to select, `*` or `Ctrl-F` to toggle favorite,
`Ctrl-P` to toggle the preview panel, `Ctrl-A`/`Ctrl-E` to jump to
start/end of the query, `Ctrl-W` to delete the previous word, `Esc` to
cancel.

## What awswit does, exactly

When you pick a profile, awswit emits a small block of `export` and
`unset` statements that the shell function evaluates. For `awswit prod`
against a profile with region `ap-northeast-1`:

```sh
export AWS_PROFILE='prod'
unset AWS_DEFAULT_PROFILE
export AWS_REGION='ap-northeast-1'
unset AWS_DEFAULT_REGION
unset AWS_ACCESS_KEY_ID
unset AWS_SECRET_ACCESS_KEY
unset AWS_SESSION_TOKEN
unset AWS_SECURITY_TOKEN
unset AWS_CREDENTIAL_EXPIRATION
```

We **set** the modern `AWS_PROFILE` / `AWS_REGION` variables, and
explicitly **clear** the legacy `AWS_DEFAULT_*` fallbacks so a stale
value left over from a CI image, a corporate dotfile, or a previous
`aws configure` run can't silently re-route credentials to the wrong
account. If the profile has no region, `AWS_REGION` is also unset and
the SDK falls back to its own resolution.

We also **clear inherited credential variables** (`AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, and friends). These outrank
`AWS_PROFILE` in the SDK resolution chain, so without clearing them a
left-over key or session token from a previous `aws sso login`,
aws-vault subshell, or awsume run would silently win — `awswit prod`
would set the profile yet `aws s3 ls` would keep hitting the old
account. Clearing them makes awswit a drop-in switcher: **after
`awswit prod`, plain `aws ...` just works as the prod profile.**

The AWS SDK resolves credentials from there — IAM keys, SSO, role
assumption, `credential_process`, anything you've already configured.
**awswit never reads, fetches, or stores credential material itself** —
it only removes conflicting variables and lets the SDK do the rest.

## Comparison

| | **awswit** | **aws-vault** | **awsume** | `export AWS_PROFILE` + `fzf` |
|---|---|---|---|---|
| Pick a profile interactively | ✅ TUI + fzf | — (`list` is read-only) | ✅ TUI | ✅ |
| Fuzzy matching | ✅ (built-in) | — | ✅ (built-in) | ✅ |
| Frecency sorting + favorites | ✅ | — | — | — |
| Auto-suggest on typo | ✅ "did you mean…?" | — | — | — |
| TUI preview (region / account / role ARN / MFA / last-used) | ✅ | — | — | — |
| One-off `exec` without mutating shell | ✅ `awswit exec` | ✅ `aws-vault exec` | ✅ | — |
| `which` (current profile + SSO token status) | ✅ | partial | partial | — |
| `doctor` (broken source chains, expired SSO tokens, missing MFA) | ✅ | — | — | — |
| Machine-readable list output (TSV / JSON / names-only) | ✅ | partial | — | n/a |
| Shell prompt integration helper | ✅ `awswit prompt` | — | — | — |
| Manages credentials / STS / MFA | — by design | ✅ | ✅ | — |
| Token caching, auto-refresh | — | ✅ | ✅ | — |
| Single static binary | ✅ Rust | ✅ Go | — Python | n/a |

awswit is the **profile switcher**: best-in-class at picking, inspecting,
and scripting profiles. It doesn't do STS / MFA / token caching — that's
the AWS SDK's job, or aws-vault's, or aws-sso-login's. Use them together:

```bash
# Auth (once per day): aws-vault or aws sso login owns STS.
aws sso login --profile main

# Switch (dozens of times per day): awswit owns the picker.
awswit prod
aws s3 ls

# One-off under a different profile, without mutating the shell:
awswit exec staging -- aws s3 ls s3://bucket
```

## CLI reference

### Subcommands

```text
awswit [PROFILE]                 pick a profile (TUI if no PROFILE and stdout is a tty)
awswit pick                      open the picker, print selection to stdout (no shell mutation)
awswit exec PROFILE -- CMD ...   run CMD with AWS_PROFILE set, without touching the shell
awswit which                     show current profile + SSO expiry / aws-vault status
awswit doctor                    audit ~/.aws/config + SSO cache; exits non-zero on errors
awswit prompt [--format F]       print current profile for shell-prompt embedding
awswit init <shell>              print the shell integration snippet
awswit completions <shell>       print a tab-completion script
```

### Top-level flags

| Flag                     | Description                                                                |
| ------------------------ | -------------------------------------------------------------------------- |
| `-v`, `--version`        | print version                                                              |
| `-s`, `--shell-export`   | emit only `set`/`unset` lines (for `eval`) — used by the shell wrapper     |
| `-u`, `--unset`          | unset every awswit-managed variable                                        |
| `-l`, `--list`           | list profiles (TSV if piped, table if tty)                                 |
| `--json`                 | with `-l`, emit JSON                                                       |
| `--names-only`           | with `-l`, emit one profile name per line — fast path for completion       |
| `-n`, `--no-interactive` | skip the picker; resolve PROFILE by name or `$AWS_PROFILE`                 |
| `--fzf`                  | use external `fzf` instead of the built-in TUI                             |
| `--region <REGION>`      | override the region                                                        |
| `--config-file <PATH>`   | path to the AWS config file                                                |
| `--verbose`              | INFO logging                                                               |
| `--debug`                | DEBUG logging                                                              |

## Environment variables

| Variable                      | Effect                                                                  |
| ----------------------------- | ----------------------------------------------------------------------- |
| `AWS_CONFIG_FILE`             | overrides `~/.aws/config`                                               |
| `AWS_SHARED_CREDENTIALS_FILE` | overrides `~/.aws/credentials`                                          |
| `AWS_PROFILE`                 | default for `-n` when no PROFILE argument is given                      |
| `AWS_VAULT`                   | if set, awswit warns that you're inside an aws-vault session            |
| `NO_COLOR`                    | disables color (de-facto standard)                                      |
| `AWSWIT_NO_FUZZY`             | disable fuzzy matching — require an exact profile-name match            |
| `AWSWIT_USE_FZF`              | use external `fzf` (same as `--fzf`)                                    |
| `AWSWIT_FZF_OPTS`             | extra options for `fzf` (dangerous options are stripped)                |
| `AWSWIT_SHELL`                | force a shell flavor (`bash` / `zsh` / `fish` / `powershell`)           |
| `AWSWIT_SSO_CACHE_DIR`        | override the AWS SSO token cache dir (default `~/.aws/sso/cache`)       |
| `XDG_DATA_HOME`               | overrides where history lives (default `~/.local/share/awswit/`)        |

awswit deliberately does **not** read a config file of its own — every
runtime knob is a CLI flag or environment variable.

## Shell prompt integration

`awswit prompt` is designed for embedding in a shell prompt — exit 0,
no trailing newline, format string under your control.

```bash
# bash — example PS1 with the profile in cyan brackets
PS1='\[\033[36m\]$(awswit prompt --format "[%s] " --default "")\[\033[0m\]\u@\h:\w\$ '

# zsh — drop it in a precmd or right-side prompt
RPROMPT='%F{cyan}$(awswit prompt --format "(%s)" --default "")%f'

# starship.toml — custom command segment
[custom.awswit]
command = "awswit prompt --format '☁ {}'"
when = '[ -n "$AWS_PROFILE" ]'
```

(`%s` and `{}` are interchangeable in `--format`.)

## Health checks: `awswit doctor`

`awswit doctor` inspects `~/.aws/config` and the AWS CLI's SSO token
cache and reports the issues that bite people in practice:

```text
ERROR [foo-role] source_profile = 'gone' references a profile that does not exist
ERROR [sso-prod] SSO token expired at 2026-05-12 03:14 UTC — run `aws sso login --profile sso-prod`
warn  [no-mfa] role profile has neither source_profile nor credential_source
warn  [legacy] mfa_serial = 'foo' does not look like an IAM MFA ARN

awswit doctor: 2 error(s), 2 warning(s)
```

Exit status: 0 if clean, 1 if any errors.

## Files

- `$XDG_DATA_HOME/awswit/history.json` — usage history and favorites.
  Migrated automatically from the legacy `~/.awswit/history.json` on
  first run after upgrading from `< 0.1.0`. Not a secret; written with
  your umask.

## Scripting examples

```bash
# Print every profile name, one per line:
awswit -l | cut -f1

# Switch to the first profile whose name contains "staging":
awswit -n "$(awswit -l | awk -F'\t' '/staging/ {print $1; exit}')"

# Inspect profile metadata as JSON:
awswit -l --json | jq '.[] | select(.type == "Role")'

# Run a shell with a temporary profile (good for ad-hoc tasks):
awswit exec prod -- bash

# Compose pick with other tools — no shell mutation.
# `pick` exits 130 on cancel, so guard with || to avoid running with "":
P=$(awswit pick) || exit 130
aws --profile "$P" sts get-caller-identity

# Wire into CI: fail the job if any profile has an expired SSO token.
awswit doctor
```

## License

MIT — see [LICENSE](LICENSE).
