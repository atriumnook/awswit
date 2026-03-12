# awswit マルチパースペクティブ コードレビュー統合レポート

## Summary

awswit は AWS プロファイル切り替え CLI ツール（Rust実装）。MFA セッション管理、ロールチェーン解決、クレデンシャル自動更新デーモン、TUI プロファイル選択を提供する。全体として堅実なセキュリティ設計（0o600パーミッション、atomic write、INI injection 防止）を持つが、デーモン管理の並行性、テストカバレッジ、一部の互換性に改善の余地がある。

---

## Critical Issues

### C1. デーモンスポーン時のレース条件
- **ファイル**: `src/autorefresh/daemon.rs:271-330`
- **レビュアー**: Concurrency, Maintainability, Principles Guardian
- **問題**: `spawn_autoawswit_daemon()` はロック下で `is_autoawswit_running()` → spawn を行うが、PIDファイルが5秒以内に書かれない場合、ロックを解放して `Ok(())` を返す。後続の呼び出し元が重複デーモンをスポーンする可能性。
- **修正案**: PIDファイルが出現しなければスポーンしたプロセスを kill してエラーを返す。ロックはPIDファイル確認まで保持する。

### C2. `credential_source` の制限が既存ユーザーを破壊
- **ファイル**: `src/profile/types.rs:4`
- **レビュアー**: API/Compatibility
- **問題**: `VALID_CREDENTIAL_SOURCES` が `["Environment"]` のみに制限。`Ec2InstanceMetadata` や `EcsContainer` を使う既存プロファイルがエラーになる。
- **修正案**: (a) EC2/ECS メタデータをサポートする、または (b) CHANGELOG で破壊的変更として明記し、メジャーバージョンを上げる。

### C3. テストカバレッジの重大な欠落
- **ファイル**: `src/profile/resolver.rs`, `src/autorefresh/runner.rs`, `src/autorefresh/credentials_file.rs`
- **レビュアー**: Testing
- **問題**: コアロジック（ロールチェーン解決、デーモンループ、クレデンシャルファイル操作）にテストがほぼ無い。resolver は3テスト、runner は0テスト、credentials_file は5テストのみ。
- **修正案**: 以下を優先的に追加:
  - ロールチェーン: サイクル検出、マルチホップ、MFA付きAssumeRole
  - デーモン: SIGTERM シャットダウン、バックオフスケーリング、期限切れクリーンアップ
  - クレデンシャルファイル: 空ファイル、CRLF、バッチ削除、ロック競合

---

## Major Issues

### M1. PID再利用によるデーモン誤検知
- **ファイル**: `src/autorefresh/daemon.rs:433-493`
- **レビュアー**: Security, Concurrency
- **問題**: `is_autoawswit_running()` は `kill(pid, 0)` → `/proc/{pid}/comm` の順で確認するが、この間にPIDが再利用される可能性。非Linux Unix では名前検証なし。
- **修正案**: `/proc/{pid}/comm` を先に確認し、その後 `kill(0)` で存在確認。プロセス開始時刻の比較も検討。

### M2. デーモンループの重複コード
- **ファイル**: `src/autorefresh/runner.rs:41-169`
- **レビュアー**: Maintainability, DRY, AI Anti-Pattern
- **問題**: `run_daemon_loop_unix()` と `run_daemon_loop_fallback()` は90%以上の重複。差分は SIGTERM ハンドリングのみ。
- **修正案**: 共通のリフレッシュ/バックオフロジックを関数に抽出し、シグナル処理のみをプラットフォーム分岐。

### M3. キャッシュキーの衝突リスク
- **ファイル**: `src/cache/manager.rs:184-188`
- **レビュアー**: Correctness, Security
- **問題**: `cache_file_path()` は特殊文字を `_` に置換。`"role/dev"` と `"role_dev"` が同じファイルに衝突する。
- **修正案**: SHA256 ハッシュをファイル名に使用する（`mfa_cache_key` と同じアプローチ）。

### M4. `credential_process` のタイムアウト時 SIGKILL でゾンビプロセス
- **ファイル**: `src/profile/resolver.rs:607-625`
- **レビュアー**: Correctness, Concurrency
- **問題**: タイムアウト時に `SIGKILL` を送るが、`waitpid()` を呼ばないためゾンビプロセスが残る。SIGTERM → 待機 → SIGKILL のグレースフル手順がない。
- **修正案**: SIGTERM → 1秒待機 → SIGKILL → waitpid() のシーケンスに変更。

### M5. ネストしたロック取得によるデッドロックの可能性
- **ファイル**: `src/autorefresh/daemon.rs:332-424`
- **レビュアー**: Concurrency
- **問題**: `kill_autoawswit_daemon()` がデーモンロック → クレデンシャルロックの順で取得。デーモンはリフレッシュ中にクレデンシャルロックを保持する可能性。ロック順序が不整合。
- **修正案**: ロック順序を文書化して統一。kill 操作ではデーモンロック解放後にクレデンシャルクリーンアップを実行。

### M6. シェルラッパーの不整合
- **ファイル**: `src/init/bash.sh` vs `shell_scripts/awswit.sh`
- **レビュアー**: API/Compatibility
- **問題**: 2つの bash ラッパーが異なる関数名（`awswit()` vs `_awswit()`）と構造を持つ。どちらが正式か不明。
- **修正案**: 正式版を1つに統合し、もう一方は削除またはシンボリックリンク。

### M7. オートコンプリートと CLI フラグの不整合
- **ファイル**: `src/shell/autocomplete.rs`, `src/cli/args.rs`
- **レビュアー**: API/Compatibility
- **問題**: オートコンプリートスクリプトが未実装フラグ（`--with-saml`, `--with-web-identity`）を含む。シェル間でフラグ一覧も不整合。
- **修正案**: clap_complete で自動生成するか、手動で4シェル分を同期。

