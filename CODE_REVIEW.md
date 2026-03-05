# AI Code Review Report: awswit (v2)

**Date:** 2026-03-05
**Branch:** claude/ai-code-review-system-gLVVv
**Reviewers:** Correctness, Security, Performance, Concurrency/Reliability, Maintainability, API/Compatibility, Testing, Principles Guardian, AI Anti-Pattern

---

## Summary

awswit は AWS プロファイルを高速に切り替えるための Rust 製 CLI ツール。インタラクティブ TUI、STS AssumeRole、クレデンシャルキャッシュ、自動リフレッシュデーモン、シェル統合（bash/zsh/fish/PowerShell）を提供する。

9つの専門レビュアーによる多角的レビューの結果、**本番障害に直結する問題**が複数検出された。特に以下の4カテゴリに重大な問題が集中している：

1. **ファイル I/O の安全性**: クレデンシャルキャッシュの非アトミック書き込み、パーミッション TOCTOU
2. **auto-refresh デーモンの設計欠陥**: 環境変数未更新、無限リトライ、シグナルハンドリング欠如
3. **セキュリティ**: Debug derive によるシークレット露出、PID ファイル攻撃
4. **テスト不足**: セキュリティクリティカルなコードパスにテストが存在しない

---

## Critical Issues

### C1. キャッシュファイルの非アトミック書き込み + パーミッション TOCTOU
**検出:** Concurrency, Security, Correctness
- **ファイル:** `src/cache/manager.rs:67-88`
- **問題:** `std::fs::write()` は truncate→write であり、デーモンの書き込み中に CLI が読むと空/不完全な JSON を取得する。さらにファイルはデフォルトパーミッション（0644）で作成され、0600 に設定するまでの間にシークレットキーが読み取られる。
- **影響:** クレデンシャル取得失敗、マルチユーザー環境での AWS シークレット漏洩
- **修正案:** write-to-temp-then-rename パターン + `OpenOptions` で mode 0o600 を指定。`fs2` が Cargo.toml にあるが未使用なので活用する。

### C2. auto-refresh デーモンがシェル環境変数を更新しない
**検出:** Correctness, Compatibility, Maintainability
- **ファイル:** `src/autorefresh/daemon.rs:152-183`
- **問題:** デーモンはキャッシュファイルを更新するだけで、ユーザーのシェルセッションの `AWS_ACCESS_KEY_ID` 等は古い値のまま。子プロセスから親プロセスの環境変数は更新不可能であり、auto-refresh 機能は環境変数ベースの使用パターンでは**根本的に機能しない**。
- **影響:** ユーザーは自動更新されたと思い込むが、expire したクレデンシャルを使い続ける
- **修正案:** (a) `credential_process` 統合でキャッシュから毎回読み取り、(b) PROMPT_COMMAND/precmd フックで環境変数を更新、(c) この制限をドキュメントに明記

### C3. `Credentials` 構造体の `#[derive(Debug)]` がシークレットを露出
**検出:** Security (x2)
- **ファイル:** `src/aws/credentials.rs:6`, `src/profile/types.rs:31`
- **問題:** `Debug` 実装で `secret_access_key` と `session_token` がそのまま出力される。`--debug` フラグは一般ユーザーがアクセス可能。パニック時のバックトレースにも含まれうる。
- **影響:** ログ・stderr へのクレデンシャル漏洩
- **修正案:** `Debug` を手動実装し、シークレットフィールドを `[REDACTED]` でマスク

### C4. シェルラッパーが export 以外の stdout 出力を eval してしまう
**検出:** Compatibility (v1から継続)
- **ファイル:** `src/shell/export.rs:232-266`
- **問題:** ラッパー関数はすべての stdout を `eval` する。`--list-profiles`、`--credential-process` の出力がシェルコマンドとして eval される。
- **修正案:** eval 不要なサブコマンドでは特定の終了コードを返し、ラッパーが 0 の時のみ eval する。

### C5. configparser がプロファイル名を小文字化する
**検出:** Compatibility, Correctness
- **ファイル:** `src/config/aws_files.rs:39`
- **問題:** `Ini::new()` のデフォルトでセクション名が小文字に変換される。`[profile MyProfile]` → `myprofile`。AWS CLI と非互換。
- **修正案:** `Ini::new()` で case sensitivity を有効化する（`set_case_sensitive(true)` または `Ini::new_cs()`）。

