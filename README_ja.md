# awswit

AWS プロファイルをインタラクティブに切り替えるツール。あいまい検索と frecency で、使いたいプロファイルにすぐたどり着ける。

[![CI](https://github.com/atnook/awswit/workflows/CI/badge.svg)](https://github.com/atnook/awswit/actions)
[![Crates.io](https://img.shields.io/crates/v/awswit.svg)](https://crates.io/crates/awswit)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[English](README.md)

<!-- TODO: docs/demo.gif TUI の録画を追加 -->

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

### 使い方

```bash
awswit                  # インタラクティブに選択
awswit prod             # 直接切り替え
awswit -l               # プロファイル一覧
awswit -u               # 環境変数を解除
```

プロファイルを選んで Enter を押すと、`AWS_PROFILE` が現在のシェルに設定される。

## 特徴

- **あいまい検索** — 入力に応じてプロファイルを絞り込む
- **Frecency ソート** — 使用頻度と新しさに基づいて順位付け
- **お気に入り** — `*` でプロファイルを先頭に固定
- **プレビューパネル** — `Ctrl+P` でリージョン、アカウント ID、ロール ARN などを確認
- **fzf 連携** — `--fzf` または `AWSWIT_USE_FZF=1` で外部 fzf を使用
- **認証には関与しない** — awswit は `AWS_PROFILE` の設定だけを行い、認証は AWS SDK・SSO・aws-vault に委ねる
- **シングルバイナリ** — Rust 製、ランタイム依存なし

## 仕組み

`~/.aws/config` を読み、選択されたプロファイルを環境変数に設定する:

```
AWS_PROFILE=prod
AWS_DEFAULT_PROFILE=prod
AWS_REGION=ap-northeast-1       # プロファイルにリージョン定義があれば
AWS_DEFAULT_REGION=ap-northeast-1
AWSWIT_PROFILE=prod
```

認証の解決は AWS SDK が行う。IAM キー、SSO、ロール引き受け、`credential_process` など、方式を問わない。awswit は認証に関与しない。

## キーバインド

| キー | 動作 |
|------|------|
| 文字入力 | あいまい検索 |
| `Enter` | 選択 |
| `↑`/`↓` or `Ctrl+k`/`Ctrl+j` | 移動 |
| `*` or `Ctrl+F` | お気に入り切り替え |
| `Ctrl+P` | プレビュー切り替え |
| `Esc` / `Ctrl+C` | キャンセル |

## CLI リファレンス

```
awswit [PROFILE]           プロファイル切り替え（名前省略で TUI 起動）
awswit init <shell>        シェル連携スクリプトを出力
awswit completions <shell> タブ補完スクリプトを生成
```

| フラグ | 説明 |
|--------|------|
| `-v, --version` | バージョン表示 |
| `-s, --show-commands` | export コマンドを表示（実行はしない） |
| `-u, --unset` | AWS 環境変数をすべて解除 |
| `-l, --list-profiles` | プロファイル一覧（`-l more` で詳細表示） |
| `-n, --no-interactive` | TUI を使わず、名前または `$AWS_PROFILE` から解決 |
| `--fzf` | 外部 fzf を使用 |
| `--region <region>` | リージョンを上書き |
| `--config-file <path>` | AWS 設定ファイルのパス |
| `--info` | INFO レベルのログを表示 |
| `--debug` | DEBUG レベルのログを表示 |

## 設定

`~/.awswit/config.toml`（すべてオプション）:

```toml
fuzzy-match = true           # あいまいマッチ（デフォルト: true）
colors = true                # カラー出力（デフォルト: Linux/macOS で true）
region = "ap-northeast-1"    # デフォルトリージョンの上書き
```

不明なキーはロード時にエラーとなるため、typo が黙って無視されることはない。

<details>
<summary>シェル補完</summary>

```bash
awswit completions bash > /etc/bash_completion.d/awswit
awswit completions zsh > ~/.zfunc/_awswit
awswit completions fish > ~/.config/fish/completions/awswit.fish
```

</details>

## FAQ

**`export AWS_PROFILE=foo` でよくない？**

もちろん可能である。awswit はプロファイルが 10 個以上あり、正確な名前を入力するのが手間になった場合に役立つ。あいまい検索・お気に入り・frecency により、通常 1〜2 打鍵で目当てのプロファイルに到達できる。

**awsume / aws-vault と何が違う？**

awsume や aws-vault は認証を管理する。STS 呼び出し、トークンのキャッシュ、MFA の処理などを行う。awswit はそれらを一切行わない。`AWS_PROFILE` を設定し、認証は SDK に委ねる。そのため:

- バックグラウンドプロセスなし
- トークンファイルのトラブルシュートなし
- 認証方式を問わない。awswit の開発後に登場した方式でも動作する

`aws sso login` や aws-vault を既に使っているなら、awswit はプロファイルを素早く選択するためのツールである。

## ライセンス

MIT — [LICENSE](LICENSE) を参照。
