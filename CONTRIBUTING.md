# Contributing to awswit

Thank you for your interest in contributing to awswit! This document provides guidelines and information for contributors.

## Development Setup

### Prerequisites

- Rust 1.75 or later (latest stable recommended)
- Git

### Building

```bash
# Clone the repository
git clone https://github.com/yourusername/awswit.git
cd awswit

# Build in debug mode
cargo build

# Build in release mode
cargo build --release

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run -- my-profile
```

### Code Style

We use `rustfmt` and `clippy` to maintain code quality:

```bash
# Format code
cargo fmt

# Run linter
cargo clippy --all-targets --all-features -- -D warnings
```

### Testing

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name
```

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Ensure tests pass (`cargo test`)
5. Ensure code is formatted (`cargo fmt`)
6. Ensure clippy passes (`cargo clippy`)
7. Commit your changes (`git commit -m 'Add amazing feature'`)
8. Push to your branch (`git push origin feature/amazing-feature`)
9. Open a Pull Request

### Commit Messages

We follow conventional commits:

- `feat:` New feature
- `fix:` Bug fix
- `docs:` Documentation changes
- `style:` Code style changes (formatting)
- `refactor:` Code refactoring
- `test:` Adding or updating tests
- `chore:` Maintenance tasks

Example: `feat: add SSO profile support`

## Project Structure

```
awswit/
├── src/
│   ├── main.rs           # CLI entry point
│   ├── cli/              # Command-line argument parsing
│   ├── config/           # Configuration file handling
│   ├── profile/          # AWS profile parsing and resolution
│   ├── aws/              # AWS SDK operations
│   ├── cache/            # Credential caching
│   ├── shell/            # Shell export and completion
│   ├── tui/              # Terminal UI (picker, spinner)
│   ├── history/          # Usage history tracking
│   ├── autorefresh/      # Auto-refresh daemon
│   ├── utils/            # Utilities (fuzzy matching)
│   └── error.rs          # Error types
├── shell_scripts/        # Shell wrapper scripts
└── tests/                # Integration tests
```

## Adding New Features

### Adding a New CLI Flag

1. Add the flag to `src/cli/args.rs`
2. Handle the flag in `src/main.rs`
3. Add tests
4. Update documentation

### Adding a New Profile Type

1. Update `src/profile/types.rs` with new fields
2. Update `src/config/aws_files.rs` for parsing
3. Update `src/profile/resolver.rs` for credential resolution
4. Add tests

## Questions?

Feel free to open an issue for questions or discussions!
