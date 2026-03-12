# awswit prot ブランチ — 統合コードレビューレポート

## Context

`plans/refactaring.md` に基づく大規模リファクタリング（Phase 0-1 ～ 6-1）の実装結果を、9つの専門観点（正しさ、セキュリティ、パフォーマンス、並行性・信頼性、保守性、API互換性、テスト、開発原則、AIアンチパターン）から並列レビューした統合結果。

---

## Summary

全Phaseの受入基準は概ね達成されている。StsOperations/CredentialStore trait、YAML→TOML移行、exec/--fzf/frecency/completions、依存整理は完了。ただし、ロック順序違反・TOCTOU・本番コードでの expect 使用・テスト隔離不足など、本番環境で問題を引き起こしうる指摘が複数ある。

---

## Critical Issues

### CR-1: `start_auto_refresh` のロック順序違反
- **場所**: `src/autorefresh/daemon.rs:122-135`
- **問題**: credentials lock → daemon lock の順で取得しており、文書化されたロック順序（daemon lock → credentials lock）に違反
- **影響**: デッドロックの可能性
- **修正案**: daemon lock を先に取得するか、`write_credentials` を `spawn_autoawswit_daemon` の `with_daemon_lock` 内に移動

### CR-2: `stop_all_auto_refresh` がデーモンロックなしで操作
- **場所**: `src/autorefresh/daemon.rs:174-191`
- **問題**: profile一覧取得→削除→kill が非アトミック。並行する `start_auto_refresh` が削除後に新profileを登録すると、daemon kill後に孤立profileが残る
- **修正案**: `stop_auto_refresh` と同様に全体を `with_daemon_lock` で囲む

### CR-3: `write_own_pid_file` に `O_NOFOLLOW` なし
- **場所**: `src/autorefresh/runner.rs:197-208`
- **問題**: シンボリックリンク攻撃でPIDファイル経由の任意ファイル上書きが可能。他の機密ファイル操作は全て `O_NOFOLLOW` を使用
- **修正案**: `.custom_flags(libc::O_NOFOLLOW)` を追加、もしくは `atomic_write_restricted` を使用

### CR-4: 本番コードでの `expect()` 使用（計画違反）
- **場所**: `src/bin/autoawswit.rs:55,103`、`src/main.rs:39`
- **問題**: コンテナ環境等で `home_dir()` が `None` の場合にパニック。計画の「新規本番コードに unwrap/expect 禁止」に違反
- **修正案**: `match` + graceful exit に置換

### CR-5: エラーコード E020 の重複
- **場所**: `src/error.rs:67-71`
- **問題**: `TomlDeError` と `TomlSerError` が同じ `[E020]`。スクリプトやモニタリングでの判別不能
- **修正案**: `TomlSerError` を `[E021]` に変更、後続コード再採番

### CR-6: `mock_single_hop_role_chain` テストが実ホームディレクトリに書き込み
- **場所**: `src/profile/resolver.rs:1134`
- **問題**: `CacheManager::new().unwrap()` が `~/.awswit/cache/` を操作。テスト隔離失敗
- **修正案**: `TempDir` ベースの `CacheManager` を使用

### CR-7: `graceful_kill` が完全なデッドコード（計画違反）
- **場所**: `src/utils/process.rs:6-26`
- **問題**: `pub` だがどこからも呼ばれていない。同じSIGTERM→wait→SIGKILLロジックが `daemon.rs:550-575` にインラインで重複実装されている。計画の「今使わないが将来使うかも」のコード禁止に違反
- **修正案**: `graceful_kill` を削除するか、`daemon.rs` のインライン実装をこの関数に統合

---

## Major Issues

### MJ-1: `get_mfa_token` が `no_interactive` フラグを無視
- **場所**: `src/profile/resolver.rs:449-453`
- **問題**: `exec` サブコマンド（`no_interactive: true`）でMFAプロファイル使用時、dialoguer がstdinから読もうとしてハング
- **修正案**: `args.no_interactive` チェックを追加し、true なら `MfaTokenRequired` エラーを返す

