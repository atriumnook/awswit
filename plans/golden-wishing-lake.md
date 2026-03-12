# awswit コードレビュー指摘修正計画

## Context

9つの専門レビュアーによるコードレビューで Critical 5件、Major 14件、Minor 18件の指摘が検出された。
本計画はこれらを優先度順に7フェーズで修正し、最高品質のプロダクトを目指す。

## 検証済み前提
- `aws-config` クレートは `Cargo.toml` L31 に既存（C2 で追加不要）
- `dotenvy = "0.15"` は依存にあるが `src/` で未使用（Phase 4 で削除対象に追加）
- `runner.rs:refresh_profile()` は stored credentials を使わず `awswit_command` を再実行する（C4 のシークレット削除は安全）
- `shell_scripts/awswit.ps1` L57 に `Export-ModuleMember` が存在（m15 確認済み）
- `main.rs` L395 に `"cli-role"` ハードコード（M14 確認済み）

---

## Phase 1: セキュリティ Critical（C1, C4, M4, M5）

### C1: `--debug` で MFA トークンが平文ログ出力される

- **ファイル**: `src/cli/args.rs`
- **現状**: L6 `#[derive(Parser, Debug, Clone, Default)]` → `Args` 全フィールドが `{:?}` で出力される
- **変更**:
  1. derive から `Debug` を除去 → `#[derive(Parser, Clone, Default)]`
  2. カスタム `impl fmt::Debug for Args` を追加し、`mfa_token` を `[REDACTED]` でマスク
- **テスト**: `Args { mfa_token: Some("123456"), .. }` を `format!("{:?}")` して `123456` が含まれないことを assert

### C4: autorefresh JSON に AWS シークレットが平文保存される

- **ファイル**: `src/autorefresh/daemon.rs`, `src/autorefresh/runner.rs`
- **現状**: `AutoRefreshProfile` (L20-30) に `aws_access_key_id`, `aws_secret_access_key`, `aws_session_token` フィールド。`~/.awswit/autorefresh/<profile>.json` に平文保存
- **分析**: `runner.rs:refresh_profile()` (L442-449) は stored credentials を一切使わず、`awswit_command` をサブプロセスとして再実行して stdout から fresh credentials を取得する。したがって JSON への credential 保存は完全に不要
- **変更**:
  1. `AutoRefreshProfile` から `aws_access_key_id`, `aws_secret_access_key`, `aws_session_token` フィールドを削除
  2. `awswit_cache_name` は access_key_id に依存 (L115) → profile_name ベースに変更: `format!("session-{}", profile_name)`
  3. `daemon.rs:108-118` の構造体構築から credential フィールドを削除。`start_auto_refresh` の引数から `credentials: &Credentials` を削除するか、expiration のみ使用に変更
  4. `runner.rs:504-506` の credential 上書きコードを削除（expiration のみ更新を維持）
  5. カスタム `Debug` impl (L32-49) を `#[derive(Debug)]` に戻す（シークレットがなくなるため）
  6. 既存 JSON との後方互換性: serde はデフォルトで未知フィールドを無視するため OK。新フィールドには `#[serde(default)]` を付与
- **テスト**: `serde_json::to_string(&profile)` の出力に `secret_access_key` / `session_token` が含まれないことを assert

### M4: `autoawswit.lock` に `0o600` パーミッションが未設定

- **ファイル**: `src/autorefresh/daemon.rs` (L257-264), `src/bin/autoawswit.rs` (L43-53)
- **変更**: 両箇所の `OpenOptions` に以下を追加:
  ```rust
  #[cfg(unix)]
  {
      use std::os::unix::fs::OpenOptionsExt;
      opts.mode(0o600);
  }
  ```
- **テスト**: Unix でロックファイル作成後 `metadata().permissions().mode() & 0o777 == 0o600` を assert

### M5: `credential_process` 実行前に config ファイルのパーミッション未検証

