# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `awswit exec <profile> -- <command>` subcommand for running commands with assumed credentials
- `awswit completions <shell>` subcommand for static shell completion generation
- `--fzf` flag and `AWSWIT_USE_FZF` env var for external fzf-based profile selection
- Frecency-based profile sorting (frequency + recency) in picker and fzf mode
- `nucleo-matcher` for improved fuzzy filtering in the interactive picker

### Changed
- Configuration format changed from YAML (`config.yaml`) to TOML (`config.toml`)
  - Existing `config.yaml` files are automatically loaded with a deprecation warning
  - To migrate: rename `~/.awswit/config.yaml` to `~/.awswit/config.toml` and convert syntax
- Replaced `colored`/`indicatif`/`console` with `crossterm` for TUI rendering
- Replaced `fs2` with `fd-lock` for file locking
- Replaced `uuid` dependency — session names now derived from profile names
- Optimized `tokio` features (removed `"full"`, using only required features)
- Internal: extracted `StsOperations` and `CredentialStore` traits for testability
- Internal: consolidated startup logic into `AppContext`
- Internal: unified `ProfileValidationError` into `AwswitError`

### Removed
- Dependencies: `uuid`, `serde_yml`, `colored`, `indicatif`, `console`, `fs2`

## [0.1.0] - 2024-XX-XX

### Added
- Initial release of awswit
- Interactive profile picker with fuzzy search (fzf-style)
- Profile favorites and usage history tracking
- AWS STS operations: AssumeRole, GetSessionToken
- MFA support with interactive prompts
- Role chaining with infinite depth support
- Credential caching (~/.awswit/cache/)
- Auto-refresh daemon for automatic credential renewal
- Shell integration: Bash, Zsh, Fish, PowerShell
- `credential_process` support
- `credential_source` support (Environment, Ec2InstanceMetadata, EcsContainer)
- Fuzzy profile name matching
- Rich TUI with progress spinners
- Tokyo Night inspired color theme

### Security
- Credentials are stored with restrictive file permissions (0600)
- Session tokens cached securely in ~/.awswit/cache/
