# awswit

AWS プロファイルを素早く切り替える。あいまい検索、frecency、お気に入り。

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

### 使う

```bash
awswit
```

プロファイルを選んで Enter。`AWS_PROFILE` が今のシェルにセットされる。それだけ。

## 特徴

- **あいまい検索** — 数文字打てばすぐ絞れる
- **Frecency ソート** — よく使う・最近使ったやつが勝手に上に来る
- **お気に入り** — `*` でピン留め。常にトップに表示
- **プレビュー** — `Ctrl+P` で種別・リージョン・アカウント ID・ロール ARN を確認
- **fzf 連携** — `--fzf` で外部 fzf に切り替え。`AWSWIT_USE_FZF=1` で常時 fzf
- **認証はノータッチ** — `AWS_PROFILE` をセットするだけ。認証は SDK / SSO / aws-vault の仕事
- **シングルバイナリ** — Rust 製、依存なし

## 仕組み

`~/.aws/config` を読んでピッカーを出し、選ばれたプロファイルを環境変数にセットする:

```
AWS_PROFILE=prod
AWS_DEFAULT_PROFILE=prod
AWS_REGION=ap-northeast-1       # プロファイルにリージョン定義があれば
AWS_DEFAULT_REGION=ap-northeast-1
AWSWIT_PROFILE=prod
```

認証情報の解決は AWS SDK の仕事。IAM キーでも SSO でもロール引き受けでも `credential_process` でも、何でもいい。awswit は認証に関与しない。

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
| `-s, --show-commands` | export コマンドを表示するだけ（実行しない） |
| `-u, --unset` | AWS 環境変数を全部解除 |
| `-l, --list-profiles` | プロファイル一覧（`-l more` で詳細） |
| `-n, --no-interactive` | TUI なしで名前 or `$AWS_PROFILE` から解決 |
| `--fzf` | 外部 fzf を使う |
| `--region <region>` | リージョン上書き |
| `--config-file <path>` | AWS 設定ファイルのパス |
| `--info` | INFO ログ |
| `--debug` | DEBUG ログ |

## 設定

`~/.awswit/config.toml`（全部オプション）:

```toml
fuzzy-match = true           # あいまいマッチ（デフォルト: true）
colors = true                # カラー出力（デフォルト: Linux/macOS で true）
region = "ap-northeast-1"    # デフォルトリージョン上書き
```

知らないキーがあるとロード時にエラーになる。typo で設定が効かない、みたいなことは起きない。

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

それでいい。awswit はプロファイルが 10 個 20 個あって正確な名前を打つのが面倒な人向け。あいまい検索とお気に入りと frecency で、だいたい 1〜2 打鍵で目当てのプロファイルにたどり着ける。

**awsume / aws-vault と何が違う？**

awsume や aws-vault は認証を管理する。STS を叩いてトークンをキャッシュして MFA を処理する。awswit はそういうことを一切やらない。`AWS_PROFILE` をセットして、あとは SDK に丸投げ。だから:

- バックグラウンドプロセスなし
- トークンファイルのトラブルシュートなし
- 認証方式を問わない。awswit より後に出てきた方式でも動く

`aws sso login` や aws-vault を既に使っているなら、awswit は「どのプロファイルにする？」を速く選ぶためのツール。足りなかったパーツ。

## ライセンス

MIT — [LICENSE](LICENSE) を参照。