- **ファイル**: `src/config/aws_files.rs` の `load()` 関数
- **変更**: config ファイル読み込み時に `o+w`（world-writable）を検出して `tracing::warn!` を出力
  ```rust
  #[cfg(unix)]
  {
      use std::os::unix::fs::MetadataExt;
      if let Ok(meta) = fs::metadata(&config_path) {
          if meta.mode() & 0o002 != 0 {
              tracing::warn!("AWS config file {} is world-writable (mode {:o})", config_path, meta.mode() & 0o777);
          }
      }
  }
  ```

### 検証
```bash
cargo test && cargo clippy --all-targets -- -D warnings
```

---

## Phase 2: セキュリティ Medium + 信頼性（C3, M2, M3, M6, M7）

### C3: `AWSWIT_SHELL` 環境変数が init スクリプトで未設定

- **ファイル**: `src/init/bash.sh`, `src/init/zsh.sh`, `src/init/fish.fish`, `src/init/powershell.ps1`
- **変更**: 各スクリプトの関数定義前に shell 識別変数を追加:
  - bash.sh: `export AWSWIT_SHELL=bash`
  - zsh.sh: `export AWSWIT_SHELL=zsh`
  - fish.fish: `set -gx AWSWIT_SHELL fish`
  - powershell.ps1: `$env:AWSWIT_SHELL = 'powershell'`

### M2: SIGTERM タイムアウト後に SIGKILL 未送信 → デーモン二重起動

- **ファイル**: `src/autorefresh/daemon.rs` (L408-414)
- **現状**: 5秒待機後 `break` するのみ。PID ファイル削除で二重起動リスク
- **変更**: タイムアウト後に SIGKILL を送信:
  ```rust
  if start.elapsed() >= wait_timeout {
      tracing::warn!("Daemon (pid={}) did not exit within {:?}, sending SIGKILL", pid, wait_timeout);
      unsafe { libc::kill(pid_i32, libc::SIGKILL) };
      std::thread::sleep(std::time::Duration::from_millis(200));
      break;
  }
  ```

### M3: `stop_auto_refresh` の TOCTOU

- **ファイル**: `src/autorefresh/daemon.rs` (L139-158, L346-438)
- **現状**: profile 削除→残存確認→daemon kill が非アトミック
- **変更**:
  1. `kill_autoawswit_daemon` の内部ロジックを `kill_autoawswit_daemon_inner()` に抽出（ロック不要版）
  2. `kill_autoawswit_daemon` は lock 取得 → `_inner()` 呼び出し
  3. `stop_auto_refresh` で daemon lock を取得してから profile 削除→残存確認→`_inner()` の一連操作を実行

### M6: `remove_credentials_batch` / `remove_credentials` の TOCTOU

- **ファイル**: `src/autorefresh/credentials_file.rs` (L194, L210)
- **変更**: `.exists()` チェックを削除し `read_to_string` の `NotFound` を直接ハンドリング:
  ```rust
  let content = match fs::read_to_string(creds_path) {
      Ok(c) => c,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
      Err(e) => return Err(AwswitError::IoError { source: e }),
  };
  ```

### M7: `atomic_write_restricted` のテンポラリファイル名に実質的なランダム性がない

- **ファイル**: `src/utils/fs.rs` (L12-25)
- **現状**: `rand_component` が nanos から決定論的に導出。同一ナノ秒の並行呼び出しで衝突
- **変更**: `AtomicU64` カウンターを追加:
  ```rust
  use std::sync::atomic::{AtomicU64, Ordering};
  static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);
  // ...
  let counter = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
  ```
  テンポラリファイル名に `counter` を含める。`rand_component` の Knuth hash は削除

### 検証
```bash
cargo test && cargo clippy --all-targets -- -D warnings
```

---

## Phase 3: 機能的正しさ（C2, M1, M8, M9, M14）

### C2: `credential_source = Ec2InstanceMetadata/EcsContainer` 未サポート

