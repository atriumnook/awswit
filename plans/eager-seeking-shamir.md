# awswit コードレビュー修正計画

## Context

コードレビューで Critical 3件、Major 15件、Minor 18件の指摘を受けた。全シェルで補完が動かない、PowerShell無限再帰、MFA伝播バグなどの本番障害級の問題から、テストの偽カバレッジ、大量のデッドコードまで幅広い。最高品質のプロダクトを目指し、全指摘を優先度順に修正する。

---

## Phase 1: Critical (ship-blocking)

### C1. オートコンプリートバイナリ名の不一致

- **ファイル**: `Cargo.toml`
- **変更**: `name = "awswit-complete"` → `name = "awswit-autocomplete"`
- 全シェルスクリプト(autocomplete.rs)が既に `awswit-autocomplete` を参照しているため、Cargo.toml側を合わせる

### C2. PowerShell init の無限再帰

- **ファイル**: `src/init/powershell.ps1`
- **変更**: 7行目 `$output = & awswit @Arguments` → `$output = & (Get-Command awswit -CommandType Application).Source @Arguments`
- bash/fish は `command awswit` でバイナリを直接呼んでいる。PowerShell の等価表現に修正

### C3. MFA serial が assume_role_with_mfa_large_duration で再解決される

- **ファイル**: `src/profile/resolver.rs`
- **変更**:
  - `assume_role_with_mfa_large_duration` のシグネチャに `mfa_serial: &Option<String>` を追加
  - 455行目の `self.get_mfa_serial_for_chain(&[profile])` を削除し、引数の `mfa_serial` を使用
  - 呼出元 (183行目) で既に解決済みの `mfa_serial` を渡す

---

## Phase 2: Major 正確性バグ

### M1. マルチホップ role chain の解決失敗

- **ファイル**: `src/profile/resolver.rs` (resolve_role_chain, 207-247行目を書き換え)
- **変更**: 単一 AssumeRole → チェーン全体をイテレーションし、各ロールを順次 AssumeRole
  - `get_source_credentials` で取得したベースクレデンシャルから開始
  - MFA 経由のセッショントークン取得後、chain 内の各ロールプロファイルを順次 assume
  - 最終ホップのみ args からの session_name/region/external_id/duration を適用、中間ホップはプロファイル設定を使用

### M2. デーモンが MFA 必須プロファイルの auto-refresh を許可

- **ファイル**: `src/autorefresh/daemon.rs` (start_auto_refresh), `src/main.rs`
- **変更**:
  - `start_auto_refresh` に `requires_mfa: bool` パラメータ追加
  - 67行目の条件を `if requires_mfa || args.mfa_token.is_some()` に変更
  - main.rs の呼出元でプロファイルチェーンの mfa_serial 有無を判定して渡す

### M3. credential_source Ec2InstanceMetadata/EcsContainer の検証ギャップ

- **ファイル**: `src/profile/types.rs`
- **変更**: `VALID_CREDENTIAL_SOURCES` を `"Environment"` のみに変更。`validate()` で `Ec2InstanceMetadata`/`EcsContainer` に対して「awswit では未サポート。AWS CLI のデフォルトチェーンを使用してください」という明示的エラーを返す

---

## Phase 3: セキュリティ・並行性・パフォーマンス

### M4. DefaultHasher によるキャッシュキー不安定性

- **ファイル**: `src/profile/resolver.rs` (13-19行目)
- **変更**: `DefaultHasher` → MFA serial の hex エンコーディングに置換
  ```rust
  fn mfa_cache_key(access_key_id: &str, mfa_serial: &str) -> String {
      let hex: String = mfa_serial.as_bytes().iter().map(|b| format!("{:02x}", b)).collect();
      format!("session-{}-{}", access_key_id, hex)
  }
  ```
- テスト (691-714行目) も新フォーマットに合わせて更新

### M5. stop_all_auto_refresh の O(N) ファイル I/O

- **ファイル**: `src/autorefresh/credentials_file.rs`, `src/autorefresh/daemon.rs`
- **変更**:
  - `remove_credentials_batch(creds_path, &[String])` を追加 (1回の lock-read-modify_all-write)
  - `stop_all_auto_refresh` のループを batch 呼出に置換

### M6. StsClient の遅延初期化

- **ファイル**: `src/main.rs` (171行目)
- **変更**: `StsClient::new().await` を `resolve_credentials` の直前に移動。既に早期リターン後にあるため、影響は最小限。キャッシュヒット時のスキップには `OnceCell` を使用するか、必要時のみ生成するヘルパーを追加

