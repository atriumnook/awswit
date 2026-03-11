# awswit 技術仕様書

**プロジェクト名:** awswit  
**バージョン:** 0.1.0  
**作成日:** 2024年

---

## 1. システムアーキテクチャ

### 1.1 全体構成

```
┌─────────────────────────────────────────────────────────────────┐
│                         awswit CLI                               │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐            │
│  │   CLI   │  │   TUI   │  │ Config  │  │  Shell  │            │
│  │  Parser │  │ Picker  │  │ Loader  │  │ Export  │            │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘            │
│       │            │            │            │                   │
│  ┌────┴────────────┴────────────┴────────────┴────┐            │
│  │              Profile Resolver                   │            │
│  └────────────────────┬────────────────────────────┘            │
│                       │                                          │
│  ┌────────────────────┴────────────────────────────┐            │
│  │                 AWS STS Client                   │            │
│  └────────────────────┬────────────────────────────┘            │
│                       │                                          │
│  ┌──────────┐   ┌─────┴─────┐   ┌──────────┐                   │
│  │  Cache   │   │  History  │   │ Auto     │                   │
│  │ Manager  │   │  Storage  │   │ Refresh  │                   │
│  └──────────┘   └───────────┘   └──────────┘                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │    AWS STS      │
                    │    Service      │
                    └─────────────────┘
```

### 1.2 モジュール構成

```
src/
├── main.rs                 # エントリーポイント、オーケストレーション
├── error.rs                # エラー型定義
├── cli/
│   ├── mod.rs
│   └── args.rs             # CLIオプション定義 (clap)
├── config/
│   ├── mod.rs
│   ├── aws_files.rs        # ~/.aws/config, credentials パース
│   └── awswit_config.rs    # ~/.awswit/config.yaml
├── profile/
│   ├── mod.rs
│   ├── types.rs            # Profile 構造体
│   └── resolver.rs         # プロファイル解決ロジック
├── aws/
│   ├── mod.rs
│   ├── credentials.rs      # Credentials 構造体
│   └── sts.rs              # STS API クライアント
├── cache/
│   ├── mod.rs
│   └── manager.rs          # キャッシュ管理
├── shell/
│   ├── mod.rs
│   ├── export.rs           # 環境変数エクスポート
│   └── autocomplete.rs     # シェル補完スクリプト生成
├── tui/
│   ├── mod.rs
│   ├── picker.rs           # インタラクティブピッカー
│   ├── preview.rs          # プレビューパネル
│   ├── spinner.rs          # プログレス表示
│   └── theme.rs            # カラーテーマ
├── history/
│   ├── mod.rs
│   └── storage.rs          # 使用履歴管理
├── autorefresh/
│   ├── mod.rs
│   └── daemon.rs           # 自動リフレッシュデーモン
├── utils/
│   ├── mod.rs
│   └── fuzzy.rs            # ファジーマッチング
└── bin/
    ├── autoawswit.rs       # デーモンバイナリ
    └── autocomplete.rs     # 補完バイナリ
```

---

## 2. データ構造

### 2.1 Profile

```rust
pub struct Profile {
    // 識別子
    pub name: String,
    
    // 認証情報
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    
    // ロール設定
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
    pub credential_source: Option<String>,
    pub external_id: Option<String>,
    pub role_session_name: Option<String>,
    pub duration_seconds: Option<i32>,
    
    // MFA
    pub mfa_serial: Option<String>,
    
    // その他
    pub region: Option<String>,
    pub output: Option<String>,
    pub credential_process: Option<String>,
    
    // SSO
    pub sso_start_url: Option<String>,
    pub sso_region: Option<String>,
    pub sso_account_id: Option<String>,
    pub sso_role_name: Option<String>,
    
    // awswit 固有
    pub manager: Option<String>,
    pub autoawswit: Option<bool>,
}
```

### 2.2 Credentials

```rust
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub expiration: Option<DateTime<Utc>>,
    pub region: Option<String>,
}
```

### 2.3 HistoryEntry

