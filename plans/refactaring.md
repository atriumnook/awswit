# awswit v2 — 実装プロンプト

---

## Claude Code 運用指示（全タスク共通・最初に渡す）

```text
あなたは Rust 製 AWS CLI ツール awswit のリファクタリング・機能追加を行います。
リポジトリは /home/claude/awswit にあります。

## 最重要原則
- 既存の外部仕様を壊さない（CLI オプション / 出力形式 / exit code / shell_scripts/ / init/）
- 変更理由が説明できない抽象化を入れない
- 新規本番コードに unwrap()/expect() 禁止（テスト除く）
- エラー握りつぶし禁止。silent fallback 禁止
- #[allow(dead_code)] 新規追加禁止
- 「今使わないが将来使うかも」のコード禁止
- pub 可視性は最小限。clone 乱用禁止
- 既存の安全策（atomic write / lock ordering / .corrupt 退避）を退化させない

## 作業手順（全タスク共通）
1. 変更対象ファイルを読んで現状把握（grep で呼び出し元・テストも特定）
2. 元計画と現実コードに差分があれば先に報告してから実装
3. 実装
4. 既存テスト修正 → 新規テスト追加
5. cargo fmt && cargo build && cargo test && cargo clippy -- -D warnings
6. 完了報告: 変更ファイル一覧 / 実装要点 / 追加テスト / 互換性影響 / 残懸念

## このリポジトリの現状（必ず認識すること）
- main.rs が責務過多（config/profiles/history 読込〜picker〜resolver〜autorefresh まで）
- ProfileResolver は StsClient / CacheManager の具象型に直接依存
- HistoryEntry は既に use_count / last_used / is_favorite を持つ（新スキーマ不要）
- AwswitConfig は colors / fuzzy-match / role-duration / region / role-session-name / session-token-duration の6フィールド + set_value/get_value/reset_value/save メソッドを持つ
- history 破損時に .json.corrupt へ退避する既存挙動がある
- error.rs に YamlError(#[from] serde_yml::Error) がある
- StsClient::new().await は run() 内で「STS が必要な場合のみ」遅延生成されている
- run() -> Result<(), AwswitError> で、main が UserCancelled→130, 他→1 で exit
- autorefresh の daemon lock / credentials lock に明示的なロック順序がある
- atomic_write_restricted は tmp+rename+O_NOFOLLOW+0o600+parent sync の堅牢実装
```

---

## 依存グラフと実行順序

```text
Phase 0-1 (trait + AppContext)  ← 最初に実行。他の全てに先行
    │
    ├─→ Phase 0-2 (CredentialStore trait)
    │
    ├─→ Phase 1-1 (YAML→TOML)          ─┐
    ├─→ Phase 1-2 (TUI依存統合)         │
    ├─→ Phase 1-3 (fs2→fd-lock)        ├── 全て並行可能
    ├─→ Phase 1-4 (uuid削除+completions)│
    ├─→ Phase 4-1 (エラー型統合)        │
    ├─→ Phase 5-1 (tokio features)      ─┘
    │
    ├─→ Phase 2-1 (nucleo-matcher)
    ├─→ Phase 3-1 (exec)  ← exit code 設計を含む
    ├─→ Phase 3-2 (Frecency)  ← 既存 use_count/last_used を活用
    │       │
    │       └─→ Phase 3-3 (--fzf)  ← 3-2 の frecency ソートを使用
    │
    └─→ Phase 6-1 (統合テスト)  ← 全 Phase 完了後
```

---

# Phase 0-1: AppContext + StsOperations trait

