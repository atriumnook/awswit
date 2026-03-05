# AI Code Review Report: awswit

**Date:** 2026-03-05
**Branch:** claude/ai-code-review-system-gLVVv
**Reviewers:** Correctness, Security, Performance, Concurrency/Reliability, Maintainability, API/Compatibility, Testing

---

## Summary

awswit は AWS プロファイルを高速に切り替えるための Rust 製 CLI ツールで、インタラクティブ TUI、STS AssumeRole、キャッシュ、自動リフレッシュデーモン、シェル統合（bash/zsh/fish/PowerShell）を提供する。

全体として機能は良く設計されているが、**並行性・ファイル安全性**、**セキュリティ（クレデンシャル保護）**、**auto-refresh デーモンの設計上の欠陥**、および**テスト不足**に重大な問題がある。

---

## Critical Issues

### 1. [Concurrency/Security] キャッシュファイルの非アトミック書き込みによるクレデンシャル破損・漏洩
- **ファイル:** `src/cache/manager.rs:67-88`
- **問題:** `std::fs::write()` は内部で truncate + write であり、デーモンがキャッシュを書き込んでいる最中に CLI が read すると、空ファイルや中途半端な JSON を読み取る。さらに、ファイルはデフォルトパーミッション（0644）で作成され、パーミッション設定（0600）までの間に他ユーザーが読み取れる。
- **影響:** クレデンシャル取得失敗、マルチユーザー環境での AWS シークレットキー漏洩
- **修正案:** write-to-temp-then-rename パターンを使用し、`OpenOptions` で mode 0o600 を指定してファイルを作成する。

### 2. [Concurrency] デーモン起動の TOCTOU 競合（PID ファイルにロックなし）
- **ファイル:** `src/autorefresh/daemon.rs:17-21`
- **問題:** `is_daemon_running()` → `kill_daemon()` → 新デーモン起動の間にファイルロックがない。2 つのプロセスが同時に `start_daemon()` を呼ぶと、孤立デーモンが発生する。
- **修正案:** PID ファイルに対して `flock(2)` (advisory lock) を取得し、check-kill-spawn をアトミックにする。`fs2` クレートが Cargo.toml に存在するが未使用。

### 3. [Correctness/Design] auto-refresh デーモンがシェル環境変数を更新しない
- **ファイル:** `src/autorefresh/daemon.rs:140-184`
- **問題:** デーモンはキャッシュファイルを更新するだけで、ユーザーのシェルセッションの `AWS_ACCESS_KEY_ID` 等は古い値のまま。ユーザーは自動更新されたと思い込むが、expire したクレデンシャルを使い続ける。
- **修正案:** (a) `credential_process` 方式でキャッシュから毎回読み取る、(b) PROMPT_COMMAND/precmd フックで環境変数を更新する、(c) この制限をドキュメントに明記する。

### 4. [Compatibility] シェルラッパーが export 以外の stdout 出力を eval してしまう
- **ファイル:** `src/shell/export.rs:232-266`
- **問題:** ラッパー関数はすべての stdout 出力を `eval` する。`--list-profiles`、`--credential-process` の出力がシェルコマンドとして eval され、エラーまたは予期しない動作になる。
- **修正案:** eval 不要なサブコマンドでは特定の終了コード（例: 2）を返し、ラッパーが 0 の時のみ eval するようにする。

### 5. [Compatibility] configparser がプロファイル名を小文字化する
- **ファイル:** `src/config/aws_files.rs:39`
- **問題:** `Ini::new()` のデフォルトではセクション名が小文字に変換される。`[profile MyProfile]` が `myprofile` になり、AWS CLI の挙動と異なる。
- **修正案:** `Ini::new_cs()` (case-sensitive) を使用する。キーの小文字化は維持。

### 6. [Testing] ProfileResolver・shell_words_split にテストが一切ない
- **ファイル:** `src/profile/resolver.rs`
- **問題:** アプリケーションの中核ロジック（キャッシュ参照、プロファイル種別分岐、ロールチェーン再帰、credential_process コマンドパース）にテストがない。
- **修正案:** StsClient を trait 化しモックを導入。shell_words_split の境界値テストを追加。

---

## Major Issues

### 7. [Correctness] shell_words_split の末尾バックスラッシュ未処理
- **ファイル:** `src/profile/resolver.rs:399-447`
- **問題:** コマンド文字列が `\` で終わる場合、`escape_next` が true のままループ終了するがエラーにならない。
- **修正案:** ループ後に `if escape_next { return Err(...); }` を追加。

### 8. [Correctness] MFA 必須プロファイルのデーモンリフレッシュが無限リトライ
- **ファイル:** `src/autorefresh/daemon.rs:140-184`
- **問題:** `mfa_token: None` で resolve するため、MFA 必須プロファイルでは毎回エラーが発生し、60 秒ごとに永遠にリトライする。
- **修正案:** auto-refresh 開始時に MFA 必須かチェックし、該当する場合はエラーを返す。

### 9. [Security] Credentials 構造体の `#[derive(Debug)]` がシークレットを露出
- **ファイル:** `src/aws/credentials.rs:6`
- **問題:** `Debug` 実装で `secret_access_key` と `session_token` がそのまま出力される。
- **修正案:** `Debug` を手動実装し、シークレットフィールドを `[REDACTED]` でマスクする。

