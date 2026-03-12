# awswit Multi-Perspective Code Review

## Summary

awswit は Rust 製の AWS プロファイル切り替え CLI ツール。58ファイル、約10,000行の新規コード。全体的にセキュリティ意識が高く（INI injection防止、atomic write、O_NOFOLLOW、credential値の検証）、堅実な実装。以下に9観点の統合レビュー結果を示す。

---

## Critical Issues

なし。

---

## Major Issues

### M1. デーモンが `AWS_SHARED_CREDENTIALS_FILE` を無視する
- **観点**: Security / Correctness / Maintainability
- **ファイル**: `src/utils/paths.rs:15-23`, `src/autorefresh/runner.rs:623`
- **問題**: `aws_credentials_path()` は常に `~/.aws/credentials` を返す。`main.rs` は `AWS_SHARED_CREDENTIALS_FILE` 環境変数を尊重するが、デーモンは常にデフォルトパスに書き込む。カスタムパスを使うユーザーの credential が更新されない。
- **修正案**: `AutoRefreshProfile` メタデータに credentials ファイルパスを保存し、デーモンがそれを使用する。

### M2. PowerShell ラッパーが空値を unset せず空文字列に設定する
- **観点**: API / Compatibility
- **ファイル**: `src/init/powershell.ps1:22-32`, `shell_scripts/awswit.ps1:23-33`
- **問題**: `AWS_REGION=`（空値）のとき、`$env:AWS_REGION = ""` が設定される。空文字列と未設定は異なり、AWS SDK がリージョンの解決に失敗する。
- **修正案**: 空値の場合は `Remove-Item Env:AWS_REGION -ErrorAction SilentlyContinue` を使用する。

### M3. `credential_process` が Windows で動作しない
- **観点**: API / Compatibility
- **ファイル**: `src/profile/resolver.rs:602-604`
- **問題**: `sh -c` をハードコードしており、Windows では `sh` が利用不可。
- **修正案**: `#[cfg(windows)]` で `cmd /c` を使用する。

### M4. ファイルロックが async ランタイムをブロックする
- **観点**: Concurrency / Reliability
- **ファイル**: `src/utils/fs.rs:104-131`
- **問題**: `lock_exclusive_with_timeout` が `std::thread::sleep` を使用し、tokio のエグゼキューターをブロックする。デーモンでロック競合時に最大1秒のブロックが発生。
- **修正案**: async 呼び出し元で `tokio::task::spawn_blocking` を使用する。

### M5. MFA チェーンの深さが1段階しか確認されない
- **観点**: Correctness
- **ファイル**: `src/main.rs:200-216`
- **問題**: auto-refresh の MFA 判定が直接の source profile しか確認しない。祖父プロファイルに MFA がある場合、デーモンが MFA プロンプトを出せずに失敗し続ける。
- **修正案**: `get_mfa_serial_for_chain` を使用してチェーン全体を確認する。

### M6. STS クライアントがテスト用に抽象化されていない
- **観点**: Principles (SOLID) / Testing
- **ファイル**: `src/aws/sts.rs`, `src/profile/resolver.rs`
- **問題**: `ProfileResolver` が具体型 `StsClient` に直接依存。credential 解決ロジックの単体テストが不可能。結果として credential 解決パイプライン全体がテストされていない。
- **修正案**: `StsOperations` トレイトを抽出し、テスト用モック実装を可能にする。

### M7. 履歴ロード失敗時のサイレントフォールバック
- **観点**: AI Anti-Pattern
- **ファイル**: `src/main.rs:136-139`
- **問題**: `ProfileHistory::load()` 失敗時に空のデフォルトを返す。`tracing::warn!` は `--debug` なしでは見えない。永続的な権限エラーで、ユーザーの favorites が毎回消える。
- **修正案**: stderr に可視の警告を出力する。

---

## Minor Issues

### m1. `stop_all_auto_refresh` がデーモンロックを取得しない
- **ファイル**: `src/autorefresh/daemon.rs:153-170`
- 並行 `start_auto_refresh` との race condition。

