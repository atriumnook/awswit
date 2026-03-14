# awswit

高速な AWS プロファイル切り替えツール。あいまい検索、frecency ソート、お気に入り機能付き。

[![CI](https://github.com/atnook/awswit/workflows/CI/badge.svg)](https://github.com/atnook/awswit/actions)
[![Crates.io](https://img.shields.io/crates/v/awswit.svg)](https://crates.io/crates/awswit)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[English](README.md)

<!-- TODO: docs/demo.gif TUI のスクリーンショット/録画を追加 -->

## クイックスタート

### インストール

```bash
cargo install awswit
```

### シェル設定

<details open>
<summary>Bash / Zsh</summary>

```bash
# ~/.bashrc or ~/.zshrc
eval "$(awswit init bash)"   # or zsh
```

</details>

<details>
<summary>Fish</summary>

```fish
# ~/.config/fish/config.fish
awswit init fish | source
```

</details>

<details>
<summary>PowerShell</summary>

```powershell
# $PROFILE
awswit init powershell | Invoke-Expression
```

</details>

### 実行

```bash
awswit
```

プロファイルを選んで Enter を押すだけ。現在のシェルに `AWS_PROFILE` がセットされます。

## 特徴

- **あいまい検索** — 数文字入力するだけでプロファイルを即座にフィルタ
- **Frecency ソート** — よく使う・最近使ったプロファイルが自動的に上位に浮上
- **お気に入り** — `*` キーでプロファイルをトップにピン留め
- **プレビューパネル** — `Ctrl+P` でプロファイル詳細（種別、リージョン、アカウント ID、ロール ARN）を表示
- **fzf 連携** — `--fzf` で外部 fzf を利用可能。`AWSWIT_USE_FZF=1` でデフォルト化
- **認証情報に触れない** — `AWS_PROFILE` をセットするだけ。認証は SDK / SSO / aws-vault に委任
- **シングルバイナリ** — Rust 製、ランタイム依存なし

## 仕組み

awswit は `~/.aws/config` を読み取り、ピッカーを表示し、現在のシェルに環境変数をセットします:

```
AWS_PROFILE=prod
AWS_DEFAULT_PROFILE=prod
AWS_REGION=ap-northeast-1       # プロファイルにリージョンが定義されている場合
AWS_DEFAULT_REGION=ap-northeast-1
AWSWIT_PROFILE=prod
```

AWS SDK がプロファイルの設定に応じて認証情報を解決します — IAM キー、SSO、ロール引き受け、`credential_process` など何でも対応。awswit は認証情報に一切触れません。

## キーバインド

| キー | 動作 |
|------|------|
| 文字入力 | あいまい検索 |
| `Enter` | プロファイルを選択 |
| `↑`/`↓` or `Ctrl+k`/`Ctrl+j` | カーソル移動 |
| `*` or `Ctrl+F` | お気に入りの切り替え |
| `Ctrl+P` | プレビューパネルの切り替え |
| `Esc` / `Ctrl+C` | キャンセル |

## CLI リファレンス

```
awswit [PROFILE]           プロファイルを切り替え（名前省略時は TUI 起動）
awswit init <shell>        シェル連携スクリプトを出力
awswit completions <shell> タブ補完スクリプトを生成
```

| フラグ | 説明 |
|--------|------|
| `-v, --version` | バージョンを表示 |
| `-s, --show-commands` | export コマンドを表示（セットせずに確認） |
| `-u, --unset` | AWS 環境変数をすべて解除 |
| `-l, --list-profiles` | プロファイル一覧を表示（`-l more` で詳細） |
| `-n, --no-interactive` | TUI をスキップし、名前または `$AWS_PROFILE` で解決 |
| `--fzf` | 外部 fzf を使用 |
| `--region <region>` | リージョンを上書き |
| `--config-file <path>` | AWS 設定ファイルのパス |
| `--info` | INFO レベルのログを表示 |
| `--debug` | DEBUG レベルのログを表示 |

## 設定

`~/.awswit/config.toml`（すべて任意）:

```toml
fuzzy-match = true           # あいまいマッチング（デフォルト: true）
colors = true                # カラー出力（デフォルト: Linux/macOS で true）
region = "ap-northeast-1"    # デフォルトリージョンの上書き
```

設定キーの typo はロード時にエラーになります — 誤設定がサイレントに無視されることはありません。

<details>
<summary>シェル補完</summary>

```bash
awswit completions bash > /etc/bash_completion.d/awswit
awswit completions zsh > ~/.zfunc/_awswit
awswit completions fish > ~/.config/fish/completions/awswit.fish
```

</details>

## FAQ

**`export AWS_PROFILE=foo` で十分では？**

もちろんそれでも動きます。awswit は 10 個以上のプロファイルを持っていて、正確な名前を打つのに疲れた人向けです。あいまい検索、お気に入り、frecency ソートにより、目的のプロファイルはたいてい 1〜2 回のキー入力で見つかります。

**awsume / aws-vault との違いは？**

awsume や aws-vault は認証情報を管理します — STS 呼び出し、トークンのキャッシュ、MFA の処理を行います。awswit はそのいずれも行いません。`AWS_PROFILE` をセットして、認証は SDK に任せるだけです。つまり:

- バックグラウンドプロセスなし
- デバッグすべきトークンファイルなし
- あらゆる認証方式に対応（awswit の作成後に登場した方式も含む）

`aws sso login` や aws-vault を既に使っているなら、awswit は「どのプロファイルをアクティブにするか」を高速・あいまい検索・frecency ソートで選べる、欠けていたピースです。

## ライセンス

MIT — [LICENSE](LICENSE) を参照。
