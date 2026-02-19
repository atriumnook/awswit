# awswit

Fast AWS profile switcher with interactive TUI, written in Rust.

## Features

- **Fast**: < 50ms startup time, Rust-native performance
- **Interactive**: fzf-style fuzzy profile picker with preview panel
- **Smart**: Usage history tracking, favorites, fuzzy matching
- **Secure**: Credential caching with proper file permissions (0600)
- **Complete**: AssumeRole, MFA, role chaining, credential_process support
- **Shell support**: Bash, Zsh, Fish, PowerShell

## Installation

### From source

```bash
cargo install --path .
```

### Shell setup

Add to your shell configuration. This installs both the shell wrapper function
(needed for `eval`-based credential export) and tab completions:

**Bash** (`~/.bashrc`):
```bash
eval "$(command awswit --completion bash)"
```

**Zsh** (`~/.zshrc`):
```bash
eval "$(command awswit --completion zsh)"
```

**Fish** (`~/.config/fish/config.fish`):
```fish
command awswit --completion fish | source
```

**PowerShell** (`$PROFILE`):
```powershell
Invoke-Expression (& awswit --completion powershell)
```

## Usage

```bash
# Interactive profile picker
awswit

# Switch to a specific profile
awswit my-profile

# Force credential refresh
awswit -r my-profile

# Show export commands without executing
awswit -s my-profile

# Unset AWS environment variables
awswit -u

# List all profiles
awswit -l

# List profiles with details
awswit -l more

# Toggle favorite
awswit --favorite my-profile

# Assume a role directly
awswit --role-arn arn:aws:iam::123456789012:role/MyRole --source-profile default

# Use as credential_process
awswit --credential-process my-profile

# Auto-refresh credentials
awswit -a my-profile

# Kill auto-refresh daemon
awswit -k
```

## Configuration

### AWS Configuration

awswit reads standard AWS configuration files:
- `~/.aws/config`
- `~/.aws/credentials`

### awswit Configuration

Optional configuration at `~/.awswit/config.yaml`:

```yaml
colors: true
fuzzy-match: true
role-duration: 3600
region: us-east-1
role-session-name: awswit-session
```

## Keyboard Shortcuts (Interactive Mode)

| Key | Action |
|-----|--------|
| `Enter` | Select profile |
| `Esc` / `Ctrl+C` | Cancel |
| `↑` / `↓` | Navigate |
| `Ctrl+F` | Toggle favorite |
| Type | Filter profiles |

## License

Apache-2.0
