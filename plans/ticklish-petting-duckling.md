# awswit v2 リファクタリング実行計画

## Context

`plans/refactaring.md` に定義された13フェーズのリファクタリングを実行する。Phase 0-1 を最初に完了・コミットし、その後をバッチ並行実行する。

---

## Phase 0-1: AppContext + StsOperations trait

### Step 1: StsOperations trait 抽出

**1.1 Cargo.toml に async-trait 追加**
```toml
async-trait = "0.1"
```

**1.2 `src/aws/traits.rs` 新規作成**
- `#[async_trait] pub trait StsOperations` を定義
- メソッド: `assume_role` (8引数) + `get_session_token` (4引数)
- シグネチャは `src/aws/sts.rs:65-74` と `136-141` から完全コピー

**1.3 `src/aws/sts.rs` を修正**
- 既存の `pub async fn assume_role` (L65-133) と `pub async fn get_session_token` (L136-191) のメソッド本体を `#[async_trait] impl StsOperations for StsClient` ブロックに移動
- inherent impl からは削除（名前衝突を避けるため）
- `new()` と `client_with_credentials()` は inherent のまま残す

**1.4 `src/aws/mod.rs` を更新**
```rust
mod traits;
pub use traits::StsOperations;
```

**1.5 `src/profile/resolver.rs` を修正**
- import: `use crate::aws::{Credentials, StsClient}` → `use crate::aws::{Credentials, StsOperations}`
- 以下6箇所の `&StsClient` → `&dyn StsOperations`:
  - L40 `resolve_credentials`
  - L111 `assume_role_from_cli`
  - L161 `resolve_role_chain`
  - L408 `get_session_token_with_mfa`
  - L464 `get_session_token_credentials`
  - L489 `assume_role_with_mfa_large_duration`

**1.6 `src/main.rs`** — 変更不要。`&StsClient` → `&dyn StsOperations` への coercion は暗黙的。

**1.7 MockStsClient テスト追加** (`src/profile/resolver.rs` の tests モジュール内)
- `MockStsClient` struct: `RefCell<Vec<AssumeRoleCall>>` で呼び出し記録
- `#[async_trait] impl StsOperations for MockStsClient`
- `#[tokio::test] async fn mock_single_hop_role_chain`: base→dev の1ホップチェーンで assume_role の引数検証

### Step 2: AppContext 導入

**2.1 `src/context.rs` 新規作成**
```rust
pub struct AppContext {
    pub args: Args,
    pub config: AwswitConfig,
    pub profiles: HashMap<String, Profile>,
    pub history: ProfileHistory,
    pub cache: CacheManager,
}
```
- `AppContext::build(args: Args) -> Result<Self, AwswitError>`
- `main.rs` L70-140 のロード処理（config, AWS files, profiles, history, cache）を集約
- エラーハンドリングは既存パターンを保持（`dirs::home_dir` → `AwswitError::ShellError`）

**2.2 `src/lib.rs` に `pub mod context;` 追加**

**2.3 `src/main.rs` の `run()` を書き換え**
- `AppContext::build(args)` → 早期リターン（unset/kill/list/autocomplete）→ picker → STS lazy init → resolve → history save → autorefresh → emit
- MFA チェーンウォーク (L204-228) を `check_chain_requires_mfa()` ヘルパーに抽出
- 目標: run() を約80行以内に

**注意**: `AppContext::build` で全リソースを先にロードするため、`--unset` 等でも profiles/history/cache のロードが走る（数ms の副作用のみ、機能影響なし）

### チェックポイント
```bash
cargo fmt && cargo build && cargo test && cargo clippy -- -D warnings
```
→ 全通過後 `git commit`

---

## バッチ並行実行戦略

Phase 0-1 完了後、以下のバッチを順次実行。各バッチ内のフェーズは worktree isolate で並行実行し、マージ順序でコンフリクトを制御する。

### Batch 1: 1-1 + 1-4 + 4-1 + 5-1

| Phase | 変更ファイル | 概要 |
|-------|-------------|------|
| 5-1 | Cargo.toml | tokio `"full"` → 必要 features のみ |
| 1-4 | Cargo.toml, src/aws/sts.rs, src/cli/args.rs, src/main.rs | uuid 削除 + completions サブコマンド |
| 1-1 | Cargo.toml, src/config/awswit_config.rs, src/error.rs, tests/ | serde_yml → toml |
| 4-1 | src/error.rs, src/profile/types.rs | ProfileValidationError → AwswitError 統合 |