### 10. [Security] PID ファイルのパーミッション未設定 + PID 再利用攻撃
- **ファイル:** `src/autorefresh/daemon.rs:127-138`
- **問題:** PID ファイルがデフォルトパーミッション(0644)で作成される。攻撃者が PID を上書きし、`kill_daemon()` 経由で任意プロセスに SIGTERM を送信可能。
- **修正案:** PID ファイルを 0600 で作成。`kill_daemon()` でプロセス名検証を追加。

### 11. [Correctness] `kill` の PID キャストによるプロセスグループへのシグナル送信リスク
- **ファイル:** `src/autorefresh/daemon.rs:78`
- **問題:** `pid as i32` キャストで、不正な PID 値が負数になるとプロセスグループにシグナルが送られる。`kill` の戻り値も無視されている。
- **修正案:** キャスト前の範囲チェックと戻り値確認を追加。

### 12. [Concurrency] 履歴ファイルの read-modify-write に lost update リスク
- **ファイル:** `src/history/storage.rs:56-72`
- **問題:** 2 つの CLI プロセスが同時に `record_usage()` すると、一方の変更が失われる。
- **修正案:** write-to-temp-then-rename + flock による排他制御。

### 13. [Concurrency] デーモンにシグナルハンドリングがなく graceful shutdown 不可
- **ファイル:** `src/autorefresh/daemon.rs:140-184`
- **問題:** SIGTERM のハンドラがなく、キャッシュ書き込み中に中断されるとファイル破損の可能性。
- **修正案:** `tokio::signal::unix::signal(SignalKind::terminate())` + `tokio::select!` で graceful shutdown。

### 14. [Concurrency] エラー時のリトライに指数バックオフ・回数制限がない
- **ファイル:** `src/autorefresh/daemon.rs:140-184`
- **問題:** 恒久的エラー（アカウント停止等）でも 60 秒ごとに永遠にリトライし、AWS API への不要な負荷とログ増大が発生。
- **修正案:** 指数バックオフ（最大 30 分）+ 連続失敗上限 + 恒久的エラーは即終了。

### 15. [Performance] `System::new_all()` がプロファイル切り替えのたびにフルスキャン
- **ファイル:** `src/autorefresh/daemon.rs:115`
- **問題:** 全プロセス・CPU・メモリ情報を収集する重い操作が毎回実行される。
- **修正案:** `libc::kill(pid, 0)` + `/proc/{pid}/cmdline` で軽量にチェック。

### 16. [Maintainability] `run()` が God Function（約 160 行、全責務を集約）
- **ファイル:** `src/main.rs:69-230`
- **問題:** CLI 分岐、プロファイル解決、資格情報取得、シェル出力、履歴記録、デーモン起動を 1 関数に集約。
- **修正案:** clap の `Subcommand` を使用し、各アクションを個別の関数/モジュールに分離。

### 17. [Maintainability] `run()` と `resolve_role_arn_profile()` のロジック重複
- **ファイル:** `src/main.rs:325-409` と `src/main.rs:69-229`
- **問題:** ProfileResolver 生成、export コマンド生成、残り時間表示が完全に重複。
- **修正案:** 共通処理を `output_credentials()` に抽出。

### 18. [Maintainability] auto_refresh の条件ロジックが冗長で意図不明
- **ファイル:** `src/main.rs:220-227`
- **問題:** `if args.auto_refresh { ... || args.auto_refresh` — 内側の `|| args.auto_refresh` は常に true で、`profile.autoawswit` チェックが無意味。
- **修正案:** 条件を `if args.auto_refresh || profile.autoawswit.unwrap_or(false)` にフラット化。

### 19. [Compatibility] STS `duration_seconds` の範囲検証なし
- **ファイル:** `src/aws/sts.rs:56`
- **問題:** 負の値やゼロ、AWS の制限範囲外の値がそのまま API に送信される。
- **修正案:** `cli/args.rs` に `value_parser(clap::value_parser!(i32).range(900..=43200))` を追加。

### 20. [Testing] CacheManager の破損 JSON テストがない
- **ファイル:** `src/cache/manager.rs:43-64`
- **問題:** 破損キャッシュで `Err` を返す（`Ok(None)` ではない）ため、クレデンシャル取得が完全に失敗する。
- **修正案:** 破損ファイルは `Ok(None)` として graceful にフォールバック + テスト追加。

### 21. [Testing] shell export の session_token=None / expiration パスが未テスト
- **ファイル:** `src/shell/export.rs:63-191`
- **問題:** IAM ユーザー直接クレデンシャル（session_token なし）の頻出パスがテストされていない。