### m2. `reap_process` の重複実装
- **ファイル**: `src/autorefresh/daemon.rs:543-558`, `src/utils/process.rs:30-43`
- 同一ロジックが2箇所。`utils::process::reap_process` を public にして統一すべき。

### m3. `matches!` を `assert!` で囲んでいない
- **ファイル**: `src/profile/types.rs:273`
- テストが常に pass する。`assert!(matches!(...))` に修正。

### m4. STS API コールにリトライがない
- **ファイル**: `src/aws/sts.rs`
- 一時的ネットワーク障害で即座に失敗。5回連続失敗でデーモンが終了。

### m5. `StsClient::new()` が不要な場合も呼ばれる
- **ファイル**: `src/main.rs:175`
- キャッシュヒット時でも AWS SDK の初期化が発生（EC2 上で IMDS タイムアウト等）。

### m6. `.awswit` ディレクトリのパーミッションが 0o755
- **ファイル**: `src/config/awswit_config.rs:84`
- `cache/manager.rs` は 0o700 を使用しているが、config/history の保存時は未設定。

### m7. CRLF/LF の混在（Windows）
- **ファイル**: `src/autorefresh/credentials_file.rs:159-168`
- 新規セクションが常に LF で追記され、既存 CRLF と混在。

### m8. プロファイル名でのパストラバーサル
- **ファイル**: `src/autorefresh/daemon.rs:183`
- `../` を含むプロファイル名でメタデータファイルが意図しないパスに書き込まれる可能性。

### m9. `role-duration` の `set_value` での範囲検証がない
- **ファイル**: `src/config/awswit_config.rs:107-111`
- `session-token-duration` は検証されるが `role-duration` は未検証。

### m10. `credential_process` の `Version` フィールド未検証
- **ファイル**: `src/profile/resolver.rs:712-723`
- Version 2 等の不正出力を静かに受け入れる。

### m11. bash/zsh の `echo` がエスケープシーケンスを解釈する可能性
- **ファイル**: `src/init/bash.sh:13`, `src/init/zsh.sh:13`
- `printf '%s\n'` を使用すべき。

### m12. `--role-arn` 短縮形が `arn:aws` パーティションを仮定
- **ファイル**: `src/cli/args.rs:121-134`
- GovCloud / 中国リージョンで不正な ARN が生成される。

### m13. YAML config のタイポが検出されない
- **ファイル**: `src/config/awswit_config.rs`
- `deny_unknown_fields` がないため `fuzzzy-match: true` 等のタイポが黙殺。

### m14. `AWS_SECURITY_TOKEN` は不要
- **ファイル**: `src/shell/export.rs:44`
- boto2（2021年EOL）向け。v0.1.0 の新ツールには不要。

### m15. デーモンの jitter 計算が実質不要
- **ファイル**: `src/autorefresh/runner.rs:138-149`
- シングルデーモン設計で thundering herd は発生しない。

### m16. 未使用コード: `ProfilePreview::with_history`, `ProfilePicker::with_theme`, `AwswitSpinner::_start_message`
- YAGNI。削除可。

### m17. `CredentialProcessOutput` の serde alias が冗長
- **ファイル**: `src/profile/resolver.rs:712-723`
- `rename_all = "PascalCase"` と同一の alias。

### m18. 不要な `async` 関数
- **ファイル**: `src/autorefresh/daemon.rs:127,153`
- `stop_auto_refresh`, `stop_all_auto_refresh` に `.await` 呼び出しがない。

---

## Improvements (コード品質・構造)

### I1. `main.rs` の `run()` 関数が肥大（約170行）
- **ファイル**: `src/main.rs`
- 認証情報ファイルパス解決、プロファイル読み込み、auto-refresh チェックなどが一枚岩。
- ファイルパス解決ロジック（87-119行目）が config/credentials で重複。
- **修正案**: サブ関数への切り出し、または `AppContext` 構造体へのグルーピング。パス解決は `utils::paths` に環境変数対応版を集約。