```rust
pub struct HistoryEntry {
    pub name: String,
    pub last_used: DateTime<Utc>,
    pub use_count: u32,
    pub is_favorite: bool,
}
```

---

## 3. 処理フロー

### 3.1 プロファイル解決フロー

```
┌─────────────────┐
│  プロファイル名  │
└────────┬────────┘
         │
         ▼
┌─────────────────┐     ┌─────────────────┐
│ プロファイル検索 │────▶│ ファジーマッチ   │
└────────┬────────┘     └─────────────────┘
         │
         ▼
┌─────────────────┐
│  タイプ判定     │
│  (Role/User/SSO)│
└────────┬────────┘
         │
    ┌────┴────┬─────────┐
    ▼         ▼         ▼
┌───────┐ ┌───────┐ ┌───────┐
│ Role  │ │ User  │ │  SSO  │
│Profile│ │Profile│ │Profile│
└───┬───┘ └───┬───┘ └───┬───┘
    │         │         │
    ▼         ▼         ▼
┌─────────────────────────────┐
│     source_profile 解決     │
│     (再帰的にチェーン処理)   │
└──────────────┬──────────────┘
               │
               ▼
┌─────────────────────────────┐
│       キャッシュ確認         │
└──────────────┬──────────────┘
               │
       ┌───────┴───────┐
       ▼               ▼
┌────────────┐  ┌────────────┐
│ キャッシュ  │  │  STS API   │
│ ヒット     │  │  コール    │
└─────┬──────┘  └─────┬──────┘
      │               │
      └───────┬───────┘
              ▼
┌─────────────────────────────┐
│       Credentials 返却       │
└─────────────────────────────┘
```

### 3.2 インタラクティブピッカーフロー

```
┌─────────────────┐
│   起動          │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ プロファイル    │
│ 一覧取得        │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ 履歴読み込み    │
│ (ソート適用)    │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ TUI レンダリング │◀──────┐
└────────┬────────┘       │
         │                │
         ▼                │
┌─────────────────┐       │
│  イベント待機   │       │
└────────┬────────┘       │
         │                │
    ┌────┴────┬────┬──────┘
    ▼         ▼    ▼
┌───────┐ ┌─────┐ ┌─────┐
│ 文字  │ │ 移動│ │Enter│
│ 入力  │ │ ↑↓  │ │選択 │
└───┬───┘ └──┬──┘ └──┬──┘
    │        │       │
    ▼        │       ▼
┌───────┐    │  ┌─────────┐
│ファジー│    │  │プロファイル│
│フィルタ│    │  │ 返却    │
└───┬───┘    │  └─────────┘
    └────────┘
```

---

## 4. API 仕様

### 4.1 CLI インターフェース

```
awswit [OPTIONS] [PROFILE_NAME]

ARGUMENTS:
    [PROFILE_NAME]    使用するプロファイル名

OPTIONS:
    -r, --refresh             強制的にクレデンシャルを再取得
    -s, --show-commands       エクスポートコマンドを表示（実行しない）
    -u, --unset               AWS 環境変数をクリア
    -a, --auto-refresh        自動リフレッシュを有効化
    -k, --kill-refresher      自動リフレッシュプロセスを終了
    -l, --list-profiles [more] プロファイル一覧を表示
    -n, --no-interactive      インタラクティブモードを無効化
    
    --role-arn <ARN>          直接ロール ARN を指定
    --source-profile <NAME>   ソースプロファイルを指定
    --external-id <ID>        外部 ID を指定
    --mfa-token <TOKEN>       MFA トークンを指定
    --region <REGION>         リージョンを指定
    --session-name <NAME>     セッション名を指定
    --role-duration <SEC>     ロールセッション期間（秒）
    
    --config-file <PATH>      設定ファイルパスを指定
    --credentials-file <PATH> 認証情報ファイルパスを指定
    
    -v, --version             バージョン表示
    -h, --help                ヘルプ表示
    --debug                   デバッグログ有効化
    --info                    情報ログ有効化
```