### MJ-2: `--list-profiles` の `detail_level` 引数が無視される
- **場所**: `src/cli/args.rs:44-45`、`src/main.rs:104-106`
- **問題**: ヘルプに "Pass 'more' for additional details" と記載されるが値は使用されない。API契約違反
- **修正案**: 値を使用して出力を切り替えるか、ヘルプ文言を修正

### MJ-3: `handle_exec` が `run()` のクレデンシャル解決フローを重複実装
- **場所**: `src/main.rs:406-469`
- **問題**: spinner、history記録、将来追加されるミドルウェアがexecパスで欠落するリスク
- **修正案**: 共通の `resolve_and_cache_credentials()` ヘルパーを抽出

### MJ-4: `AWSWIT_FZF_OPTS` がfzfの任意コマンド実行フラグを許容
- **場所**: `src/tui/fzf.rs:16-33,64`
- **問題**: `--preview=malicious_command` 等が渡される可能性（自己XSSだがCI/sudo環境で問題）
- **修正案**: 危険なフラグ（`--preview`, `--bind`, `--execute`）のブロックリスト、またはドキュメントに信頼前提を明記

### MJ-5: スピナーの `finish_success`/`finish_error` がスレッドjoinなしの10msスリープ
- **場所**: `src/tui/spinner.rs:86-116`
- **問題**: 高負荷時にスピナースレッドが出力中でstderr出力が競合
- **修正案**: `self` を消費して `handle.join()` するAPIに変更

### MJ-6: `unquote_yaml_value` のOR論理バグ
- **場所**: `src/config/awswit_config.rs:438-439`
- **問題**: `||` で片側のみ引用符がある値（例: `team's`）を誤って引用符付きとして扱う
- **修正案**: `starts_with && ends_with` に修正

### MJ-7: `credential_process` stderrがDEBUGログに出力
- **場所**: `src/profile/resolver.rs:662-667`
- **問題**: `--debug` 使用時にcredential helperのstderr（機密情報を含む可能性）が端末に表示
- **修正案**: stderrログを削除するか、固定メッセージに置換

### MJ-8: `lock_exclusive_with_timeout` のasyncコンテキストからの直接呼び出し
- **場所**: `src/utils/fs.rs:129`、`src/autorefresh/daemon.rs` の `stop_auto_refresh`/`stop_all_auto_refresh`
- **問題**: `thread::sleep` がtokioランタイムスレッドを最大30秒ブロック
- **修正案**: 呼び出し元を `spawn_blocking` でラップ

### MJ-9: プロファイルリフレッシュが逐次実行
- **場所**: `src/autorefresh/runner.rs:338-348`
- **問題**: N個のプロファイル × 最大60秒タイムアウト。5+プロファイルでチェック間隔を超過しうる
- **修正案**: `JoinSet` で並行リフレッシュ

### MJ-10: multi-hop role chainのE2Eテスト不足
- **場所**: テスト全体
- **問題**: `get_role_chain` は構造テストのみ。中間クレデンシャルの受け渡しがMockSTS経由でテストされていない
- **修正案**: 2-hop chain で MockStsClient の assume_role 呼び出し引数を検証するテスト追加

### MJ-11: `ProfileHistory` の load/save にロックなし
- **場所**: `src/history/storage.rs`
- **問題**: 並行するawswitプロセスがload→modify→saveすると最後のwriterが勝ち、set_favorite等の変更が無言で消失
- **修正案**: fd-lockベースのロックをload/saveに追加

### MJ-12: `runner.rs:295` の silent fallback（計画違反）
- **場所**: `src/autorefresh/runner.rs:295-299`
- **問題**: `credentials_path_for_profile` のエラーが `unwrap_or_else` で握りつぶされ、ログ出力もない
- **修正案**: `tracing::warn!` でエラーをログ

### MJ-13: `remove_auto_refresh_profile` の TOCTOU
- **場所**: `src/autorefresh/daemon.rs:232-238`
- **問題**: `path.exists()` + `remove_file` は非アトミック。他箇所は `remove_file` + `NotFound` 無視パターンを使用
- **修正案**: `remove_file` して `ErrorKind::NotFound` を無視