### M7. デーモン子プロセスがロック未保持

- **ファイル**: `src/bin/autoawswit.rs`
- **変更**: 子プロセス起動後、`write_own_pid_file()` の前にデーモンロックを取得。PID ファイル書込み完了後にロック解放

### M8. kill_autoawswit_daemon がプロセス終了を待たない

- **ファイル**: `src/autorefresh/daemon.rs` (402-410行目)
- **変更**: SIGTERM 送信後、`kill(pid, 0)` で最大5秒ポーリング確認してから PID ファイル削除

### M9. クレデンシャル/メタデータ書込み順序

- **ファイル**: `src/autorefresh/runner.rs` (434-461行目)
- **変更**: クレデンシャル(451-461行) を先に書き、メタデータ(436-449行) を後に書く。メタデータだけ更新されてクレデンシャルが古いままという状態を防止

### M10. ロックファイルのパーミッション

- **ファイル**: `src/autorefresh/credentials_file.rs` (69-73行目)
- **変更**: Unix で `.mode(0o600)` を追加
  ```rust
  #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; opts.mode(0o600); }
  ```

### M11. auto-refresh JSON の期限切れクリーンアップ

- **ファイル**: `src/autorefresh/runner.rs` (refresh_all_profiles)
- **変更**: プロファイル読込後、`MAX_EXPIRED_HOURS` を超えて期限切れのものは JSON ファイルとクレデンシャルセクションを自動削除

---

## Phase 4: デッドコード削除 (M15)

### 未使用 CLI フラグ削除

- **ファイル**: `src/cli/args.rs`
- 削除対象: `--clean`, `--with-saml`, `--with-web-identity`, `--session-policy`, `--session-policy-arns`, `--principal-arn`

### 未使用メソッド削除

| メソッド | ファイル |
|---------|---------|
| `Credentials::from_sdk_credentials` | `src/aws/credentials.rs:83-96` |
| `Credentials::expires_within` + テスト | `src/aws/credentials.rs:60-66, 126-134` |
| `Profile::is_user_profile` | `src/profile/types.rs:82-84` |
| `Profile::get_region` | `src/profile/types.rs:162-166` |
| `Theme::minimal` | `src/tui/theme.rs:87-99` |
| `ProfileHistory::toggle_favorite` + テスト | `src/history/storage.rs:133-137` |

### オートコンプリートスクリプトからの未使用フラグ削除

- **ファイル**: `src/shell/autocomplete.rs`
- 全4シェルのフラグリストから `--with-saml`, `--with-web-identity`, `--clean`, `--principal-arn` を削除

---

## Phase 5: DRY リファクタリング (M14)

### パスモジュール集約

- **新規ファイル**: `src/utils/paths.rs`
- `awswit_home_dir() -> Result<PathBuf>`, `aws_credentials_path() -> Result<PathBuf>`, `aws_config_path() -> Result<PathBuf>`
- 10箇所の `dirs::home_dir().ok_or_else(...)?.join(".awswit")` パターンを置換

### デーモンループ統合

- **ファイル**: `src/autorefresh/runner.rs`
- `run_daemon_loop_unix` と `run_daemon_loop_fallback` の共通ロジックを抽出
- シグナルハンドリング部分のみを分離し、共通のループ本体を共有

### write_credentials 統合

- **ファイル**: `src/autorefresh/credentials_file.rs`
- `write_credentials` が `Credentials` を key/value に変換して `write_credentials_from_output` を呼ぶ形に統一

### ロックヘルパー共通化

- **ファイル**: `src/utils/fs.rs` (既存ファイルに追加)
- `lock_exclusive_with_timeout(file: &File, timeout: Duration) -> io::Result<()>`
- `credentials_file.rs:lock_with_timeout` と `daemon.rs:lock_daemon` の重複を解消

### ロールパラメータ解決ヘルパー

- **ファイル**: `src/profile/resolver.rs`
- session_name / region / external_id / role_duration のカスケード解決を `resolve_role_params()` に抽出

---

## Phase 6: Minor 修正