### C6. デーモンの無限リトライ（バックオフなし・上限なし）
**検出:** AI Anti-Pattern, Concurrency
- **ファイル:** `src/autorefresh/daemon.rs:177-180`, `src/bin/autoawswit.rs:23-48`
- **問題:** 恒久的エラー（AccessDenied、ロール削除等）でも固定間隔で永遠にリトライ。AWS API レートリミット発動やCloudTrail アラートを引き起こす。`autoawswit` バイナリも同様で、exit code も確認しない。
- **修正案:** 指数バックオフ（60s→120s→240s、最大30分）+ 連続失敗上限 + 恒久的エラー（AccessDenied等）は即終了

### C7. セキュリティクリティカルなコードにテストが存在しない
**検出:** Testing
- **ファイル:** `src/profile/resolver.rs` (shell_words_split, ProfileResolver), `src/shell/export.rs` (shell metacharacter handling)
- **問題:** credential_process コマンドパーサー（shell injection 防止）、export コマンド生成（eval されるコード）、`is_expired` 境界値にテストが一切ない。
- **影響:** コマンドインジェクション、クレデンシャル期限切れ見逃し

---

## Major Issues

### M1. PID ファイルのパーミッション未設定 + kill_daemon のプロセス名検証欠如
**検出:** Security, Correctness
- **ファイル:** `src/autorefresh/daemon.rs:127-138, 61-92`
- **問題:** PID ファイルが 0644 で作成され、攻撃者が上書き可能。`kill_daemon()` は `is_daemon_running()` と異なりプロセス名を検証せずに SIGTERM を送信する。
- **修正案:** PID ファイルを 0600 で作成。`kill_daemon()` でプロセス名検証を追加。

### M2. デーモン PID ファイルにロック機構なし（TOCTOU）
**検出:** Concurrency
- **ファイル:** `src/autorefresh/daemon.rs:18-21`
- **問題:** 2 つのプロセスが同時に `start_daemon()` を呼ぶと孤立デーモンが発生。
- **修正案:** PID ファイルに対する `flock` でアトミック化。

### M3. 履歴ファイルの read-modify-write に lost update リスク
**検出:** Concurrency, Correctness
- **ファイル:** `src/history/storage.rs:56-72`
- **問題:** 複数 CLI プロセスの同時実行で変更が失われる。
- **修正案:** ファイルロック + atomic write。

### M4. デーモンにシグナルハンドリングなし
**検出:** Concurrency
- **ファイル:** `src/autorefresh/daemon.rs:141-184`
- **問題:** SIGTERM で即座に中断。キャッシュ書き込み中の中断でファイル破損。PID ファイルのクリーンアップもされない。
- **修正案:** `tokio::signal` + `tokio::select!` で graceful shutdown。

### M5. `kill_daemon` が SIGTERM 送信後に終了を待たない
**検出:** Concurrency
- **ファイル:** `src/autorefresh/daemon.rs:61-92`
- **問題:** SIGTERM 直後に PID ファイル削除・新デーモン起動するため、旧デーモンと新デーモンが一時的に並行動作する。
- **修正案:** `kill(pid, 0)` でポーリングして終了確認後に続行。タイムアウト後は SIGKILL。

### M6. MFA 必須プロファイルのデーモンリフレッシュが無限エラーループ
**検出:** Correctness
- **ファイル:** `src/autorefresh/daemon.rs:140-184`
- **問題:** `mfa_token: None` で resolve するため MFA 必須プロファイルでは毎回エラー → 60 秒ごとに永久リトライ。
- **修正案:** auto-refresh 開始時に MFA 必須かチェックし、該当する場合はエラーを返す。

### M7. `run()` の God Function 化 + `resolve_role_arn_profile()` との重複
**検出:** Maintainability, Principles Guardian (DRY)
- **ファイル:** `src/main.rs:69-230, 325-409`
- **問題:** 約 160 行に全責務を集約。クレデンシャル出力ロジックが2箇所に重複し、一方だけ修正して他方を忘れるリスク。
- **修正案:** `output_credentials()` を抽出し、各アクションを個別関数に分離。

### M8. 2つの異なるデーモン実装の共存
**検出:** Maintainability, Principles Guardian (KISS), AI Anti-Pattern
- **ファイル:** `src/autorefresh/daemon.rs:28-55`, `src/bin/autoawswit.rs`, `src/main.rs:29-38`
- **問題:** `run_daemon_loop`（75% of remaining、in-process）と `autoawswit`（固定 45 分、shell out）の 2 実装が共存。動作が予測不能。`AWSWIT_DAEMON_MODE` 環境変数による隠れたエントリーポイントも。
- **修正案:** 1つの実装に統一。

### M9. `atty` クレートが unmaintained (RUSTSEC-2021-0145)
**検出:** Compatibility
- **ファイル:** `Cargo.toml:43`
- **問題:** Windows での未定義動作。Rust 1.70+ では `std::io::IsTerminal` で代替可能（rust-version = 1.75）。
- **修正案:** `atty` を削除し `std::io::IsTerminal` に置き換え。

