# awswit

awswit is an interactive AWS profile switcher with fuzzy search and frecency sorting.

[![CI](https://github.com/atriumnook/awswit/workflows/CI/badge.svg)](https://github.com/atriumnook/awswit/actions)
[![Crates.io](https://img.shields.io/crates/v/awswit.svg)](https://crates.io/crates/awswit)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[日本語](README_ja.md)

## Quick Start

### Install

Pre-built binaries are attached to each [release](https://github.com/atriumnook/awswit/releases).
To install the latest:

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

Pick the line for your shell and add it to your rc file. The snippet defines
an `awswit` shell function that wraps the binary so a selection actually
updates the current shell's `AWS_PROFILE`.

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

### Usage

```text
awswit              # open the TUI picker
awswit prod         # switch directly
awswit -l           # list profiles (TSV when piped, table in a terminal)
awswit -l --json    # list profiles as JSON
awswit -u           # unset AWS_PROFILE and AWS_REGION
```

In the TUI: type to fuzzy-filter, `↑/↓` (or `Ctrl-k`/`Ctrl-j`) to navigate,
`Enter` to select, `*` or `Ctrl-F` to toggle favorite, `Ctrl-P` to toggle
the preview panel, `Esc` to cancel.

## What awswit does, exactly

When you pick a profile, awswit emits two `export` (or `set`/`set -gx`)
statements that the shell function evaluates:

```sh
export AWS_PROFILE='prod'
export AWS_REGION='ap-northeast-1'
```

That's it. The AWS SDK resolves credentials from there — IAM keys, SSO,
role assumption, `credential_process`, anything you've already configured.
awswit never reads or stores credential material.

If the profile has no region, awswit emits `unset AWS_REGION` so the SDK
falls back to its own resolution.

## How it differs from neighbouring tools

- **aws-vault / aws-sso-login**: manage credentials (STS, token caching,
  MFA). awswit doesn't — it picks a profile name and lets these tools
  handle authentication.
- **`export AWS_PROFILE=...`**: works fine for a handful of profiles. awswit
  starts paying off when you have 10+ and don't want to type exact names.
- **`fzf` over `aws configure list-profiles`**: that's awswit minus the
  preview pane, the frecency ordering, favorites, and the cross-shell
  wrapper. If those features don't matter to you, the fzf one-liner is
  perfectly fine.

## CLI reference

```text
awswit [PROFILE]           pick a profile (TUI if no PROFILE and stdout is a tty)
awswit init <shell>        print the shell integration snippet
awswit completions <shell> print a static tab-completion script
```

| Flag                   | Description                                                            |
| ---------------------- | ---------------------------------------------------------------------- |
| `-v`, `--version`      | print version                                                          |
| `-s`, `--shell-export` | emit only `set`/`unset` lines (for `eval`) — used by the shell wrapper |
| `-u`, `--unset`        | unset every awswit-managed variable                                    |
| `-l`, `--list`         | list profiles (TSV if piped, table if tty)                             |
| `--json`               | with `-l`, emit JSON                                                   |
| `-n`, `--no-interactive` | skip the picker; resolve PROFILE by name or `$AWS_PROFILE`           |
| `--fzf`                | use external `fzf` instead of the built-in TUI                         |
| `--region <REGION>`    | override the region                                                    |
| `--config-file <PATH>` | path to the AWS config file                                            |
| `--verbose`            | INFO logging                                                           |
| `--debug`              | DEBUG logging                                                          |

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
| `XDG_DATA_HOME`               | overrides where history lives (default `~/.local/share/awswit/`)        |

awswit deliberately does **not** read a config file of its own — every
runtime knob is a CLI flag or environment variable.

## Files

- `$XDG_DATA_HOME/awswit/history.json` — usage history and favorites.
  Migrated automatically from the legacy `~/.awswit/history.json` on
  first run after upgrading from < 0.1.0. Not a secret; written with
  your umask.

## Scripting examples

```bash
# Print every profile name, one per line:
awswit -l | cut -f1

# Switch to the first profile whose name contains "staging":
awswit -n "$(awswit -l | awk -F'\t' '/staging/ {print $1; exit}')"

# Inspect profile metadata as JSON:
awswit -l --json | jq '.[] | select(.type == "Role")'
```

## License

MIT — see [LICENSE](LICENSE).