| # | 修正内容 | ファイル |
|---|---------|---------|
| m1 | `history_path()` の `unwrap_or_default()` → `Result` 返却 | `src/history/storage.rs:35-39` |
| m2 | Credentials Debug のバイトスライス → `is_ascii()` ガード追加 | `src/aws/credentials.rs:27-31` |
| m3 | Windows credential_process タイムアウト → 警告ログ追加 | `src/profile/resolver.rs:582-586` |
| m4 | `get_value` の `{:?}` → String なら直接返す | `src/config/awswit_config.rs:152` |
| m5 | temp ファイル名にランダム成分追加 | `src/utils/fs.rs:15-19` |
| m6 | serde_yaml → serde_yml への移行を検討 (コメント追加) | `Cargo.toml` |
| m7 | fuzzy matching の `to_lowercase()` 事前計算 | `src/utils/fuzzy.rs` |
| m8 | `lock_with_timeout` の `thread::sleep` → 意図的である旨のコメント追加 | `src/autorefresh/credentials_file.rs:96` |
| m9 | ProfileHistory TOCTOU → `read` して `NotFound` ハンドリング | `src/history/storage.rs:43-68` |
| m10 | fallback daemon loop にシグナル非対応の旨コメント | `src/autorefresh/runner.rs:122` |
| m11 | `refresh_profile` 子プロセスに60秒タイムアウト追加 | `src/autorefresh/runner.rs:397-400` |
| m12 | AWSume-rs/AWSUME_SHELL → awswit に改名 (互換 fallback 付き) | `src/shell/export.rs:107-110`, `src/cli/args.rs:3,9` |
| m13 | Cargo.toml のプレースホルダー更新 | `Cargo.toml:6,9` |
| m14 | zsh 用 init スクリプトに compdef 追加 | `src/init/bash.sh` or 新規 `src/init/zsh.sh` |
| m15 | `assume_role` 8引数 → `AssumeRoleParams` ビルダー導入 | `src/aws/sts.rs:65-75` |
| m16 | フィールド名を繰り返すだけの doc comment 削除 | 複数ファイル |
| m17 | AWS_SECURITY_TOKEN に互換性理由のコメント追加 | `src/shell/export.rs:40-47` |
| m18 | daemon JSON パース失敗に `tracing::warn` 追加 | `src/autorefresh/runner.rs:270-272` |

---

## Phase 7: テスト改善

### M12. 偽テストカバレッジの修正

| テストファイル | 修正内容 |
|-------------|---------|
| `tests/unit_tests.rs` | ローカル再実装を削除し、`awswit::` クレートの実関数を import してテスト。`validate_mfa_token` を `pub` 公開 |
| `tests/config_parsing.rs` | `AwsFiles::load()` を呼び出して実際のパース結果を検証 |
| `tests/fuzzy_matching.rs` | `awswit::utils::fuzzy::find_closest_profile` を通してテスト (strsim 直接呼出を廃止) |
| `tests/profile_resolution.rs` | `Args::resolve_role_arn()` を直接テスト (ローカル ARN パース再実装を廃止) |

### M13. セキュリティクリティカルパスのテスト追加

新規 `tests/security_tests.rs`:

- **should_refresh()**: 期限内/期限切れ/None/MAX_EXPIRED_HOURS超過/不正文字列
- **refresh_profile コマンド検証**: ALLOWED_NAMES 拒否、非sibling パス拒否
- **shell_quote()**: `'; rm -rf /`, `$(whoami)`, embedded quotes
- **generate_shell_output()**: `\n`/`\r` を含む profile_name / credential value → エラー
- **remove_credentials_section**: 空content、不在セクション、末尾セクション、隣接セクション
- **validate_profile_name / validate_credential_value**: 既存テスト拡充

---

## 検証方法

1. `cargo build` — 全バイナリがビルドされること (`awswit`, `autoawswit`, `awswit-autocomplete`)
2. `cargo test` — 全テストが pass
3. `cargo clippy -- -D warnings` — warning なし
4. シェル統合テスト:
   - `eval "$(./target/debug/awswit init bash)"` → bash で awswit 関数が登録
   - `awswit init powershell` の出力に `Get-Command` を含むこと確認
   - `awswit-autocomplete` バイナリが実行可能であること確認
5. PowerShell テスト (可能なら): `pwsh -c 'awswit init powershell | Invoke-Expression; awswit --version'` で無限再帰しないこと
6. デッドコード確認: `cargo test` + `grep -rn` で削除したフラグ/メソッドへの参照が残っていないこと

## 実行順序

Phase 1 → 2 → 4(デッドコード削除、リファクタリング前にノイズ除去) → 3 → 5 → 6 → 7

各 Phase は独立したコミットとし、Phase ごとに `cargo test && cargo clippy` で回帰確認する。