### M10. `serde_yaml` がアーカイブ済み (deprecated)
**検出:** Compatibility
- **ファイル:** `Cargo.toml:27`
- **修正案:** `serde_yml` (community fork) に移行。

### M11. `System::new_all()` による全プロセススキャン
**検出:** Performance, Maintainability
- **ファイル:** `src/autorefresh/daemon.rs:115`
- **問題:** PID 存在確認のために全プロセス・CPU・メモリ情報を収集。数百ミリ秒のレイテンシ。
- **修正案:** `libc::kill(pid, 0)` + `/proc/{pid}/cmdline` で軽量にチェック。

### M12. 破損キャッシュファイルで Err を返す（graceful fallback なし）
**検出:** Concurrency, AI Anti-Pattern (Unnecessary Fallback の逆)
- **ファイル:** `src/cache/manager.rs:49-55`
- **問題:** JSON パース失敗時に `Err` を返してプロファイル切替全体が失敗する。キャッシュミスとして `Ok(None)` を返すべき。
- **修正案:** パース失敗時は warn ログ + ファイル削除 + `Ok(None)` 返却。

### M13. 履歴ファイル破損時のサイレントデータ消失
**検出:** AI Anti-Pattern (Silent Fallback)
- **ファイル:** `src/history/storage.rs:40-47`
- **問題:** 不正 JSON で全履歴・お気に入りが無警告で空に置き換わる。ユーザーに通知されない。
- **修正案:** stderr に可視的な警告を表示。破損ファイルをバックアップ。

### M14. 未使用依存関係の蓄積
**検出:** AI Anti-Pattern (Over-engineering)
- **ファイル:** `Cargo.toml`
- **問題:** `anyhow`, `dotenvy`, `dialoguer`, `console`, `humantime`, `uuid`, `fs2` がソースコードで使用されていない。AI生成の「念のため」依存の典型。
- **修正案:** 未使用依存を削除。`cargo build` で確認。

---

## Minor Issues

### m1. `dirs::home_dir()` 失敗時のリテラル `~` フォールバック
**検出:** Correctness, AI Anti-Pattern
- **ファイル:** `src/config/aws_files.rs:17-20`, `src/config/awswit_config.rs:70-72`
- **修正案:** 明確なエラーメッセージで早期リターン。

### m2. `ErrorCode` enum が `AwswitError` variant と冗長に二重管理
**検出:** Principles Guardian (KISS)
- **ファイル:** `src/error.rs:4-44`
- **修正案:** `ErrorCode` を削除し、エラーコード文字列を `#[error("...")]` に直接埋め込む。

### m3. 未使用エラーバリアント (`CredentialsExpired`, `AutoRefreshDurationLimit`, `ShellError`)
**検出:** Principles Guardian (YAGNI)
- **ファイル:** `src/error.rs:52, 72-76`
- **修正案:** 削除。必要になったら追加。

### m4. `#![allow(dead_code)]` が未使用コードを隠蔽
**検出:** Principles Guardian (YAGNI)
- **ファイル:** `src/lib.rs:1`
- **修正案:** 削除して個別に対処。

### m5. `HistoryData.favorites` と `HistoryEntry.is_favorite` の二重管理
**検出:** Maintainability, Principles Guardian (YAGNI)
- **ファイル:** `src/history/storage.rs:22`
- **修正案:** 一方に統一。

### m6. `ShellType::detect()` の Bash デフォルトフォールバック
**検出:** AI Anti-Pattern
- **ファイル:** `src/shell/export.rs:32-49`
- **修正案:** `from_name()` で不明な名前にはエラーを返す。`detect()` はデバッグログを追加。

### m7. `--unset` が `AWS_PROFILE` を unset しない
**検出:** Compatibility
- **ファイル:** `src/shell/export.rs:195-203`
- **修正案:** `AWS_PROFILE` を unset 対象に追加。

### m8. `credential_process` JSON の `Expiration` がナノ秒精度を含む可能性
**検出:** Compatibility
- **ファイル:** `src/aws/credentials.rs:42-57`
- **修正案:** `to_rfc3339_opts(SecondsFormat::Secs, true)` で秒精度に固定。

### m9. `credential_process` 出力の `Version` フィールド未検証
**検出:** Compatibility
- **ファイル:** `src/profile/resolver.rs:262-268`
- **修正案:** `Version: 1` の検証を追加。

### m10. `auto_refresh` 条件ロジックの冗長性
**検出:** Maintainability
- **ファイル:** `src/main.rs:220-227`
- **問題:** `|| args.auto_refresh` が常に true で `profile.autoawswit` チェックが無意味。
- **修正案:** 条件をフラット化。