### MJ-14: fzfへのstdin書き込みエラーが握りつぶされている
- **場所**: `src/tui/fzf.rs:86`
- **問題**: `let _ = stdin.write_all(input.as_bytes())` — fzfが即座に終了した場合、書き込み失敗が無視され空の選択肢が表示される
- **修正案**: `write_all` のエラーをハンドリングし、適切なエラーメッセージを返す

### MJ-15: `read_pid` がPermissionDeniedを「デーモン未起動」と誤判定
- **場所**: `src/autorefresh/daemon.rs:614-615`
- **問題**: `.ok()?` が `NotFound` と `PermissionDenied` を同一視。PIDファイルが読めない場合に `is_autoawswit_running()` が `false` を返し、`stop_auto_refresh` がデーモン停止に失敗
- **修正案**: `NotFound` のみ `None` にし、他のIOエラーは `Err` として返す

### MJ-16: `child.kill()`/`child.wait()` のエラーが握りつぶされている
- **場所**: `src/autorefresh/daemon.rs:415-417,424-426,454-455`
- **問題**: デーモンハンドシェイク失敗時のcleanup処理で `let _ = child.kill(); let _ = child.wait()` — 子プロセスがゾンビ化する可能性
- **修正案**: エラーを `tracing::warn!` でログ

### MJ-17: `AWS_SECURITY_TOKEN` レガシー変数が永続的に出力（boto2 EOL 2021年）
- **場所**: `src/shell/export.rs:41-44`
- **問題**: 計画の「期限のない互換コード」禁止に該当。セッショントークンが2つの変数名で環境に露出
- **修正案**: 削除するか、設定で無効化可能にする

### MJ-18: Legacy YAML パーサーに削除期限なし
- **場所**: `src/config/awswit_config.rs:57-145`
- **問題**: 33行以上のbespoke YAMLパーサーが起動ごとに実行される。計画の「永続的な YAML/TOML 両対応分岐」禁止に該当
- **修正案**: 削除期限（例: v2.1で削除）をコメントに明記

---

## Minor Issues

### MN-1: dead code — `Credentials` の未使用メソッド3つ (`time_until_expiration`, `expiration_string`, `with_region`)
### MN-2: dead code — `AwswitSpinner` の未使用ファクトリメソッド3つ (`mfa`, `session_token`, `refreshing`)
### MN-3: dead code — `ProfilePreview.history` フィールドが常に `None`
### MN-4: `AwswitSpinner::set_message` が意図的なno-op（不要な後方互換）
### MN-5: `humantime`/`aws-types` が `Cargo.toml` にあるがソースで未使用の可能性
### MN-6: `strsim` が `[dependencies]` と `[dev-dependencies]` に重複
### MN-7: hex文字列生成がバイトごとに `format!` 呼び出し（不要なアロケーション）
### MN-8: `Matcher::new` がキー入力ごとに再生成（`PickerApp` にフィールド保持すべき）
### MN-9: `truncate` 関数が非切り捨て時も `String` をアロケート（`Cow<str>` 推奨）
### MN-10: `sanitize_profile_name` と `validate_profile_name` のルール不一致（DRY違反）
### MN-11: エラーコード E026/E027 が E099 の後に定義（非順序）
### MN-12: `AWSWIT_FZF_OPTS` の空白分割がクォート内スペースを非対応（`shlex` 推奨）
### MN-13: history corruption時の `.corrupt` バックアップ動作のテスト不足
### MN-14: `StsClient::client_with_credentials` がhopごとに `aws_config::load()` を再実行
### MN-15: completions の zsh/fish/powershell テスト不足
### MN-16: `init` サブコマンドのテスト不足
### MN-17: dead code — `graceful_kill` が未使用（別途CR-7でも指摘）
### MN-18: dead code — `favorite_profiles()` が本番コードから未呼び出し（テストのみ）
### MN-19: dead code — `Icons::ascii()` が未呼び出し（ASCII fallback機能が未接続）
### MN-20: dead code — `Profile::awswit_cache_name` / `Profile::autoawswit` フィールドが設定されるが未参照
### MN-21: `aws_files.rs` の `fs::metadata` エラーが「ファイル不在」と同一視（PermissionDenied等を無視）
### MN-22: `AWSWIT_NOTIFY_FD` のparse失敗が無言（デーモン起動失敗の原因特定が困難）
### MN-23: `sanitize_sdk_error` のcatch-allが有用なエラー詳細を破棄（`"AWS request failed"` のみ返却）
### MN-24: PIDファイル削除時 `get_pid_file_path()` 失敗で無言スキップ（stale PID残留リスク）