```text
## 目的
テスタビリティと責務分離。StsOperations trait 抽出 + AppContext 導入。

## 変更1: StsOperations trait
- src/aws/traits.rs 新規作成
- assume_role / get_session_token の2メソッドを持つ trait
- async-trait crate 追加。impl StsOperations for StsClient
- ProfileResolver: &StsClient → &dyn StsOperations

## 変更2: AppContext
- src/context.rs 新規作成
- 具象型で保持: Args, AwswitConfig, HashMap<String, Profile>, ProfileHistory, CacheManager
- AppContext::build(args) で config/profiles/history/cache_manager のロード処理を集約
- ★ STS client は AppContext に入れない。遅延生成を維持する
- run() は AppContext::build → コマンドディスパッチの流れへ。目安80行以内、超える場合は理由報告

## 受け入れ基準
1. cargo build/test/clippy -- -D warnings 全通過
2. StsOperations が object-safe（&dyn で使える）
3. MockStsClient を使い、単一ホップ role chain で assume_role の引数検証テストを1つ追加
4. 既存テスト全通過。外部仕様の変更なし

## 禁止事項
- StsOperations 以外の trait 追加（YAGNI）
- AppContext に Box<dyn ...> を入れない
- STS client を AppContext に保持しない
- 行数だけ減らすための無意味な分割
```

---

# Phase 0-2: CredentialStore trait

```text
## 目的
CacheManager を trait 越しに使えるようにし、ProfileResolver のテスト容易性を上げる。

## 方針
- CredentialStore trait: get / set / remove の3メソッドのみ。async 不要（現在同期）
- 既存 CacheManager の名前は変えても変えなくてもよい。pub API 影響が小さい方を選べ
- impl CredentialStore for CacheManager（または FileCredentialStore）
- ProfileResolver: &CacheManager → &dyn CredentialStore
- ★ AppContext は具象型のまま保持。Box<dyn> にしない

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. CredentialStore が object-safe
3. 既存 cache テスト全通過（期限切れ→None、破損JSON→graceful miss、衝突回避）
4. trait 経由の get/set/remove テストを1つ追加

## 禁止事項
- KeyringStore スケルトン（YAGNI）
- associated type / generic trait
- async 化
- list / clear 等の追加メソッド
```

---

# Phase 1-1: serde_yml → toml

```text
## 目的
semver 不安定な serde_yml 0.0.12 を削除し、toml に移行する。

## 現状の注意点（必ず事前確認）
- AwswitConfig は6フィールド + set_value/get_value/reset_value/save を持つ
- save() で serde_yml::to_string を使っている → toml::to_string_pretty に置換必要
- error.rs に YamlError(#[from] serde_yml::Error) がある → TomlError 相当に置換
- テストに serde_yml::from_str / serde_yml::to_string を使うものがある → 全置換
- serde(rename = "fuzzy-match") 等のハイフン付きキーは TOML でもそのまま動く

## 作業内容
1. Cargo.toml: serde_yml 削除、toml = "0.8" 追加
2. src/config/awswit_config.rs: パース/保存を toml に置換。パスを config.toml に変更
3. 移行パス: config.yaml のみ存在し config.toml が無い場合、yaml を読み stderr に1回警告
   "Warning: ~/.awswit/config.yaml is deprecated. Rename to config.toml."
4. 両方あれば toml 優先。どちらも無ければデフォルト
5. error.rs: YamlError → toml 由来のエラーに置換
6. テスト全修正。移行パステストを追加

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. serde_yml が Cargo.toml / Cargo.lock から完全消滅
3. config.toml 読み込み動作
4. yaml fallback + 警告動作
5. 既存テスト全通過

## 禁止事項
- 永続的な YAML/TOML 両対応分岐
- 自動変換ツール
```

---

# Phase 1-2: colored / indicatif / console 削除

```text
## 目的
TUI/出力系を ratatui + crossterm に寄せ、3クレートを削除する。dialoguer は残す。

## 作業内容
1. colored → crossterm::style::Stylize に置換（主に handle_list_profiles）
2. indicatif → crossterm ベースの最小スピナー実装
   - ★ 現スピナーは progress 表示 / success・error 表示 / Drop 時クリアを持つ。このライフサイクルを維持すること
3. console → crossterm に置換
4. Cargo.toml から3クレート削除

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. colored / indicatif / console が Cargo.toml から消えている
3. `awswit -l` でカラー出力が動作
4. STS 呼び出し中にスピナー表示。success/error で適切に終了表示
5. dialoguer が残っている

## 禁止事項
- crossterm の上に wrapper struct/trait を作らない
- async スピナーにしない
- スピナーの見た目テストは不要
```