### 4.2 環境変数出力

```bash
# 成功時の出力フォーマット
AWS_ACCESS_KEY_ID=AKIAXXXXXXXXXXXXXXXX
AWS_SECRET_ACCESS_KEY=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
AWS_SESSION_TOKEN=xxxxxxxx...
AWS_REGION=us-east-1
AWS_DEFAULT_REGION=us-east-1
AWSWIT_PROFILE=my-profile
AWSWIT_EXPIRATION=2024-01-01T12:00:00Z

# クリア時の出力
AWSWIT_UNSET=1
```

---

## 5. ファイル仕様

### 5.1 設定ファイル

**場所:** `~/.awswit/config.yaml`

```yaml
# 色付き出力を有効化
colors: true

# ファジーマッチングを有効化
fuzzy-match: true

# デフォルトのロールセッション期間（秒）
role-duration: 3600

# デフォルトリージョン
region: us-east-1

# デフォルトのセッション名
role-session-name: awswit-session

# デバッグ設定
debug:
  session_token_duration: 43200
```

### 5.2 履歴ファイル

**場所:** `~/.awswit/history.json`

```json
{
  "entries": {
    "prod-admin": {
      "name": "prod-admin",
      "last_used": "2024-01-15T10:30:00Z",
      "use_count": 42,
      "is_favorite": true
    },
    "dev": {
      "name": "dev",
      "last_used": "2024-01-15T09:00:00Z",
      "use_count": 128,
      "is_favorite": false
    }
  },
  "favorites": ["prod-admin"]
}
```

### 5.3 キャッシュファイル

**場所:** `~/.awswit/cache/<hash>.json`

```json
{
  "access_key_id": "ASIAXXXXXXXXXXX",
  "secret_access_key": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "session_token": "xxxxxxxx...",
  "expiration": "2024-01-15T22:00:00Z",
  "region": "us-east-1"
}
```

**パーミッション:** `0600` (所有者のみ読み書き可能)

---

## 6. エラーコード

| コード | 名前 | 説明 |
|--------|------|------|
| E001 | ProfileNotFound | 指定されたプロファイルが見つからない |
| E002 | InvalidProfile | プロファイル設定が無効 |
| E003 | SourceProfileNotFound | ソースプロファイルが見つからない |
| E004 | RoleChainCycle | ロールチェーンの循環を検出 |
| E005 | MissingProfileKey | プロファイルに必要なキーが不足 |
| E006 | InvalidCredentialSource | 無効なクレデンシャルソース |
| E007 | AssumeRoleFailed | AssumeRole API 呼び出しが失敗 |
| E008 | GetSessionTokenFailed | GetSessionToken API 呼び出しが失敗 |
| E009 | MfaTokenRequired | MFA トークンが必要 |
| E010 | InvalidMfaToken | MFA トークンが無効 |
| E011 | CacheError | キャッシュ操作エラー |
| E012 | ConfigFileError | 設定ファイルの読み込みエラー |
| E013 | ConfigKeyNotFound | 設定キーが見つからない |
| E014 | InvalidConfigCommand | 無効な設定コマンド |
| E015 | AwsSdkError | AWS STS SDK エラー |
| E016 | CredentialProcessFailed | credential_process の実行が失敗 |
| E017 | AutoRefreshError | 自動リフレッシュエラー |
| E018 | ShellError | シェル連携エラー |
| E019 | IoError | IO エラー |
| E020 | YamlError | YAML パースエラー |
| E021 | JsonError | JSON パースエラー |
| E022 | ValidationError | バリデーションエラー |
| E023 | AutoRefreshDurationLimit | 自動リフレッシュ時の期間制限超過 (> 1時間) |
| E024 | EnvError | 環境変数エラー |
| E025 | UserCancelled | ユーザーによる操作キャンセル |
| E026 | StsTimeout | STS リクエストタイムアウト |
| E027 | InvalidStsTimestamp | 無効な STS タイムスタンプ |
| E099 | Other | その他のエラー |

