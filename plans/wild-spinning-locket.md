# awswit マルチパースペクティブ コードレビュー統合レポート

**対象:** `prot` ブランチ全変更 (main比 +9,326行, 57ファイル, 34コミット)
**レビュアー:** 9名の専門サブエージェント (Correctness, Security, Performance, Concurrency, Maintainability, API/Compatibility, Testing, Principles Guardian, AI Anti-Pattern)

---

## Summary

awswitはAWSプロファイル切り替えCLIツールのRust実装。バックグラウンドデーモンによる認証情報自動更新、MFA対応、ロールチェーン解決、TUIプロファイルピッカーを備える。コードベース全体のセキュリティ意識は高い（INIインジェクション防止、アトミック書き込み、ファイル権限管理）。主な懸念はデーモンライフサイクル管理の競合状態、ProfileResolverの複雑性、テストカバレッジ不足に集中している。

---

## Critical Issues

### C1. デーモン起動時のfork/PID確認競合 [Correctness + Concurrency]
- **ファイル:** `src/bin/autoawswit.rs:9-25`, `src/autorefresh/daemon.rs:273-285`
- **問題:** 親プロセスはfork後にPIDファイルのポーリングで子の起動を確認するが、子がロック取得前にクラッシュした場合、親はexit(0)で「成功」を返す。PIDファイルポーリングはタイミング依存で信頼性が低い。
- **影響:** 自動更新が開始されたと表示されるが、実際にはデーモンが動いていない（サイレント障害）
- **修正案:** パイプによる親子ハンドシェイク。子がロック取得・初期化成功後にパイプ経由で親に通知。

### C2. デーモンロックのTOCTOU: stop_auto_refresh [Concurrency]
- **ファイル:** `src/autorefresh/daemon.rs:230-248`
- **問題:** `stop_auto_refresh()`でプロファイル削除後にremaining確認→daemon kill判断するが、ロック範囲が不十分。kill操作中にロックが解放される可能性あり。
- **影響:** 別プロセスのstart_auto_refreshとの競合でデーモンが起動直後にkillされうる
- **修正案:** daemon kill完了までロックを保持

### C3. SIGKILL後のプロセス回収不足 [Concurrency]
- **ファイル:** `src/autorefresh/daemon.rs:375-397`
- **問題:** SIGKILL送信後200msのsleepのみでwaitpid未実施。ゾンビプロセス化やPID再利用リスク。
- **修正案:** waitpid(WNOHANG)ループで確実に回収

### C4. AWS STS session_name未検証 [API/Compatibility]
- **ファイル:** `src/profile/resolver.rs:238`, `src/cli/args.rs:137-144`
- **問題:** ユーザー指定・設定由来のセッション名がAWS STS制約（2-64文字、特定文字種）を満たさない可能性。STS APIエラーとなり原因特定が困難。
- **修正案:** `validate_session_name()`関数を実装し、全ソースに適用

---

## Major Issues

### M1. async コンテキストでのブロッキングsleep [Performance + Concurrency]
- **ファイル:** `src/utils/fs.rs:97-131`
- **問題:** `lock_exclusive_with_timeout()`が`std::thread::sleep()`を使用。asyncデーモンから呼ばれた場合、tokioエグゼキュータスレッド全体をブロック。
- **修正案:** `tokio::task::spawn_blocking()`でラップ、または非同期版ロック関数を提供

### M2. 認証情報書き込みとメタデータ更新のアトミック性欠如 [Correctness + Concurrency]
- **ファイル:** `src/autorefresh/runner.rs:540-548`
- **問題:** credentials_file書き込み成功後にメタデータ更新が失敗すると、次回リフレッシュで冪等性チェックが古いメタデータを参照し、正当な更新をスキップする可能性。
- **修正案:** 両操作を単一ロック下で実行

### M3. プロファイルJSON削除がロック保護外 [Concurrency]
- **ファイル:** `src/autorefresh/runner.rs:219-235`
- **問題:** expired_profilesのJSONファイル削除がロックなし。credentials_file操作はロック内だが不整合。
- **修正案:** daemon lockまたはcredentials lockの範囲を拡大