---

# Phase 1-3: fs2 → fd-lock

```text
## ★ これは低リスクではない。guard の lifetime を誤るとロックが即解放される。

## 作業内容
1. Cargo.toml: fs2 → fd-lock = "4.0"
2. src/utils/fs.rs: lock_exclusive_with_timeout / lock_file_with_permissions を修正
   - fd-lock の RwLockWriteGuard を返す設計に寄せる
   - guard が Drop されるとロック解放。呼び出し元が guard を保持してロック維持する前提
3. autorefresh/daemon.rs, autorefresh/credentials_file.rs の呼び出し箇所修正
4. ロック順序（daemon lock → credentials lock）は絶対に変えない

## 検証
- ロック順序コメント（daemon.rs 冒頭）が実装と一致していることを目視確認
- guard のライフタイムがスコープ末尾まで維持されることを確認

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. fs2 が消えている
3. autorefresh テスト全通過
4. ロック順序コメントが正確

## 禁止事項
- 新抽象化レイヤー / async lock / mock lock
```

---

# Phase 1-4: uuid 削除 + clap_complete

```text
## 作業A: uuid 削除
- uuid の使用箇所を確認（セッション名生成のはず）
- PID + epoch_secs で置換。同一プロセスから同時に2回 AssumeRole を呼ぶことはない（sequential role chain）ため衝突しない。この理由をコメントに明記
- Cargo.toml から uuid 削除

## 作業B: clap_complete 導入
- Cargo.toml に clap_complete = "4.5" 追加
- Command enum に Completions { shell: clap_complete::Shell } 追加
- main.rs にハンドラ追加
- ★ 既存の autocomplete バイナリ (src/bin/autocomplete.rs) / --refresh-autocomplete とは役割が異なる。completions は static shell completion、既存は dynamic profile list。README に明記

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. uuid 消滅
3. `awswit completions bash/zsh/fish` が出力を生成
4. 既存 init / autocomplete と競合しない

## 禁止事項
- 動的補完の実装
- autocomplete バイナリの削除（別作業）
```

---

# Phase 2-1: nucleo-matcher 導入

```text
## 目的
TUI picker の fuzzy matching を strsim (Jaro-Winkler) から nucleo-matcher に置換。
CLI の typo suggestion (src/utils/fuzzy.rs) は strsim のまま残す。

## 作業内容
1. Cargo.toml に nucleo-matcher = "0.3" 追加（高レベル nucleo は不要）
2. src/tui/picker.rs のフィルタリングロジックを差し替え
3. ソート仕様（クエリあり時）:
   (1) fuzzy score 降順 → (2) favorite 優先 → (3) 履歴順 → (4) name 昇順
   ★ 同スコア時の順序を安定させること
4. 空クエリは全件表示（既存の favorite/履歴/名前順を維持）

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. "prod" → "production-admin" ヒット
3. "adm" → admin 系ヒット
4. 空クエリで全件表示
5. typo suggestion が継続動作
6. strsim が dependencies に残っている

## 禁止事項
- 高レベル nucleo crate（並列マッチング不要。プロファイルは50件程度）
- グローバル Matcher / ハイライト実装 / strsim 削除
```

---

# Phase 3-1: exec サブコマンド

```text
## ★ 最重要: exit code 伝播の設計
現在 run() -> Result<(), AwswitError> で、main 側が UserCancelled→130, 他→1 で exit。
このままでは子プロセスの exit 42 を伝播できない。

まず戻り値設計を整理すること。最小の変更候補:
- run() の成功時戻り値を i32 にする: Result<i32, AwswitError>
  - 通常コマンドは Ok(0)、exec は Ok(child_exit_code)
  - main: Ok(code) → exit(code), Err(UserCancelled) → exit(130), Err(_) → exit(1)

## 作業内容
1. src/cli/args.rs に Exec バリアント追加
   ```rust
   Exec {
       profile: String,
       #[arg(short = 'r', long = "refresh")]
       force_refresh: bool,
       #[arg(long = "region")]
       region: Option<String>,
       #[arg(last = true, required = true)]
       command: Vec<String>,
   }
   ```
2. exec ハンドラ実装（std::process::Command, stdin/stdout/stderr inherit）
3. 環境変数: AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_SESSION_TOKEN / AWS_DEFAULT_REGION / AWSWIT_PROFILE
4. 子プロセスの exit code をそのまま返す

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. `awswit exec <profile> -- echo hello` → "hello" + exit 0
3. `awswit exec <profile> -- false` → exit 1（伝播）
4. 既存コマンドの exit code 挙動が変わらない（0 / 1 / 130）
5. 親環境が汚染されない

## 禁止事項
- sh -c / eval の使用
- credential をコマンドライン引数に含めない
- 独自 cache / タイムアウト / 追加オプション
```