- **ファイル**: `src/profile/types.rs` (L4, L86-92), `src/profile/resolver.rs` (L529-559)
- **現状**: `get_credentials_from_source()` は `Environment` のみ実装。`Ec2InstanceMetadata`/`EcsContainer` は L552-554 で明示的にエラー
- **変更**:
  1. `types.rs` L4: `VALID_CREDENTIAL_SOURCES` を `["Environment", "Ec2InstanceMetadata", "EcsContainer"]` に拡張
  2. `types.rs` L88-92: `Ec2InstanceMetadata`/`EcsContainer` の `UnsupportedCredentialSource` エラー分岐を削除
  3. `resolver.rs` L552-554: `get_credentials_from_source()` に AWS SDK default chain ブランチを追加:
     ```rust
     "Ec2InstanceMetadata" | "EcsContainer" => {
         // aws-config は Cargo.toml L31 に既存
         let sdk_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
         let provider = sdk_config.credentials_provider().ok_or_else(|| ...)?;
         let creds = provider.provide_credentials().await.map_err(|e| ...)?;
         Ok(Credentials { ... })
     }
     ```
  4. `get_credentials_from_source()` を `async fn` に変更（呼び出し元 `get_source_credentials` は既に async）
- **テスト**: validation テストを更新。`Ec2InstanceMetadata` が `UnsupportedCredentialSource` ではなく `Ok` を返すことを assert

### M1: async context で `std::thread::sleep` による executor ブロック

- **ファイル**: `src/utils/fs.rs` (L81-83 のコメント)
- **判断**: 既存コメントが設計意図を説明済み。sleep は最大 1 秒で、daemon の non-critical path のみ。リスク低。
- **変更**: コメントを強化して async context での使用パターンと許容理由を明記:
  ```rust
  /// Uses `thread::sleep` for backoff intentionally — this is called from both
  /// sync and async contexts. In async context (autorefresh daemon), this blocks
  /// the executor thread but the max sleep is 1s with exponential backoff, and
  /// lock contention is rare. If this becomes problematic, wrap call sites in
  /// `tokio::task::spawn_blocking`.
  ```

### M8: `--kill` vs 仕様書の `--kill-refresher` 不一致

- **ファイル**: `src/cli/args.rs` (L40)
- **変更**: `long = "kill-refresher"` に変更し `alias = "kill"` で後方互換:
  ```rust
  #[arg(short = 'k', long = "kill-refresher", alias = "kill")]
  pub kill_refresher: bool,
  ```
- **テスト**: `--kill-refresher` と `--kill` の両方でパース成功を assert

### M9: エラーコード体系が仕様書と実装で不一致

- **ファイル**: `docs/SPECIFICATION.md`
- **変更**: 仕様書のエラーコード表を `src/error.rs` の実装（E001-E027）に合わせて更新

### M14: `--role-arn` 指定時に `AWSWIT_PROFILE=cli-role` がハードコード

- **ファイル**: `src/main.rs` L395 の `determine_target_profile` 関数
- **現状**: `if args.role_arn.is_some() { return Ok("cli-role".to_string()); }`
- **変更**: ARN からロール名を抽出して使用:
  ```rust
  if args.role_arn.is_some() {
      let name = args.session_name.clone()
          .or_else(|| args.resolve_role_arn().and_then(|arn| {
              arn.rsplit('/').next().map(|s| s.to_string())
          }))
          .unwrap_or_else(|| "cli-role".to_string());
      return Ok(name);
  }
  ```

### 検証
```bash
cargo test && cargo clippy --all-targets -- -D warnings
```

---

## Phase 4: コード品質 + 重複排除（M10, M11, M13, m5, m6, m7, m8, m9, m16, m18）

### M10: `get_auto_refresh_dir` / `get_pid_file_path` が daemon.rs と runner.rs に重複

- **ファイル**: `src/autorefresh/mod.rs`, `src/autorefresh/daemon.rs`, `src/autorefresh/runner.rs`
- **変更**: `mod.rs` に共通関数を定義し、両ファイルから参照

### M11: `runner.rs` のエラー型が `Box<dyn Error>` で不統一