> **Note:** エラーコードの正式な定義は `src/error.rs` を参照してください。

---

## 7. 依存ライブラリ

### 7.1 主要依存

| クレート | バージョン | 用途 |
|----------|-----------|------|
| aws-config | 1.1.1 | AWS SDK 設定 |
| aws-sdk-sts | 1.9.0 | STS API クライアント |
| tokio | 1.35 | 非同期ランタイム |
| clap | 4.4 | CLI パーサー |
| ratatui | 0.25 | TUI フレームワーク |
| crossterm | 0.27 | ターミナル操作 |
| serde | 1.0 | シリアライズ |
| chrono | 0.4 | 日時処理 |

### 7.2 全依存一覧

```toml
[dependencies]
aws-config = { version = "=1.1.1", features = ["behavior-version-latest"] }
aws-sdk-sts = "=1.9.0"
aws-credential-types = "=1.1.1"
aws-types = "=1.1.1"
tokio = { version = "1.35", features = ["full"] }
clap = { version = "4.4", features = ["derive", "env", "string"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde_yaml = "0.9"
configparser = "3.0"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1.0"
anyhow = "1.0"
colored = "2.0"
dirs = "5.0"
dotenvy = "0.15"
sysinfo = "0.29"
strsim = "0.10"
dialoguer = "0.11"
ratatui = { version = "0.25", features = ["crossterm"] }
crossterm = "0.27"
indicatif = "0.17"
console = "0.15"
humantime = "2.1"
atty = "0.2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
fs2 = "0.4"
shellexpand = "3.1"
uuid = { version = "1.6", features = ["v4"] }

[target.'cfg(unix)'.dependencies]
libc = "0.2"
```

---

## 8. ビルド仕様

### 8.1 サポートターゲット

| ターゲット | OS | アーキテクチャ |
|-----------|-----|---------------|
| x86_64-unknown-linux-gnu | Linux | x86_64 |
| aarch64-unknown-linux-gnu | Linux | ARM64 |
| x86_64-apple-darwin | macOS | x86_64 |
| aarch64-apple-darwin | macOS | ARM64 (Apple Silicon) |
| x86_64-pc-windows-msvc | Windows | x86_64 |

### 8.2 リリースプロファイル

```toml
[profile.release]
lto = true           # Link-Time Optimization
codegen-units = 1    # 単一コード生成ユニット
strip = true         # シンボル除去
```

### 8.3 CI/CD パイプライン

```yaml
# GitHub Actions ワークフロー
- フォーマットチェック (cargo fmt)
- Lint (cargo clippy)
- テスト (cargo test)
- マルチプラットフォームビルド
- リリース自動作成 (タグプッシュ時)
```

---

## 9. セキュリティ考慮事項

### 9.1 クレデンシャル保護

- キャッシュファイルは `0600` パーミッションで作成
- メモリ上のクレデンシャルは使用後に明示的にクリア推奨
- 環境変数経由でのみクレデンシャルを公開

### 9.2 ファイルシステム

- `~/.awswit/` ディレクトリは `0700` パーミッション
- 一時ファイルは使用後に削除

### 9.3 ネットワーク

- AWS STS API への通信は TLS 1.2+ を使用
- プロキシ設定は環境変数経由で設定可能

---

## 10. テスト仕様

### 10.1 ユニットテスト

| モジュール | テスト内容 |
|-----------|-----------|
| config | INI パース、YAML パース |
| profile | プロファイル解決、タイプ判定 |
| cache | 読み書き、期限切れ判定 |
| history | 履歴記録、ソート |
| utils/fuzzy | ファジーマッチング |

### 10.2 統合テスト

| テスト | 内容 |
|--------|------|
| config_parsing | 設定ファイルのパース |
| profile_resolution | プロファイル解決チェーン |
| fuzzy_matching | ファジーマッチング精度 |

### 10.3 テストコマンド

```bash
# 全テスト実行
cargo test

# 特定テスト実行
cargo test test_fuzzy_matching

# 出力付きテスト
cargo test -- --nocapture
```
