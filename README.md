# awswit

awswit is an interactive AWS profile switcher with fuzzy search and frecency sorting.

[![CI](https://github.com/atnook/awswit/workflows/CI/badge.svg)](https://github.com/atnook/awswit/actions)
[![Crates.io](https://img.shields.io/crates/v/awswit.svg)](https://crates.io/crates/awswit)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[日本語](README_ja.md)

<!-- TODO: Add docs/demo.gif screenshot/recording of the TUI in action -->

## Quick Start

### Install

```bash
cargo install awswit
```

### Shell Setup

<details open>
<summary>Bash / Zsh</summary>

```bash
# ~/.bashrc or ~/.zshrc
eval "$(awswit init bash)"   # or zsh
```

</details>

<details>
<summary>Fish</summary>

```fish
# ~/.config/fish/config.fish
awswit init fish | source
```

</details>

<details>
<summary>PowerShell</summary>

```powershell
# $PROFILE
awswit init powershell | Invoke-Expression
```

</details>

### Usage

```bash
awswit                  # interactive picker
awswit prod             # switch directly
awswit -l               # list profiles
awswit -u               # unset
```

Pick a profile, hit Enter. `AWS_PROFILE` is set in your current shell.

## Features

- **Fuzzy search** — type to filter, matches appear instantly
- **Frecency sorting** — frequently and recently used profiles are ranked higher
- **Favorites** — pin profiles to the top with `*`
- **Preview panel** — press `Ctrl+P` to see region, account ID, role ARN, and more
- **fzf integration** — pass `--fzf` or set `AWSWIT_USE_FZF=1`
- **No credential handling** — awswit sets `AWS_PROFILE` and leaves authentication to the AWS SDK, SSO, or aws-vault
- **Single binary** — written in Rust with no runtime dependencies

## How It Works

awswit reads `~/.aws/config`, shows you a picker, and sets these in your shell:

```
AWS_PROFILE=prod
AWS_DEFAULT_PROFILE=prod
AWS_REGION=ap-northeast-1       # if the profile defines a region
AWS_DEFAULT_REGION=ap-northeast-1
AWSWIT_PROFILE=prod
```

The AWS SDK resolves credentials based on the profile configuration — IAM keys, SSO, role assumption, `credential_process`, or anything else. awswit does not touch credentials.

## Keybindings

| Key | Action |
|-----|--------|
| Type | Fuzzy search |
| `Enter` | Select profile |
| `↑`/`↓` or `Ctrl+k`/`Ctrl+j` | Navigate |
| `*` or `Ctrl+F` | Toggle favorite |
| `Ctrl+P` | Toggle preview panel |
| `Esc` / `Ctrl+C` | Cancel |

## CLI Reference

```
awswit [PROFILE]           Switch to a profile (TUI if no name given)
awswit init <shell>        Print shell integration script
awswit completions <shell> Generate tab-completion script
```

| Flag | Description |
|------|-------------|
| `-v, --version` | Print version |
| `-s, --show-commands` | Print export commands instead of setting them |
| `-u, --unset` | Unset all AWS environment variables |
| `-l, --list-profiles` | List profiles (`-l more` for details) |
| `-n, --no-interactive` | Skip TUI, resolve profile by name or `$AWS_PROFILE` |
| `--fzf` | Use external fzf |
| `--region <region>` | Override region |
| `--config-file <path>` | Path to AWS config file |
| `--info` | INFO-level logs |
| `--debug` | DEBUG-level logs |

## Configuration

`~/.awswit/config.toml` (all optional):

```toml
fuzzy-match = true           # Fuzzy profile name matching (default: true)
colors = true                # Colored output (default: true on Linux/macOS)
region = "ap-northeast-1"    # Default region override
```

Unknown keys are rejected on load, so typos are caught immediately.

<details>
<summary>Shell Completions</summary>

```bash
awswit completions bash > /etc/bash_completion.d/awswit
awswit completions zsh > ~/.zfunc/_awswit
awswit completions fish > ~/.config/fish/completions/awswit.fish
```

</details>

## FAQ

**Why not just `export AWS_PROFILE=foo`?**

You can. awswit is for when you have 10+ profiles and typing exact names gets tedious. With fuzzy search, favorites, and frecency, the right profile is usually one or two keystrokes away.

**How is this different from awsume / aws-vault?**

They manage credentials — STS calls, token caching, MFA. awswit does none of that. It sets `AWS_PROFILE` and lets the SDK handle authentication:

- No background processes
- No token files to debug
- Works with any auth method, including ones that did not exist when awswit was written

If you already use `aws sso login` or aws-vault, awswit is the missing piece — a fast way to pick which profile is active.

## License

MIT — see [LICENSE](LICENSE).