---

# Phase 3-2: Frecency（既存 history 活用）

```text
## ★ 既存 HistoryEntry は use_count / last_used / is_favorite を既に持つ。
## 新スキーマ導入も移行ロジックも不要。ソートロジックのみ追加する。

## 作業内容
1. HistoryEntry に frecency_score メソッド追加:
   ```rust
   pub fn frecency_score(&self, now: DateTime<Utc>) -> f64 {
       let hours = (now - self.last_used).num_seconds().max(0) as f64 / 3600.0;
       let weight = if hours < 1.0 { 4.0 }
           else if hours < 24.0 { 2.0 }
           else if hours < 168.0 { 1.0 }
           else { 0.5 };
       self.use_count as f64 * weight
   }
   ```
2. TUI picker の既定ソート（クエリなし時）を変更:
   (1) favorite 優先 → (2) frecency 降順 → (3) name 昇順
3. ★ 既存の record_use / is_favorite / set_favorite / .corrupt 退避挙動は一切変更しない

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. use_count 高いプロファイルが上位
3. 同 use_count なら last_used が新しい方が上位
4. favorite が非 favorite より常に上位
5. history.json 不在で正常動作
6. 破損時の .corrupt 退避が継続動作
7. frecency_score のユニットテスト4ケース以上（0h/1h/24h/168h/1000h 境界）

## 禁止事項
- history スキーマ変更 / 移行ロジック
- MAXAGE / weight のユーザー設定化
- history バックアップ/リストア機能
```

---

# Phase 3-3: --fzf フラグ

```text
## 目的
外部 fzf を使いたいユーザーにフォールバック手段を提供。デフォルトは既存 TUI。

## 作業内容
1. src/cli/args.rs に `#[arg(long = "fzf")] pub use_fzf: bool` 追加
2. src/tui/fzf.rs 新規作成
   - select_with_fzf(profiles, history) -> Result<String, AwswitError>
   - fzf 入力: frecency ソート済みプロファイル名（1行1名）
   - fzf オプション: --prompt "AWS Profile> " --height 40% --reverse --no-sort
   - fzf 不在時: "fzf not found in PATH. Install fzf or use the default picker."
   - exit 130 → UserCancelled

3. AWSWIT_USE_FZF の truthy パース:
   - 有効値: "1" / "true" / "yes" / "on"（大小文字無視）
   - それ以外は全て false（空文字、"0"、"false"、未定義 含む）
   - ★ 単純な .is_ok() 判定は禁止

4. AWSWIT_FZF_OPTS:
   - 空白区切りのトークン分割で fzf に追加引数として渡す
   - sh -c は禁止。シェル展開はしない

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. --fzf で fzf 起動。選択が機能
3. fzf 不在時に明確なエラー
4. Ctrl-C → exit 130
5. --fzf なしで既存 TUI 維持
6. AWSWIT_USE_FZF=true / 1 / yes / on で有効化
7. AWSWIT_USE_FZF=0 / false / 空文字 / 未定義 で無効
8. truthy パーサーのユニットテスト

## 禁止事項
- preview / skim 対応 / バージョンチェック / async IO / sh -c
```

---

# Phase 4-1: ProfileValidationError 統合

```text
## 目的
ProfileValidationError を廃止し AwswitError に統合。

