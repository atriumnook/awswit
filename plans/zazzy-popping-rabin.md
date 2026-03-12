# コードレビュー指摘修正計画

## Context

マルチモデルコードレビューで20件の指摘事項が検出された。Critical 4件、Major 8件、Minor 8件。
MFAキャッシュが完全に機能しない致命的バグ、INIインジェクション脆弱性、ロック周りの信頼性問題が最優先。
クレデンシャルファイル操作の重複（daemon.rs / runner.rs）を共有モジュールに統合し、バリデーション欠如も同時に解消する。

---

## Phase 1: Critical Fixes

### 1A: MFAキャッシュキー不整合 (Critical #1)

**ファイル:** `src/profile/resolver.rs`

キャッシュ取得(364-367行)はハッシュ形式、保存(390-392行)は生文字列。キーが一致しないためキャッシュが常にミスする。

**修正:**
1. `mfa_cache_key(access_key_id, mfa_serial) -> String` ヘルパー関数を追加（ハッシュ形式で統一）
2. 取得・保存の両方でこの関数を使用
3. ユニットテスト追加（同一入力で同一キー、異なる入力で異なるキー）

### 1B: INIインジェクション + クレデンシャル操作の統合 (Critical #2 + Major #8)

**ファイル:** `src/autorefresh/daemon.rs`, `src/autorefresh/runner.rs`, 新規 `src/autorefresh/credentials_file.rs`

daemon.rs の `write_auto_refresh_credentials_at_path()` にプロファイル名・値のバリデーションがない。
runner.rs には `validate_profile_name()` (414-430行) と値バリデーション (443-447行) がある。
ほぼ同一のINI操作が2箇所に存在。

**修正:**
1. `src/autorefresh/credentials_file.rs` を作成:
   - `validate_profile_name()` — runner.rs から移動、AwswitError返却に変更
   - `validate_credential_value()` — runner.rs から抽出
   - `write_credentials()` — バリデーション付き統一書き込み関数
   - `remove_credentials()` — セクション削除
   - `lock_aws_credentials_file()` — ロック取得（内部使用）
   - `get_aws_credentials_path()` — パス解決
2. daemon.rs から重複ロジックを削除、`credentials_file::write_credentials()` を使用
3. runner.rs から重複ロジックを削除、`credentials_file::write_credentials()` を使用
4. INIインジェクションテスト追加

### 1C: デーモンスポーン時のロック競合 (Critical #3)

**ファイル:** `src/autorefresh/daemon.rs`

spawn後にロック解放されるが、子プロセスのPIDファイル書き込み前に解放される。

**修正:**
- spawn後、PIDファイル出現をポーリング（最大5秒、100msインターバル）
- PIDファイル確認後にロック解放（関数return時のDrop）

### 1D: ロックタイムアウト欠如 (Critical #4)

**ファイル:** `src/utils/fs.rs`, `src/autorefresh/daemon.rs`, `src/autorefresh/runner.rs`

`lock_exclusive()` が無期限ブロック。プロセスクラッシュ時に全操作がハング。

**修正:**
1. `src/utils/fs.rs` に `lock_with_timeout(file, timeout) -> io::Result<()>` を追加
   - `try_lock_exclusive()` + 指数バックオフ（10ms→1s）+ タイムアウト（30秒）
2. 全 `lock_exclusive()` 呼び出しを置換

---

## Phase 2: Major Fixes

Phase 1完了後、各項目は独立して実施可能。

### 2A: Zsh自動補完修正 (Major #5)

**ファイル:** `src/shell/autocomplete.rs` (48行)

`#compdef awswit awswit` → `#compdef awswit` に修正し、関数定義後に `compdef _awswit_rs awswit` を追加。

### 2B: AWS_PROFILE unset の意図文書化 (Major #6)

**ファイル:** `src/shell/export.rs` (57-62行)

意図的な設計判断（直接クレデンシャルエクスポートとの競合回避）。コードコメントで理由を記載。機能変更なし。

### 2C: クレデンシャル+メタデータの更新順序 (Major #7)

**ファイル:** `src/autorefresh/runner.rs`

メタデータを先に書き込む（クラッシュ時に自己修復可能な方向）。メタデータも `atomic_write_restricted` を使用。

### 2D: ファジーマッチ空間最適化 (Major #10)

**ファイル:** `src/utils/fuzzy.rs` (103-123行)

2D DPテーブル → 2行ローリング配列に変換。O(m*n) → O(n) 空間。既存テスト `tests/fuzzy_matching.rs` で検証。

### 2E: デーモンファイル再読み込み最適化 (Major #11)

**ファイル:** `src/autorefresh/runner.rs` (248-276行)

ファイルのmtimeを記録し、変更があった場合のみ再パース。

### 2F: リトライロジックの改善 (Major #12)

**ファイル:** `src/autorefresh/runner.rs` (67-102行)

プロファイル単位の失敗カウントとバックオフを導入。全プロファイル失敗時のみ全体バックオフ。

### 2G: ProfileResolver分離 (Major #9)

Phase 2の他の修正完了後に検討。現時点ではスキップ（リファクタリングリスク大、機能的問題なし）。

---

## Phase 3: Minor Fixes (バッチ処理可能)

### 3A: PowerShell .exe 拡張子 (Minor #14)
`src/init/powershell.ps1:7` — `awswit.exe` → `awswit`

### 3B: AWS SDK バージョンピン (Minor #15)
`Cargo.toml:31-34` — `=1.1.1` → `^1.1.1` 等。`cargo update` + テスト実行で検証。

### 3C: 未使用CLIフラグ (Minor #16)
`src/cli/args.rs:75-81` — `#[arg(hide = true)]` を追加

### 3D: Dead Code (Minor #17)
- `src/tui/picker.rs:39` — `_use_count` フィールド削除
- `src/tui/spinner.rs:88-127` — `MultiStepProgress` 構造体削除

### 3E: home_dir フォールバック (Minor #19)
`src/config/awswit_config.rs:55-56` — `config_path()` を `Result` 返却に変更

### 3F: STS リージョンフォールバック (Minor #20)
`src/aws/sts.rs:22-29` — `eprintln!` 警告を追加

### 3G: 非Linux プロセス検証 (Minor #13)
`src/autorefresh/daemon.rs:475-478` — `tracing::debug!` で検証なしの旨を記録

### 3H: 履歴保存エラー (Minor #18)
`src/main.rs:191-193` — 現状の `tracing::warn!` で十分。変更なし。

---

## 実施順序と依存関係

```
1A (resolver.rs) ─── 独立、最初に実施
1B (credentials_file.rs新規 + daemon.rs + runner.rs リファクタ)
  └→ 1C (daemon.rs spawn修正) ─ 1Bの後
  └→ 1D (lock timeout) ─ 1Bの後
2A〜2F ─── Phase 1完了後、互いに独立
3A〜3H ─── いつでも可能、1-2コミットにバッチ化
```

## 検証方法

1. 各Phase後に `cargo test` + `cargo clippy` 実行
2. Phase 1A: MFAキャッシュキーの一貫性テスト（新規追加）
3. Phase 1B: INIインジェクションテスト（新規追加）+ 自動リフレッシュの手動テスト
4. Phase 2D: `tests/fuzzy_matching.rs` 既存テストで回帰確認
5. Phase 3B: `cargo update` 後の全テスト通過確認
