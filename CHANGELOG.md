# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed - v2 Architecture Overhaul
- **awswit は認証に関与しなくなりました。** プロファイル切り替え UX に特化し、認証は AWS SDK / aws-vault / SSO 等に委ねます。
- プロファイル選択時の出力が `AWS_PROFILE=<name>` ベースに変更（クレデンシャル直接出力を廃止）
- 単一バイナリ化（`autoawswit`, `awswit-autocomplete` を廃止）
- tokio (async runtime) を完全削除し、同期実行に変更
- MSRV を 1.94 に引き上げ
- Edition を 2024 に更新

### Removed
- `exec` サブコマンド
- STS AssumeRole / GetSessionToken の直接呼び出し
- バックグラウンドデーモン（auto-refresh）機構
- credentials ファイル書き換え・ファイルロック機構
- ディスクキャッシュ機構
- 以下の CLI フラグ: `-a/--auto-refresh`, `-k/--kill-refresher/--kill`, `-r/--refresh`, `--refresh-autocomplete`, `--role-arn`, `--source-profile`, `--external-id`, `--mfa-token`, `--session-name`, `--role-duration`, `--credentials-file`
- 以下の依存関係: `aws-config`, `aws-sdk-sts`, `aws-credential-types`, `aws-types`, `tokio`, `async-trait`, `fd-lock`, `humantime`, `dialoguer`, `libc`
- シェルスクリプトから `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AWSWIT_EXPIRATION` の管理を削除
- awswit 設定から `role-duration`, `session-token-duration`, `role-session-name` を削除
- Legacy YAML config (`config.yaml`) サポートを削除

### Added
- `awswit completions <shell>` subcommand for static shell completion generation
- `--fzf` flag and `AWSWIT_USE_FZF` env var for external fzf-based profile selection
- Frecency-based profile sorting (frequency + recency) in picker and fzf mode