---

## Acceptance Criteria Status

| Phase | 基準 | 状態 |
|-------|------|------|
| 0-1 | StsOperations object-safe + MockStsClient テスト | PASS（ただしテスト隔離問題: CR-6） |
| 0-2 | CredentialStore object-safe + get/set/remove テスト | PASS |
| 1-1 | serde_yml 消滅 + TOML読込 + YAML fallback警告 | PASS |
| 1-2 | colored/indicatif/console 消滅 | PASS |
| 1-3 | fs2 消滅 + fd-lock | PASS |
| 1-4 | uuid 消滅 + completions 動作 | PASS |
| 2-1 | nucleo-matcher でTUI fuzzy matching | PASS |
| 3-1 | exec + exit code 伝播 | PASS |
| 3-2 | frecency_score 4+テストケース | PASS |
| 3-3 | --fzf + truthy parser テスト | PASS |
| 4-1 | ProfileValidationError 消滅 | PASS |
| 5-1 | tokio "full" 消滅 | PASS |
| 計画 | 本番コードに unwrap/expect なし | **FAIL** (CR-4) |
| 計画 | silent fallback なし | **FAIL** (MJ-12) |
| 計画 | エラーコード体系の整合性 | **FAIL** (CR-5) |
| 計画 | ロック順序の維持 | **FAIL** (CR-1) |
| 計画 | 「今使わないが将来使うかも」のコード禁止 | **FAIL** (CR-7, MN-17~20) |
| 計画 | 永続的な YAML/TOML 両対応分岐禁止 | **FAIL** (MJ-18) |
| 計画 | #[allow(dead_code)] 新規追加禁止 | PASS |
| 計画 | pub 可視性は最小限 | **WARN** (MN-17,18,19 で pub な dead code) |

---

## Recommended Tests

1. **multi-hop role chain E2E** — MockStsClient で2-hop chain の中間クレデンシャル受け渡しを検証
2. **`get_session_token` mock検証** — MFA付きsourceプロファイルのSTS呼び出し引数テスト
3. **history corruption → `.corrupt` 退避** — `ProfileHistory::load` の破損ファイルハンドリングテスト
4. **`ProfileHistory` round-trip** — `record_use` → `save` → `load` のシリアライズ往復テスト
5. **`no_interactive` + MFAプロファイル** — `MfaTokenRequired` エラーが返ることの検証
6. **ソート動作テスト** — favorite優先が実際のsort comparatorで機能することの検証
7. **`init` サブコマンド出力テスト**
8. **`completions zsh/fish` 出力テスト**

---

## Merge Decision

**Conditional** — 以下をマージ前に修正必須:
- CR-1（ロック順序違反 — デッドロックリスク）
- CR-2（stop_all非アトミック — profile孤立リスク）
- CR-3（O_NOFOLLOW欠落 — シンボリックリンク攻撃）
- CR-4（expect使用 — 計画違反、コンテナ環境でパニック）
- CR-5（エラーコード重複 — E020）
- CR-7（graceful_kill デッドコード — 計画違反「将来のためのコード禁止」）
- MJ-1（no_interactive無視 — execでMFAプロファイル使用時ハング）
- MJ-18（YAML両対応の削除期限なし — 計画違反「永続的な両対応分岐禁止」）

以下は強く推奨（マージ後の速やかな対応）:
- MJ-14～16（silent fallback系 — 計画の「エラー握りつぶし禁止」違反）
- MJ-17（AWS_SECURITY_TOKEN — 不要な後方互換）
- MN-17～20（dead code群 — pub最小化違反）
- CR-6（テスト隔離 — 開発者環境汚染）
