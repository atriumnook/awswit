# Fix: テストコンパイルエラー — `credentials_file_path` フィールド不足

## Context
`AutoRefreshProfile` 構造体に `credentials_file_path: Option<String>` フィールドが追加されたが（`src/autorefresh/daemon.rs:38`）、テストコード内の構造体初期化が3箇所で更新されていない。`cargo test` がコンパイルエラーで全テスト失敗するため、mainマージ前に修正必須。

## 修正箇所

各テストの `AutoRefreshProfile { ... }` 初期化に `credentials_file_path: None,` を1行追加する。

| # | ファイル | 行 | テスト名 |
|---|---------|-----|---------|
| 1 | `src/autorefresh/daemon.rs` | 685 | `auto_refresh_profile_does_not_serialize_secrets` |
| 2 | `src/autorefresh/runner.rs` | 784 | `refresh_profile_rejects_disallowed_command` |
| 3 | `src/autorefresh/runner.rs` | 803 | `refresh_profile_rejects_path_traversal` |

## 検証
```bash
cargo test
cargo clippy -- -D warnings
```
両方がエラーゼロで通ること。