### M4. ProfileResolverの過大責務（986行） [Maintainability + SOLID]
- **ファイル:** `src/profile/resolver.rs` 全体
- **問題:** ロールチェーン解決、認証情報取得、STS操作、キャッシュ管理、MFAトークン入力を1つの構造体に集約。テスタビリティと変更耐性が低い。
- **修正案:** RoleChainResolver / CredentialSourceResolver / MfaHandler に分割

### M5. resolve_role_chain() のパラメータ解決ロジック重複 [DRY]
- **ファイル:** `src/profile/resolver.rs:116-178`, `src/profile/resolver.rs:231-259`
- **問題:** role_duration解決ロジックが3箇所に重複。session_name/region/external_idの解決もfinal_hop/intermediate_hopで類似コード。
- **修正案:** `resolve_role_duration()`ヘルパー、`HopParameters`構造体の導入

### M6. runner.rsのプロファイルメタデータ二重読み込み [Performance]
- **ファイル:** `src/autorefresh/runner.rs:512-572`
- **問題:** 冪等性チェックで1回、更新で1回、同じJSONファイルを2回読み込み。リフレッシュサイクル毎に発生。
- **修正案:** 1回の読み込みで両方の処理を実行

### M7. TUIでProfileを全フィールドclone [Performance]
- **ファイル:** `src/tui/picker.rs:111-124`
- **問題:** ProfileEntry作成時に全Profileをclone。15+のStringフィールド × プロファイル数の不要アロケーション。
- **修正案:** `&'a Profile`参照に変更

### M8. 非LinuxでのPID再利用検出不能 [Concurrency]
- **ファイル:** `src/autorefresh/daemon.rs:451-520`
- **問題:** macOS/BSDでは/proc未対応でverify_process_identityがNone返却。kill(pid,0)のみでプロセス同一性確認不可。PID再利用時に誤判定。
- **修正案:** PIDファイルに起動時刻を記録し照合、またはソケットベースのデーモン検出

### M9. リトライにジッター未実装 [AI Anti-Pattern: 雑なリトライ]
- **ファイル:** `src/autorefresh/runner.rs:127-138`
- **問題:** 指数バックオフにジッターなし。複数デーモンインスタンスが同期的にリトライしthundering herd発生の可能性。
- **修正案:** `backoff += random(0..backoff/2)` を追加

### M10. role_duration の上限値検証なし [API/Compatibility]
- **ファイル:** `src/aws/sts.rs:94-95`
- **問題:** 任意の巨大値をSTSに渡せる。APIエラーは不明瞭。
- **修正案:** クライアント側で43200秒（12時間）上限バリデーション

### M11. exit code 0 でユーザーキャンセル [API/Compatibility]
- **ファイル:** `src/main.rs:40-46`
- **問題:** Ctrl+Cキャンセル時にexit(0)を返す。スクリプトからは成功と区別不能。
- **修正案:** exit code 130（SIGINT慣例）を使用

### M12. CRLF→LF変換によるファイル形式変更 [Correctness]
- **ファイル:** `src/autorefresh/credentials_file.rs:71-93`
- **問題:** `content.lines()`でCRLFが消失し`\n`で再構築。Windows環境でファイル形式が変わる。
- **修正案:** 元の改行文字を検出・保持

### M13. ProfileResolverにMFAトークン入力I/Oが混在 [Maintainability + SOLID]
- **ファイル:** `src/profile/resolver.rs:420-435`
- **問題:** ドメインロジック内でdialoguer::Inputを直接呼出。ユニットテスト不可能。
- **修正案:** `MfaTokenProvider`トレイトを導入しDI

### M14. プロセス終了ロジックの重複 [DRY]
- **ファイル:** `src/profile/resolver.rs:625-645`, `src/autorefresh/daemon.rs:376-395`
- **問題:** SIGTERM→SIGKILL のgraceful終了パターンが2箇所に重複。
- **修正案:** `utils/process.rs`に共通関数を抽出

---

## Minor Issues

### m1. lock timeout 30秒がハードコード [Correctness]
- `src/autorefresh/credentials_file.rs:55` — 環境変数かconfigで設定可能に