### m11. `clear_all()` がファイル削除失敗を無視
**検出:** AI Anti-Pattern (Silent Fallback)
- **ファイル:** `src/cache/manager.rs:108`
- **修正案:** エラーを収集して返す。

### m12. `session_token_duration` の i64→i32 キャストがオーバーフローしうる
**検出:** Correctness
- **ファイル:** `src/profile/resolver.rs:134`
- **修正案:** `i32::try_from()` またはクランプ。

---

## Suggested Improvements

1. **ファイル I/O 安全性の体系的修正**: 全ファイル書き込みを write-to-temp-then-rename に統一。`fs2` を活用してロック機構を導入。
2. **auto-refresh 設計の再考**: `credential_process` 統合またはシェルフック方式。少なくとも制限をドキュメント化。
3. **デーモン実装の統一**: 2つの実装を1つに統合。`AWSWIT_DAEMON_MODE` 環境変数を廃止し、明示的なサブコマンドまたは専用バイナリに。
4. **依存関係の整理**: 未使用依存の削除、unmaintained クレートの置き換え (`atty`→`IsTerminal`, `serde_yaml`→`serde_yml`, `sysinfo`→`libc::kill`)。
5. **StsClient の trait 化**: テスタビリティ向上。ProfileResolver のユニットテストを可能にする。
6. **セキュリティテストの追加**: パーミッション検証、shell metacharacter エスケープ、adversarial input テスト。

---

## Recommended Tests

### Priority 1 — Critical (セキュリティ・正確性に直結)
| テスト名 | 対象 | 理由 |
|----------|------|------|
| `test_shell_words_split_*` (suite) | `resolver.rs:shell_words_split` | コマンドインジェクション防止のパーサー。クォート、エスケープ、末尾バックスラッシュ等 |
| `test_export_with_shell_metacharacters` | `export.rs:generate_export_commands` | eval されるコードに `$(cmd)`, backtick, `;` 等を含むクレデンシャル |
| `test_is_expired_boundary` | `credentials.rs:is_expired` | 残り 59s/60s/61s の境界値 |
| `test_credential_process_json_variants` | `credentials.rs:to_credential_process_json` | session_token=None, expiration=Some/None |

### Priority 2 — Major (中核ロジック)
| テスト名 | 対象 | 理由 |
|----------|------|------|
| `test_profile_resolver_*` (suite with mock STS) | `resolver.rs` | キャッシュヒット/ミス、ロールチェーン深さ制限、force_refresh |
| `test_cache_file_permissions` | `manager.rs:put` | 0600 パーミッション検証 |
| `test_cache_corrupted_json_fallback` | `manager.rs:get` | 破損 JSON での graceful 処理 |
| `test_unset_commands_powershell` | `export.rs` | PowerShell の unset パス |
| `test_shell_detect_*` | `export.rs:detect` | 環境変数別のシェル検出 |

### Priority 3 — Robustness
| テスト名 | 対象 | 理由 |
|----------|------|------|
| `test_daemon_pid_lifecycle` | `daemon.rs` | PID 保存・確認・停止の一連フロー |
| `test_sorted_profile_names_stability` | `storage.rs` | 同一条件でのアルファベット順フォールバック |
| `test_config_forward_compatibility` | `awswit_config.rs` | 未知フィールドを含む YAML |
| `test_daemon_refresh_near_zero_remaining` | `daemon.rs` | 残り1秒でのスリープ時間計算 |

---

## Merge Decision

### **Block**

以下の理由によりマージをブロックします：

| # | 理由 | カテゴリ |
|---|------|----------|
| C1 | キャッシュの非アトミック書き込みでクレデンシャル漏洩リスク | Security/Reliability |
| C2 | auto-refresh が環境変数を更新せず機能が根本的に破綻 | Design |
| C3 | Debug derive でシークレットがログに露出 | Security |
| C4 | シェルラッパーの eval 問題 | Correctness |
| C5 | プロファイル名の大文字小文字が破壊される | Compatibility |
| C6 | デーモンの無限リトライで AWS API 過負荷 | Reliability |
| C7 | セキュリティクリティカルなコードにテストなし | Testing |

**最低限の修正要件:**
1. C1: atomic write + 適切なパーミッション
2. C3: Debug の手動実装（シークレットマスク）
3. C4: eval 対象の stdout 出力を制御
4. C5: `Ini::new_cs()` に変更
5. C6: 指数バックオフ + 失敗上限
6. C7: shell_words_split + export metacharacter テスト追加
7. C2: 少なくともドキュメントで制限を明記