**マージ順序**: 5-1 → 1-4 → 1-1 → 4-1（4-1 は 1-1 の後。両方 error.rs を触るが別領域）

**コンフリクト箇所**:
- `Cargo.toml`: 各フェーズが異なる行を変更。5-1→1-4→1-1 の順で安全
- `src/error.rs`: 1-1 は L67-68 の YamlError→TomlError。4-1 は新バリアント追加 + ProfileValidationError 削除。別領域

### Batch 2: 0-2 + 2-1 + 3-2

| Phase | 変更ファイル | 概要 |
|-------|-------------|------|
| 0-2 | src/cache/ (trait追加), src/profile/resolver.rs | CredentialStore trait |
| 2-1 | Cargo.toml, src/tui/picker.rs | nucleo-matcher 導入 |
| 3-2 | src/history/storage.rs, src/tui/picker.rs | frecency_score + ソート変更 |

**マージ順序**: 0-2 → 2-1 → 3-2（3-2 は 2-1 の後。両方 picker.rs を触るが別関数）

**コンフリクト箇所**:
- `src/tui/picker.rs`: 2-1 は `update_filter()` のフィルタロジック。3-2 はコンストラクタのソートロジック。別関数
- `src/profile/resolver.rs`: 0-2 のみ（`&CacheManager` → `&dyn CredentialStore`）

### Batch 3: 1-2 + 1-3

| Phase | 変更ファイル | 概要 |
|-------|-------------|------|
| 1-2 | Cargo.toml, src/tui/spinner.rs, src/main.rs (handle_list_profiles) | colored/indicatif/console → crossterm |
| 1-3 | Cargo.toml, src/utils/fs.rs, autorefresh/*.rs | fs2 → fd-lock |

**コンフリクトなし**: 完全に独立したファイル群。どちらが先でもOK。

### Batch 4: 3-1 → 3-3 (逐次)

| Phase | 変更ファイル | 概要 |
|-------|-------------|------|
| 3-1 | src/cli/args.rs, src/main.rs | exec サブコマンド + run() 戻り値を Result<i32, _> に |
| 3-3 | src/cli/args.rs, src/tui/fzf.rs (新規) | --fzf フラグ (3-2 の frecency ソートに依存) |

**前提**: Batch 2 の 3-2 が完了していること（3-3 が frecency ソートを使う）
**3-1 は main.rs の run() 戻り値型を変更するため、Batch 1-3 の統合後に実行が安全**

---

## クロスバッチ統合順序

```
Phase 0-1 → commit
  ├── Batch 1 (5-1→1-4→1-1→4-1) → commit
  ├── Batch 2 (0-2→2-1→3-2) → commit
  ├── Batch 3 (1-2→1-3) → commit
  └── Batch 4 (3-1→3-3) → commit
```

各バッチ完了後に `cargo test && cargo clippy -- -D warnings` で検証。

---

## 対象ファイル一覧

| ファイル | 用途 |
|---------|------|
| `Cargo.toml` | 依存関係（ほぼ全フェーズ） |
| `src/aws/traits.rs` | **新規** StsOperations trait |
| `src/aws/sts.rs` | trait impl 移行 |
| `src/aws/mod.rs` | export 追加 |
| `src/context.rs` | **新規** AppContext |
| `src/lib.rs` | module 登録 |
| `src/main.rs` | run() 書き換え |
| `src/profile/resolver.rs` | &dyn StsOperations, &dyn CredentialStore |
| `src/error.rs` | TomlError, ProfileValidationError 統合 |
| `src/config/awswit_config.rs` | YAML→TOML |
| `src/cache/manager.rs` | CredentialStore trait |
| `src/tui/picker.rs` | nucleo + frecency |
| `src/tui/spinner.rs` | crossterm 移行 |
| `src/tui/fzf.rs` | **新規** fzf 統合 |
| `src/history/storage.rs` | frecency_score() |
| `src/utils/fs.rs` | fd-lock 移行 |
| `src/cli/args.rs` | Completions, Exec, --fzf |
| `src/profile/types.rs` | validate() 戻り値変更 |

## 検証

各フェーズ完了時:
```bash
cargo fmt && cargo build && cargo test && cargo clippy -- -D warnings
```

Phase 6-1 (最終) で統合テスト + チェックリスト15項目を実施。