### M8. プロファイルリフレッシュが逐次実行
- **ファイル**: `src/autorefresh/runner.rs:262-272`
- **レビュアー**: Performance
- **問題**: デーモンが複数プロファイルを逐次リフレッシュ。5プロファイル × 30秒 = 最大150秒。
- **修正案**: `futures::future::join_all()` で並行リフレッシュ（並行数制限付き）。

### M9. MFA処理ロジックの重複
- **ファイル**: `src/profile/resolver.rs:84-138, 392-428, 472-527`
- **レビュアー**: Principles Guardian (DRY)
- **問題**: MFAトークン抽出・検証・キャッシュキー生成が複数メソッドで重複。
- **修正案**: `MfaContext` 型に一元化。

### M10. シェルエクスポートのプラットフォーム別テスト不足
- **ファイル**: `src/shell/export.rs`
- **レビュアー**: Testing
- **問題**: Cmd.exe エスケープ、PowerShell 特殊文字、Fish ユニバーサル変数のテストが欠如。
- **修正案**: 各シェル × 特殊文字の組み合わせテストを追加。

---

## Minor Issues

### m1. `ptr::eq` による最終ホップ判定の脆弱性
- **ファイル**: `src/profile/resolver.rs:220-221`
- **問題**: `std::ptr::eq(*role_profile, target_profile)` はHashMapへの参照が同一であることに依存。現状動作するが脆弱。`i == chain.len() - 1` のみで十分。

### m2. AWS リージョンのサイレントフォールバック
- **ファイル**: `src/aws/sts.rs:36-42`
- **問題**: リージョン未設定時に us-east-1 にサイレントフォールバック。GovCloud/中国リージョンで問題。

### m3. PIDファイル削除エラーの握りつぶし
- **ファイル**: `src/autorefresh/runner.rs:116-118`
- **問題**: `let _ = fs::remove_file(pid_path)` でエラー無視。次回デーモン起動に影響。

### m4. キャッシュ破損のサイレント回復
- **ファイル**: `src/cache/manager.rs:69-78`
- **問題**: JSON パースエラーを warn ログのみでキャッシュミス扱い。組織的な破損が検知不能。

### m5. Profile 構造体の未使用フィールド
- **ファイル**: `src/profile/types.rs:34-35`
- **問題**: `manager` と `awswit_cache_name` が未使用の可能性。YAGNI違反。

### m6. ファジーマッチの曖昧なタイブレーク
- **ファイル**: `src/utils/fuzzy.rs`
- **問題**: LCS/Levenshtein 同スコア時に None を返すが、テスト不足。

### m7. TUI のキー入力ごとの全プロファイル Jaro-Winkler 計算
- **ファイル**: `src/tui/picker.rs:432-484`
- **問題**: デバウンスなしで全プロファイルをスコアリング。100プロファイル程度なら実用上問題ないが、インクリメンタルフィルタリングで改善可能。

### m8. `get_source_credentials()` の未使用パラメータ
- **ファイル**: `src/profile/resolver.rs:322-328`
- **問題**: `_args`, `_sts_client`, `_cache_manager` が未使用。

### m9. History のコマンドによらない無条件ロード
- **ファイル**: `src/main.rs:133-137`
- **問題**: `--show-commands` など非対話コマンドでも History をロード。

### m10. LCS の O(n) スペース最適化
- **ファイル**: `src/utils/fuzzy.rs:104-127`
- **問題**: 計測なしのマイクロ最適化。標準的 DP テーブルの方が可読性高い。

### m11. エラーコードの非連続性
- **ファイル**: `src/error.rs`
- **問題**: エラーコードに欠番あり（E024-E025 → E026-E027）、E099 キャッチオール。

---

## Suggested Improvements

1. **ロック順序の文書化**: daemon lock → credentials lock の順序をモジュールレベルで明記
2. **clap_complete 導入**: オートコンプリートスクリプトを自動生成してフラグ不整合を根絶
3. **ProfileResolver の分割**: role_chain, mfa_handler, credential_source に分割して可読性向上
4. **デーモンリフレッシュの並行化**: `join_all` + セマフォで I/O バウンドな STS 呼び出しを並行実行
5. **キャッシュ形式のバージョニング**: アップグレード時のキャッシュ無効化を graceful に処理

---

## Recommended Tests (優先順)

### 最優先
1. ロールチェーン解決: サイクル検出、マルチホップ、MFA 付きチェーン
2. デーモンループ: SIGTERM、バックオフ、期限切れクリーンアップ
3. クレデンシャルファイル: 空ファイル、CRLF、バッチ削除、並行ロック

### 高優先
4. シェルエクスポート: Cmd.exe/PowerShell/Fish 特殊文字エスケープ
5. キャッシュ: 並行アクセス、キー衝突、破損ファイル
6. 設定パース: 不正形式 INI、重複セクション、SSO プロファイル

### 中優先
7. ファジーマッチ: 空入力、Unicode、タイブレーク境界
8. MFA トークン: スペース入力、全角数字
9. CLI 引数: 空文字列、不正ショートハンド

---

## Merge Decision

**Conditional**

ブロッカー（マージ前に修正必須）:
- **C1**: デーモンスポーンのレース条件修正
- **C3**: コアロジックの最低限のテスト追加（ロールチェーン、デーモン）
- **M7**: オートコンプリートの未実装フラグ除去

推奨（マージ後でも可）:
- C2: credential_source 制限の文書化または実装
- M2-M5: デーモン関連の並行性改善
- M6: シェルラッパー統合
- 残りの Major/Minor issues