### m2. shell_quote("")の空文字列テスト不足 [Correctness]
- `src/shell/export.rs:3-6`

### m3. MFAトークンエラーメッセージが不明瞭 [Correctness]
- `src/profile/resolver.rs:733` — "ASCII digits 0-9のみ"と明示すべき

### m4. stale PIDファイル削除がロック外 [Correctness]
- `src/autorefresh/daemon.rs:438-449`

### m5. AWS_SECURITY_TOKEN（boto2レガシー）の無期限互換 [AI Anti-Pattern]
- `src/shell/export.rs:41-44` — 廃止期限を設定するか削除

### m6. get_source_credentials()に未使用パラメータ [AI Anti-Pattern: 過剰抽象]
- `src/profile/resolver.rs:320-361`

### m7. 壊れたプロファイルJSONをWARNで黙殺 [AI Anti-Pattern: silent fallback]
- `src/autorefresh/runner.rs:297-304` — 壊れたファイルはエラーにすべき

### m8. unwrap_or_default()で空エラーメッセージ [AI Anti-Pattern]
- `src/autorefresh/runner.rs:269` — "Unknown error"をデフォルトに

### m9. hex encodingの逐次アロケーション [Performance]
- `src/profile/resolver.rs:16-22`, `src/cache/manager.rs:133-138`

### m10. expired_profilesのVec contains検索 [Performance]
- `src/autorefresh/runner.rs:237-241` — HashSetに変更

### m11. INI設定ファイルのcase-sensitive解析 [API/Compatibility]
- `src/config/aws_files.rs:57-61` — AWS CLIはcase-insensitive

### m12. Fish shellスクリプトの空値ハンドリング不足 [API/Compatibility]
- `src/init/fish.fish:11-39`

### m13. ProfileEntry.scoreフィールドが表示未使用 [YAGNI]
- `src/tui/picker.rs:32-40`

### m14. fuzzy matchでlowercaseが毎回再計算 [Performance]
- `src/utils/fuzzy.rs:63-90`

### m15. credential_processタイムアウト時のゾンビプロセス（非Unix） [Security]
- `src/profile/resolver.rs:623-658`

### m16. 孤立tmpファイルのクリーンアップ機構なし [Concurrency]
- `src/utils/fs.rs` 全体

### m17. プロファイル名のバリデーションが厳格すぎる [API/Compatibility]
- `src/autorefresh/credentials_file.rs:14-27` — AWSはスペース含む名前を許容

### m18. PowerShellラッパーのexit code未伝播 [API/Compatibility]
- `src/init/powershell.ps1:12-14`

---

## Recommended Tests (テスト不足の優先対応)

### Critical
1. デーモンspawn/kill ライフサイクル統合テスト
2. ロック競合・タイムアウトテスト
3. atomic_write_restricted 失敗パステスト
4. STS タイムアウトシミュレーション

### Major
5. MFA付きロールチェーン統合テスト（duration > 1時間含む）
6. credential_process 実行・タイムアウトテスト
7. INIインジェクション: 全制御文字(0x00-0x1F)テスト
8. キャッシュ権限(0o700)検証テスト
9. AutoRefreshProfile バージョン互換テスト

### Minor
10. shell_quote 空文字・連続シングルクォートテスト
11. fuzzy match 空リスト・超長名テスト
12. SdkError全バリアント網羅テスト

---

## Merge Decision: **Conditional**

### Merge条件（Critical修正必須）
1. **C1** デーモン起動ハンドシェイク修正（パイプ方式）
2. **C2** stop_auto_refreshのロック範囲拡大
3. **C3** SIGKILL後のwaitpid実装
4. **C4** session_nameバリデーション追加

### 強く推奨（次リリースまでに対応）
- M1 asyncブロッキング解消
- M2 credentials+メタデータのアトミック操作
- M11 exit code修正
- M12 CRLF保持

### 中期対応
- M4/M5/M13/M14 のリファクタリング（ProfileResolver分割、重複排除）
- テストカバレッジ拡充（推定50-60件追加）
