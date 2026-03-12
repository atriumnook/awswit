# awswit `prot` branch - Multi-Perspective Code Review

## Summary

AWS CLI プロファイル切り替えツール `awswit` の Rust 実装（約9,300行の新規コード）。ロール連鎖、MFA、自動リフレッシュデーモン、TUI ピッカーを含む。全体的にセキュリティ意識が高く、INI インジェクション防止、アトミック書き込み、ロック順序の文書化など良い設計が見られる。以下は9名のレビュアーによる統合結果。

---

## Critical Issues

### C1: 非Linux Unixでのプロセス検証なしSIGTERM送信
**Reviewer:** Concurrency / Reliability
**File:** `src/autorefresh/daemon.rs:354-364`

macOS/BSDでは `/proc/{pid}/comm` が存在しないため、PIDファイルのPIDが実際に `autoawswit` プロセスかを検証せずに `verified = true` としてSIGTERMを送信する。PIDリサイクルが発生した場合、無関係なプロセスを終了させる可能性がある。

**修正案:** macOSでは `sysctl(KERN_PROCARGS2)` で検証を実装する。実装されるまでは非Linuxプラットフォームではkillを拒否し、手動でのPIDファイル削除を要求する。

### C2: デーモンのリトライに冪等性保証なし
**Reviewer:** AI Anti-Pattern
**File:** `src/autorefresh/runner.rs:74-151`

`refresh_all_profiles()` が途中で失敗した場合（クレデンシャル書き込み後、メタデータ更新前など）、リトライ時に不整合な状態が生じる。バックオフ付きリトライは実装されているが、操作の冪等性が保証されていない。

**修正案:** `refresh_profile()` に部分完了チェックを追加するか、クレデンシャル書き込みとメタデータ更新をトランザクション的に扱う。

---

## Major Issues

### M1: シェル出力フォーマットにバージョンネゴシエーションなし
**Reviewer:** API / Compatibility
**File:** `src/shell/export.rs:163-201`

Rustバイナリとシェルラッパー間の `KEY=VALUE\n` フォーマットにバージョン情報がない。フォーマット変更時にシェルラッパーが無言で壊れる。

**修正案:** 出力の先頭行に `AWSWIT_VERSION=<version>\n` を追加し、シェルラッパー側で互換性チェックする。

### M2: ロックファイルのシンボリックリンク攻撃への脆弱性
**Reviewer:** Security
**File:** `src/autorefresh/credentials_file.rs:72`, `src/autorefresh/daemon.rs:228`

`OpenOptions::new().write(true).create(true)` はシンボリックリンクをフォローする。攻撃者が `~/.aws/credentials.lock` にシンボリックリンクを配置した場合、任意ファイルが切り詰められる。

**修正案:** `O_NOFOLLOW` フラグの使用、またはオープン後にファイルがレギュラーファイルであることを検証する。

### M3: credentials ファイルのパーミッションチェック未実装
**Reviewer:** Security
**File:** `src/config/aws_files.rs:40-51`

config ファイルのworld-writable警告はあるが、credentials ファイルのパーミッションチェックがない。credentials ファイルは `0o600` であるべき。

**修正案:** credentials ファイルが `0o600` でない場合はエラーまたは強い警告を出す。

### M4: デーモンスポーン時のロック長期保持
**Reviewer:** Concurrency
**File:** `src/autorefresh/daemon.rs:241-308`

`spawn_autoawswit_daemon()` でデーモンロックを保持したまま最大5秒間のポーリングループに入る。複数の `awswit --auto-refresh` が同時実行されると後続の呼び出しがブロックされる。

**修正案:** スポーン後すぐにロックを解放するか、PIDファイル出現をinotify/FSEventsで監視する。

### M5: ロック順序の不整合
**Reviewer:** Concurrency
**File:** `src/autorefresh/runner.rs:346-471`

`daemon.rs` 先頭のコメントで「daemon lock → credentials lock」の順序を規定しているが、`refresh_profile()` はデーモンロックなしで credentials ロックのみ取得する。将来の変更でデッドロックの原因となりうる。

**修正案:** ドキュメントを実態に合わせて更新するか、`refresh_profile()` でもデーモンロックを取得する。

### M6: 自動リフレッシュプロファイルメタデータにバージョンなし
**Reviewer:** API / Compatibility
**File:** `src/autorefresh/daemon.rs:18-27`

`AutoRefreshProfile` JSON にバージョンフィールドがない。フィールド追加時に旧バージョンのファイルがデシリアライズに失敗する。

**修正案:** `version: i32` フィールドを追加し、マイグレーション関数を実装する。

### M7: コマンドリプレイのバージョン互換性なし
**Reviewer:** API / Compatibility
**File:** `src/autorefresh/daemon.rs:57-84`

自動リフレッシュ用に保存されたコマンドライン引数はバージョン情報なし。CLIフラグの変更時に保存済みコマンドが無言で失敗する。

**修正案:** コマンドにバージョンタグを付与し、リプレイ時にバージョンチェックとマイグレーションを行う。

### M8: ファイルロック作成コードの重複
**Reviewer:** Maintainability
**File:** `credentials_file.rs:60-78`, `daemon.rs:219-239`, `autoawswit.rs:37-56`

