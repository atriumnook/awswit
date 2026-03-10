# awswit

> **AWS Wit** - A fast, modern AWS profile switcher with interactive TUI

[![CI](https://github.com/yourusername/awswit/workflows/CI/badge.svg)](https://github.com/yourusername/awswit/actions)
[![Crates.io](https://img.shields.io/crates/v/awswit.svg)](https://crates.io/crates/awswit)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

```
┌─────────────────────────────────────────────────────────────┐
│  🔐 awswit                                        5/12     │
├─────────────────────────────────────────────────────────────┤
│  > dev                                                      │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ ★ prod-admin          Role   us-west-2   123456789012 ││
│  │ ★ dev-admin           Role   us-east-1   234567890123 ││
│  │   staging-readonly    Role   us-west-2   345678901234 ││
│  │   sandbox             User   us-east-1   -            ││
│  └─────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────┤
│  ↑↓ navigate  ⏎ select  ★ favorite  ^P preview  Esc cancel │
└─────────────────────────────────────────────────────────────┘
```

## ✨ Features

- **🔍 Interactive Profile Picker** - fzf-style fuzzy search with real-time preview
- **⭐ Favorites** - Pin frequently used profiles for quick access
- **📜 Usage History** - Recently used profiles shown first
- **🎨 Beautiful TUI** - Tokyo Night inspired color scheme
- **⏱️ Progress Indicators** - Spinners during AWS API calls
- **🚀 Instant Startup** - Written in Rust for speed
- **🔄 Auto-refresh** - Background credential renewal
- **🔗 Role Chaining** - Unlimited depth support
- **🔐 MFA Support** - Interactive prompts for MFA tokens
- **💾 Credential Caching** - Cache session tokens for up to 12 hours

## Installation

### From Cargo (Recommended)

```bash
cargo install awswit
```

### From Source

```bash
git clone https://github.com/yourusername/awswit.git
cd awswit
cargo build --release
sudo cp target/release/awswit /usr/local/bin/
```

### Shell Setup

Add to your `~/.bashrc` or `~/.zshrc`:

```bash
# awswit shell integration
alias awswit='source <(awswit --shell-init bash)'
```

Or use the provided shell wrapper:

```bash
cp shell_scripts/awswit.sh ~/.local/bin/
alias awswit='source ~/.local/bin/awswit.sh'
```

## Usage

### Interactive Mode (Default)

Simply run `awswit` without arguments to launch the interactive picker:

```bash
awswit
```

**Keyboard shortcuts:**

| Key | Action |
|-----|--------|
| `↑`/`↓` or `Ctrl+k`/`Ctrl+j` | Navigate profiles |
| `Enter` | Select profile |
| `*` or `Ctrl+F` | Toggle favorite |
| `Ctrl+P` | Toggle preview panel |
| `Esc` or `Ctrl+C` | Cancel |
| Type | Filter profiles (fuzzy search) |

### Direct Profile Selection

```bash
# Assume a specific profile
awswit my-profile

# Force refresh credentials
awswit my-profile -r

# Show export commands (without executing)
awswit my-profile -s

# Unset AWS credentials
awswit -u

# Disable interactive mode (for scripts)
awswit -n my-profile
```

### Profile Management

```bash
# List all profiles
awswit -l

# List with account IDs
awswit -l more
```

### Auto-refresh

```bash
# Enable auto-refresh for a profile
awswit my-profile -a

# Kill all auto-refresh processes
awswit -k
```

### Role ARN Shorthand

```bash
# Full ARN
awswit --role-arn arn:aws:iam::123456789012:role/MyRole

# Shorthand (account:role)
awswit --role-arn 123456789012:MyRole
```

## Configuration

awswit uses the standard AWS configuration files:

- `~/.aws/config` - Profile definitions
- `~/.aws/credentials` - Access keys

### awswit-specific Configuration

Create `~/.awswit/config.yaml`:

```yaml
# Enable fuzzy matching for profile names
fuzzy-match: true

# Enable colored output
colors: true

# Default session duration (seconds)
role-duration: 3600
```

## AWS Profile Examples

### Basic Role Profile

```ini
[profile dev]
role_arn = arn:aws:iam::123456789012:role/DevRole
source_profile = default
region = us-west-2
```

### Role with MFA

```ini
[profile prod-admin]
role_arn = arn:aws:iam::987654321098:role/AdminRole
source_profile = default
mfa_serial = arn:aws:iam::123456789012:mfa/myuser
region = us-east-1
```

### Role Chaining

```ini
[profile level1]
role_arn = arn:aws:iam::111111111111:role/Level1
source_profile = default

[profile level2]
role_arn = arn:aws:iam::222222222222:role/Level2
source_profile = level1

[profile level3]
role_arn = arn:aws:iam::333333333333:role/Level3
source_profile = level2
```

## Shell Support

| Shell | Status |
|-------|--------|
| Bash | ✅ Full support |
| Zsh | ✅ Full support |
| Fish | ✅ Full support |
| PowerShell | ✅ Full support |

## Comparison

| Feature | awswit | awsume | aws-vault |
|---------|--------|--------|-----------|
| Language | Rust | Python | Go |
| Startup time | ~10ms | ~200ms | ~50ms |
| Interactive picker | ✅ | ❌ | ❌ |
| Favorites | ✅ | ❌ | ❌ |
| Usage history | ✅ | ❌ | ❌ |
| MFA caching | ✅ | ✅ | ✅ |
| Role chaining | ✅ | ✅ | ✅ |
| Auto-refresh | ✅ | ✅ | ✅ |

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