- **ファイル**: `src/autorefresh/runner.rs`
- **変更**: 全関数の戻り値を `Result<_, AwswitError>` に統一。文字列エラーは `AwswitError::AutoRefreshError { message }` に変換

### M13: `MANAGED_VARS` と shell wrapper の unset リスト同期保証

- **ファイル**: `src/shell/export.rs`
- **変更**: `MANAGED_VARS` にドキュメントコメントで同期が必要なファイル一覧を明記。テストで各 init スクリプト（`include_str!`）に全 `MANAGED_VARS` が含まれることを検証

### m5: `extract_mfa_args` が無意味なラッパー

- **ファイル**: `src/profile/resolver.rs` (L381-388)
- **変更**: 関数を削除し、呼び出し元で `self.get_mfa_token(args)?` を直接呼ぶ

### m6: `AssumeRoleParams` が未使用

- **ファイル**: `src/aws/sts.rs` (L13-24)
- **変更**: `#[allow(dead_code)]` 付きの構造体を削除

### m7: `ProfileHistory` に未使用メソッド多数

- **ファイル**: `src/history/storage.rs`
- **変更**: `recent_profiles`, `most_used`, `cleanup_old`, `len`, `is_empty` を削除（`favorite_profiles` も未使用なら削除）

### m8: `CacheManager` に未使用メソッド

- **ファイル**: `src/cache/manager.rs`
- **変更**: `clear_all`, `clear_expired`, `list_keys` を削除

### 未使用依存 `dotenvy` の削除

- **ファイル**: `Cargo.toml` (L64)
- **現状**: `dotenvy = "0.15"` が依存にあるが `src/` のどこでも `use dotenvy` されていない
- **変更**: `Cargo.toml` から `dotenvy` を削除

### m9: `VarBinding` 構造体が不要

- **ファイル**: `src/shell/export.rs` (L23-26)
- **変更**: `VarBinding` を削除し `(&'static str, Option<String>)` タプルに置換

### m16: `PickerApp::last_tick` が dead code

- **ファイル**: `src/tui/picker.rs`
- **変更**: `last_tick` フィールドと `#[allow(dead_code)]` を削除

### m18: ARN 抽出ロジックが `main.rs` と `types.rs` に重複

- **ファイル**: `src/main.rs`
- **変更**: `extract_account_from_arn` 関数を削除し `profile.get_account_id()` を使用

### 検証
```bash
cargo test && cargo clippy --all-targets -- -D warnings
```

---

## Phase 5: テスト品質（C5, M12, m13, m14）

### C5: テストが実際にはライブラリコードをテストしていない

- **ファイル**: `tests/config_parsing.rs`, `src/utils/fuzzy.rs`
- **変更**:
  1. `tests/config_parsing.rs`: `AwsFiles::load()` + `merge_profiles()` を直接呼ぶテストに書き直す
  2. `fuzzy.rs` の恒真式 `assert!(result.is_none() || result.is_some())` を正しい期待値に修正

### M12: `runner.rs` にテストが0件

- **ファイル**: `src/autorefresh/runner.rs`
- **変更**: `#[cfg(test)] mod tests` を追加:
  1. `should_refresh` — 有効期限境界値テスト（6分前→false、4分前→true、1時間超過→false、None→false、不正RFC3339→false）
  2. `update_credentials_file` — 正常出力パース、`=` を含む値、必須キー欠落エラー

### m13: テスト重複の整理

- **ファイル**: `tests/unit_tests.rs`, `tests/fuzzy_matching.rs`, `tests/profile_resolution.rs`
- **変更**: `unit_tests.rs` の `fuzzy_matching` / `profile_resolution` モジュールを削除し、専用テストファイルに一本化

### m14: `AwswitConfig::set_value` 境界値テスト不足

- **ファイル**: `src/config/awswit_config.rs`
- **変更**: テスト追加:
  - `session-token-duration`: 899（reject）, 900（accept）, 129600（accept）, 129601（reject）
  - 非数値入力（"abc"）