ロックファイル作成・取得・パーミッション設定が3箇所にコピペされている。

**修正案:** `utils/fs.rs` に `lock_file_with_permissions(path, timeout) -> Result<File>` を抽出し、全箇所で共用する。

### M9: PID検証の暗黙的フォールバック
**Reviewer:** AI Anti-Pattern
**File:** `src/autorefresh/daemon.rs:457-474`

`/proc/{pid}/comm` の読み取り失敗をすべて「プロセス消滅」として扱い、PIDファイルを削除する。権限不足やSELinux制限の場合、稼働中のプロセスを死亡と誤認する。

**修正案:** エラー種別を区別し、「検証不能 = 生存と仮定」の保守的戦略に変更する。

### M10: `refresh_profile()` のテスト不在
**Reviewer:** Testing
**File:** `src/autorefresh/runner.rs:346-471`

セキュリティ上最も重要な関数（任意コマンド実行防止のパス検証含む）にテストがない。

**修正案:** 許可コマンド名検証、パス正規化、ディレクトリパーミッション検証、タイムアウト処理のテストを追加する。

### M11: `credential_process` のタイムアウト/シグナルハンドリングテスト不在
**Reviewer:** Testing
**File:** `src/profile/resolver.rs:578-690`

SIGTERM→wait→SIGKILLの複雑なタイムアウトロジックにテストがない。

**修正案:** タイムアウトパス、不正なJSON出力、部分出力のテストを追加する。

### M12: ファジーマッチング実装の重複
**Reviewer:** Principles (DRY)
**File:** `src/utils/fuzzy.rs` vs `src/tui/picker.rs:431-483`

CLI用（prefix → LCS → Levenshtein）とTUI用（prefix → substring → Jaro-Winkler）で2つの独立したファジーマッチングがあり、結果が異なりうる。

**修正案:** 統一マッチャーに集約する。

---

## Minor Issues

| # | Reviewer | File | Issue |
|---|---------|------|-------|
| m1 | Correctness | credentials_file.rs:87-107 | `remove_credentials_section()` がセクションヘッダー前後の空白を考慮しない |
| m2 | Correctness | resolver.rs:720-736 | MFAトークン長 6-8 桁の根拠が未文書化 |
| m3 | Security | runner.rs:254 | プロファイル名のログ出力がインフラ情報を漏洩しうる |
| m4 | Security | resolver.rs:198-272 | クレデンシャルがメモリに長期保持される（zeroize未使用） |
| m5 | Performance | main.rs:119-121 | 1プロファイルしか必要でも全プロファイルをロードする |
| m6 | Performance | cache/manager.rs | MFAキャッシュがファイルベースのみ（インメモリキャッシュなし） |
| m7 | Maintainability | resolver.rs:141-276 | `resolve_role_chain()` が135行で複雑度が高い |
| m8 | Maintainability | daemon.rs, runner.rs | `AutoRefreshError` のエラーメッセージにプロファイル名等のコンテキスト不足 |
| m9 | API | runner.rs:164-190 | PIDファイルフォーマットが未文書化 |
| m10 | Testing | tests/fuzzy_matching.rs | 曖昧一致のタイブレーキングテストなし |
| m11 | Testing | shell/export.rs | UTF-8やロング値のシェル出力テストなし |
| m12 | Principles (KISS) | daemon.rs:322-406 | プラットフォーム分岐の深いネスト |
| m13 | Principles (DRY) | awswit_config.rs:100-144 | `set_value()` の反復的パース処理 |
| m14 | AI Anti-Pattern | shell/export.rs:41-43 | `AWS_SECURITY_TOKEN`（boto2用レガシー）の永続的後方互換コード |
| m15 | AI Anti-Pattern | picker.rs:110-123 | `name_lower` の事前計算は計測なし最適化 |
| m16 | AI Anti-Pattern | runner.rs:153-161 | PIDファイル削除失敗の例外握りつぶし |

---

## Suggested Improvements

1. `zeroize` クレートの導入でクレデンシャルのメモリ上のライフタイムを最小化
2. デーモンのシグナル/終了コード/PIDファイルフォーマットを DAEMON_PROTOCOL.md として文書化
3. プロファイル数が多い環境向けの遅延ロード実装
4. シェル出力フォーマットの統合テスト（実際のシェルでパースして検証）

---

## Recommended Tests

1. **`refresh_profile()` のセキュリティテスト** - 許可コマンド名、パス正規化、パーミッション検証
2. **`credential_process` タイムアウトテスト** - SIGTERM/SIGKILLフォールバック
3. **PID検証テスト** - stale PID検出、プロセスアイデンティティ確認
4. **シェル出力統合テスト** - 実シェルでの変数設定/解除検証
5. **ファジーマッチ曖昧一致テスト** - タイブレーキング動作の文書化テスト
6. **CRLF混在テスト** - credentials ファイルの混在改行コード処理

---

## Merge Decision

**Conditional**

Critical Issues C1（非Linuxでの誤プロセスkill）と C2（冪等性なしリトライ）を修正後にマージ可。Major Issues は後続PRで対応可だが、M2（シンボリックリンク攻撃）とM3（credentialsパーミッションチェック）はセキュリティ上早期対応を推奨。
