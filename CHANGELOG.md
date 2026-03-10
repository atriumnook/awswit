# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
