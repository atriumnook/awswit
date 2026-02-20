# awswit

高速な AWS プロファイル切り替えツール。インタラクティブ TUI 搭載、Rust 製。

[English](README.md)

## 特徴

- **高速**: 起動時間 50ms 未満、Rust ネイティブのパフォーマンス
- **インタラクティブ**: fzf スタイルのファジープロファイルピッカー＋プレビューパネル
- **スマート**: 使用履歴の追跡、お気に入り機能、ファジーマッチング
- **安全**: クレデンシャルキャッシュに適切なファイルパーミッション (0600) を適用
- **多機能**: AssumeRole、MFA、ロールチェーン、credential_process 対応
- **シェル対応**: Bash、Zsh、Fish、PowerShell

## インストール

### ソースからビルド

```bash
cargo install --path .
```

### シェル設定

シェルの設定ファイルに以下を追加してください。eval 用のラッパー関数とタブ補完の両方がインストールされます：

**Bash** (`~/.bashrc`):
```bash
eval "$(command awswit --completion bash)"
```

**Zsh** (`~/.zshrc`):
```bash
eval "$(command awswit --completion zsh)"
```

**Fish** (`~/.config/fish/config.fish`):
```fish
command awswit --completion fish | source
```

**PowerShell** (`$PROFILE`):
```powershell
Invoke-Expression (& awswit --completion powershell)
```

## 使い方

```bash
# インタラクティブなプロファイルピッカーを起動
awswit

# 指定したプロファイルに切り替え
awswit my-profile

# クレデンシャルを強制的に再取得
awswit -r my-profile

# エクスポートコマンドを表示（実行しない）
awswit -s my-profile

# AWS 環境変数をクリア
awswit -u

# プロファイル一覧を表示
awswit -l

# プロファイル一覧を詳細表示
awswit -l more

# お気に入りの切り替え
awswit --favorite my-profile

# ロール ARN を直接指定して AssumeRole
awswit --role-arn arn:aws:iam::123456789012:role/MyRole --source-profile default

# credential_process として使用
awswit --credential-process my-profile

# クレデンシャルの自動リフレッシュを有効化
awswit -a my-profile

# 自動リフレッシュデーモンを停止
awswit -k
```

## 設定

### AWS 設定ファイル

awswit は標準的な AWS 設定ファイルを読み込みます：
- `~/.aws/config`
- `~/.aws/credentials`

### awswit 設定ファイル

`~/.awswit/config.yaml` にオプションの設定ファイルを配置できます：

```yaml
colors: true
fuzzy-match: true
role-duration: 3600
region: us-east-1
role-session-name: awswit-session
```

## キーボードショートカット（インタラクティブモード）

| キー | 操作 |
|------|------|
| `Enter` | プロファイルを選択 |
| `Esc` / `Ctrl+C` | キャンセル |
| `↑` / `↓` | 移動 |
| `Ctrl+K` / `Ctrl+J` | 移動（Vim スタイル） |
| `Ctrl+F` | お気に入りの切り替え |
| 文字入力 | プロファイルをフィルタ |

## ライセンス

Apache-2.0