### 検証
```bash
cargo test --all-features --verbose
```

---

## Phase 6: ポリッシュ + 残り Minor 修正（m1-m4, m10, m11, m12, m15, m17）

### m1: `duration_seconds` パース失敗がサイレント

- **ファイル**: `src/config/aws_files.rs` (L120)
- **変更**: `.ok()` を `tracing::warn!` 付きに変更

### m2: `AwswitConfig::load` の TOCTOU

- **ファイル**: `src/config/awswit_config.rs`
- **変更**: `exists()` チェックを削除し `read_to_string` の `NotFound` を直接ハンドリング（`history/storage.rs` と同じパターン）

### m3: GovCloud ARN ショートハンド

- **ファイル**: `src/cli/args.rs` (L125)
- **変更**: コメント追加: `// Note: shorthand always uses arn:aws partition. For GovCloud/China, use full ARN.`

### m4: TUI カーソル位置表示が末尾固定

- **ファイル**: `src/tui/picker.rs` の `render_search_bar`
- **変更**: `cursor_pos` でクエリ文字列を分割し、カーソル位置に `│` を挿入:
  ```rust
  let (before, after) = self.query.split_at(self.cursor_pos);
  // before + "│" + after
  ```

### m10: `ProfilePicker::new` で履歴を二重ロード

- **ファイル**: `src/tui/picker.rs` (L53)
- **変更**: `new()` で `ProfileHistory::load()` を呼ばず `ProfileHistory::default()` を使用

### m11: `serde_yaml` が unmaintained

- **ファイル**: `Cargo.toml`, `src/config/awswit_config.rs`, `src/error.rs`
- **変更**: `serde_yaml = "0.9"` → `serde_yml = "0.0.12"`。`use serde_yaml` → `use serde_yml`。エラー型のバリアント名は維持

### m12: macOS でデーモン PID identity 未検証

- **ファイル**: `src/autorefresh/daemon.rs` (L386-394)
- **変更**: コメントを追加して制限を明記:
  ```rust
  // TODO: macOS could use sysctl(KERN_PROCARGS2) for process identity verification.
  // Current fallback: trust PID file without verification on non-Linux Unix.
  ```

### m15: PowerShell `Export-ModuleMember` が `.ps1` では無効

- **ファイル**: `shell_scripts/awswit.ps1`
- **変更**: `Export-ModuleMember -Function awswit` 行を削除

### m17: `handle_list_profiles` の `"Fetching..."` が実際にはフェッチしない

- **ファイル**: `src/main.rs`
- **変更**: `"Fetching..."` を `profile.get_account_id().unwrap_or_else(|| "-".to_string())` に置換

### 検証
```bash
cargo test && cargo clippy --all-targets -- -D warnings
cargo build --release
```

---

## Phase 7: 最終検証

- `cargo test --all-features --verbose` — 全テスト通過
- `cargo clippy --all-targets --all-features -- -D warnings` — 警告ゼロ
- `cargo fmt --check` — フォーマット準拠
- `cargo build --release` — リリースビルド成功
- `./target/release/awswit --help` — ヘルプ表示確認
- `./target/release/awswit --debug --version 2>&1` — MFA トークンがログに漏れないこと確認
- `ls -la ~/.awswit/autorefresh/` — JSON ファイルにシークレットが含まれないこと確認（既存ファイルがある場合）

---

## フェーズ間の依存関係

```
Phase 1 (Security) → Phase 2 (Reliability) → Phase 3 (Correctness)
                                                      ↓
Phase 4 (Code Quality) → Phase 5 (Tests) → Phase 6 (Polish) → Phase 7 (Final)
```

- Phase 1 は依存なし、最初に着手
- Phase 3 (M10, M11) は Phase 2 (M3) の daemon.rs リファクタリング後が望ましい
- Phase 5 は Phase 3, 4 の変更反映が必要（C2 の validation 変更等）
- Phase 6, 7 は独立して実行可能