### I2. `clone()` の多用
- **ファイル**: `src/profile/resolver.rs`, `src/shell/export.rs`
- `args.session_name.clone()`, `args.region.clone()` 等が頻出。`&str` や `AsRef<str>` で借用を通せる箇所あり。
- `credential_bindings()` で `creds.access_key_id.clone()` 等 — `Cow<str>` で不要なアロケーション削減可能。

### I3. デーモン周りの `unsafe` ブロック
- **ファイル**: `src/autorefresh/daemon.rs`
- `libc::pipe`, `libc::fcntl`, `libc::kill`, `libc::waitpid` 等を直接使用。正しく動作するが、`nix` クレートで安全なラッパーが利用可能。
- `pre_exec` クロージャ内の unsafe は fork/exec の制約上避けがたい（コメント十分）。

### I4. 不要な `async` マーク（m18 の詳細）
- **ファイル**: `src/autorefresh/daemon.rs:127,153`
- `stop_auto_refresh()` / `stop_all_auto_refresh()` が async fn だが、中身は同期処理のみ。
- `get_credentials_from_source()` の "Environment" アームも同期だが他アームとの統一のため許容。

### I5. エラーバリアントが汎用的すぎる
- **ファイル**: `src/error.rs`
- `ShellError { message }`, `AutoRefreshError { message }` は catch-all 的で原因の区別が困難。「ホームディレクトリが見つからない」も `ShellError` に分類されている（`main.rs:94-96`）。
- **修正案**: `HomeDirNotFound`, `CredentialFileNotFound` 等の具体的なバリアントに分割。

### I6. テストカバレッジの追加対象
- TUI (`picker.rs`, `spinner.rs`) のロジック部分の分離・テスト
- `config/aws_files.rs` のパーミッションチェックロジック
- `autorefresh/runner.rs` のリフレッシュループの単体テスト

### I7. 依存関係の懸念
- `serde_yml 0.0.12` はプレリリース版。安定版の `serde_yaml` または代替を検討。
- `chrono::Duration` は deprecated 警告の可能性あり。`chrono::TimeDelta` への移行推奨。

---

## Other Suggested Improvements

1. **STS トレイト抽出** (M6) -- テスタビリティの大幅な向上。credential 解決、MFA キャッシュ、ロールチェーンのテストが可能に。
2. **デーモンメタデータに credentials パスを含める** (M1) -- `AWS_SHARED_CREDENTIALS_FILE` サポート。
3. **`tokio::task::spawn_blocking`** (M4) -- async コンテキストのブロック解消。
4. **`update_credentials_file` のE2Eテスト追加** -- crash recovery やべき等性の検証。
5. **config ロード時の unknown key 警告** -- ユーザビリティ向上。

---

## Recommended Tests

| 優先度 | テスト対象 | 理由 |
|--------|-----------|------|
| High | Credential resolution (role chain, MFA cache, credential_process) | 最重要パスがテストゼロ (M6 のトレイト抽出後) |
| High | Daemon lifecycle (spawn, PID management, kill) | プラットフォーム固有コードがテストゼロ |
| High | `update_credentials_file` E2E | crash recovery・べき等性の検証 |
| Medium | PowerShell wrapper の空値処理 | M2 の修正検証 |
| Medium | `types.rs` の `matches!` → `assert!(matches!)` 修正 | 既存テストが偽陽性 |
| Low | Config round-trip (save → load) | `atomic_write_restricted` パスの検証 |

---

## Merge Decision

**Conditional**

M1〜M5 は本番環境で実際に問題を引き起こす可能性がある。特に:
- **M2** (PowerShell): リージョン解決が壊れる実ユーザーシナリオ
- **M1** (credentials path): カスタムパスユーザーの auto-refresh が無効化
- **M5** (MFA chain): MFA付き深いロールチェーンでデーモンが失敗し続ける

M6, M7 はリスクは低いが品質向上のために推奨。