## 作業内容
1. Profile::validate() の戻り値を Result<(), AwswitError> に変更
2. MissingSourceForRole / ConflictingCredentialSource / MissingAccessKeys → InvalidProfile
3. InvalidCredentialSource(String) → InvalidCredentialSource
4. エラーメッセージは現在の Display 文言をそのまま使用
5. ProfileValidationError enum を削除
6. grep で残骸確認

## 受け入れ基準
1. cargo build/test/clippy 全通過
2. ProfileValidationError 完全消滅
3. エラーメッセージ実質変更なし
4. 既存テスト全通過

## 禁止事項
- 新エラーバリアント追加
- エラーコード体系（E001〜E027）の破壊
```

---

# Phase 5-1: tokio features 最適化

```text
## 目的
tokio の features = ["full"] を実使用分のみに絞る。

## 作業内容
1. 全ソースから tokio 使用を調査（#[tokio::main], tokio::time, tokio::process 等）
2. 必要 feature のみ Cargo.toml に列挙
3. cargo build && cargo test && cargo test --all-targets で確認

## 受け入れ基準
1. build / test / all-targets 全通過
2. "full" 削除済み

## 禁止事項
- tokio バージョン変更
- rt-multi-thread を軽率に外すこと（AWS SDK が要求する可能性）
```

---

# Phase 6-1: 統合テスト + 最終検証

```text
## Part A: 統合テスト追加

1. tests/integration_basic.rs:
   - --version / --help / -l / completions bash / 存在しないプロファイル指定

2. tests/credential_store.rs:
   - set/get/remove ライフサイクル
   - 期限切れ → None
   - 破損 JSON → graceful miss

3. tests/frecency.rs:
   - frecency_score 境界値テスト
   - favorite 優先確認

4. exec テスト:
   - exit code 伝播
   - 環境変数注入

5. fzf ユニットテスト（外部バイナリ不要な部分のみ）:
   - truthy パーサー
   - オプション構築

## Part B: 最終チェックリスト（全件確認して結果報告）

1.  cargo test 全通過
2.  cargo clippy -- -D warnings 通過
3.  新規本番コードに unwrap/expect なし
4.  credential が debug/tracing ログに出ない
5.  新規ファイル書き込みが atomic_write_restricted を使用
6.  不要 clone 最小化
7.  lock order コメント（daemon.rs 冒頭）と実装の一致
8.  #[allow(dead_code)] 新規なし
9.  pub 範囲最小
10. 既存 CLI short/long オプション破壊なし
11. 新規 public 関数にテストあり
12. 1実装 trait は StsOperations / CredentialStore のみ
13. silent fallback なし
14. history .corrupt 退避挙動の維持
15. spinner の success/error 表示が退化していない

## Part C: CHANGELOG / README 更新
- YAML → TOML（移行手順記載）
- exec / completions / --fzf
- nucleo-matcher / frecency
- 依存整理（削除クレート一覧）

## 受け入れ基準
1. 全テスト通過
2. チェックリスト全項目 OK（根拠を簡潔に記載）
3. README / CHANGELOG 更新
4. 残課題があれば正直に列挙
```

---

## タスク一覧と想定規模

| # | Task | 依存 | 想定行数 | リスク |
|---|------|------|---------|--------|
| 0-1 | AppContext + StsOperations | なし | ~300 | 高 |
| 0-2 | CredentialStore trait | 0-1 | ~100 | 中 |
| 1-1 | YAML→TOML | 0-1 | ~100 | 低 |
| 1-2 | TUI依存統合 | 0-1 | ~200 | 中 |
| 1-3 | fs2→fd-lock | 0-1 | ~60 | 中 |
| 1-4 | uuid+completions | 0-1 | ~50 | 低 |
| 2-1 | nucleo-matcher | 0-1 | ~80 | 低 |
| 3-1 | exec | 0-1, 0-2 | ~150 | 中 |
| 3-2 | Frecency | 0-1 | ~60 | 低 |
| 3-3 | --fzf | 3-2 | ~100 | 低 |
| 4-1 | エラー統合 | 0-1 | ~60 | 低 |
| 5-1 | tokio features | 0-1 | ~5 | 低 |
| 6-1 | 統合テスト | 全完了 | ~400 | — |