---

## Minor Issues

### 22. [Correctness] `dirs::home_dir()` が None の場合に無意味な `~` パスを使用
- **ファイル:** `src/config/aws_files.rs:17-18`
- **修正案:** 明確なエラーメッセージで早期リターン。

### 23. [Security] Profile 構造体の `#[derive(Serialize)]` が `aws_secret_access_key` を含む
- **ファイル:** `src/profile/types.rs:31`
- **修正案:** `#[serde(skip_serializing)]` を付与。

### 24. [Correctness] キャッシュの expiry 判定が二重化（CacheManager + Resolver）
- **ファイル:** `src/cache/manager.rs:57` / `src/profile/resolver.rs:77`
- **修正案:** 一方に統一。

### 25. [Compatibility] `atty` クレートが unmaintained（RUSTSEC-2024-0375）
- **ファイル:** `Cargo.toml:43`
- **修正案:** `std::io::IsTerminal` (Rust 1.70+) に置き換え。

### 26. [Compatibility] `sysinfo = "0.29"` が古い
- **ファイル:** `Cargo.toml:35`
- **修正案:** 0.30+ へ移行（ただし System::new_all() の除去で sysinfo 依存自体が不要になる可能性）。

### 27. [Compatibility] `credential_process` JSON の `Expiration` がナノ秒精度を含む可能性
- **ファイル:** `src/aws/credentials.rs:42-57`
- **修正案:** `to_rfc3339_opts(chrono::SecondsFormat::Secs, true)` で秒精度に固定。

### 28. [Security] CI の `softprops/action-gh-release@v1` がタグ固定でない
- **ファイル:** `.github/workflows/ci.yml`
- **修正案:** SHA 固定を推奨。

---

## 改善提案

1. **ファイル I/O の安全性強化**: `fs2` クレートが依存にあるが未使用。全てのファイル書き込みに write-to-temp-then-rename パターンと flock を導入する。
2. **auto-refresh 設計の再考**: デーモンがキャッシュのみ更新する現状では、環境変数ベースのシェル統合と噛み合わない。`credential_process` 統合か、シェルフック方式を検討。
3. **`autoawswit` バイナリと `AWSWIT_DAEMON_MODE` の統一**: 同じ機能に 2 つの入口があり、挙動も異なる。
4. **StsClient の trait 化**: テスタビリティ向上のため、trait 抽象化とモック導入。
5. **`colored` クレートの活用**: ハードコードされた ANSI エスケープを `colored` に統一し、`colors: false` 設定を尊重する。
6. **シェルタイプの `--shell` オプション追加**: `$SHELL` ベースの自動検出は信頼性が低いため、明示的な指定を可能にする。

---

## 推奨テスト

### Priority 1 (Critical)
- `shell_words_split`: 通常コマンド、クォート、エスケープ、末尾バックスラッシュ、空文字列
- `ProfileResolver::resolve_profile_inner`: StsClient モックによるキャッシュヒット/ミス、ロールチェーン深さ制限、各プロファイル種別の分岐
- `is_expired` 境界値: 残り 59 秒 / 60 秒 / 61 秒

### Priority 2 (Major)
- `generate_export_commands`: session_token=None の各シェル種別、expiration=Some の RFC3339 出力
- `CacheManager::get`: 破損 JSON ファイル、同一プロファイル上書き
- `to_credential_process_json`: expiration=Some、session_token=None のケース
- `HistoryStorage`: save/load ラウンドトリップ、sorted_profile_names の安定性

### Priority 3 (Major)
- daemon: save_pid / is_daemon_running のユニットテスト（tempdir 使用）
- kill_daemon: 不正 PID ファイル内容のハンドリング
- ShellType::from_name: 大文字入力、エッジケース

### Priority 4 (Minor)
- generate_unset_commands: 全シェルタイプでの変数 unset
- AwswitConfig: 不正 YAML のエラーハンドリング
- remaining_time: 0 秒 / 数秒 / 数分のフォーマット

---

## マージ判断

### **Block**

以下の理由によりマージをブロックします：

1. **クレデンシャル漏洩リスク**: キャッシュファイルが一時的に 0644 で作成され、AWS シークレットキーが他ユーザーに読み取られる可能性がある（Critical #1）
2. **auto-refresh が実質機能しない**: デーモンがキャッシュを更新しても環境変数は更新されず、ユーザーの期待と実際の動作が乖離している（Critical #3）
3. **シェルラッパーの eval 問題**: `--list-profiles` 等の出力が eval されてしまう（Critical #4）
4. **プロファイル名の大文字小文字が無視される**: AWS CLI との互換性が損なわれる（Critical #5）
5. **中核ロジックのテスト不在**: ProfileResolver にテストが一切ない（Critical #6）

最低限、Critical Issues #1, #4, #5 を修正し、#3 の制限をドキュメント化してからマージすべきです。
