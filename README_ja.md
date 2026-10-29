# awswit

AWS プロファイルをインタラクティブに切り替えるツール。あいまい検索、frecency ソート、スクリプトからの利用に対応し、数十個のプロファイルから目当てのものをすぐ選べる。

[![CI](https://github.com/atriumnook/awswit/workflows/CI/badge.svg)](https://github.com/atriumnook/awswit/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[English](README.md)

## クイックスタート

### インストール

ビルド済みバイナリは各 [GitHub Release](https://github.com/atriumnook/awswit/releases) に添付している。
crates.io には公開して**いない**ため、`cargo install awswit` は 404 になる。

`releases/latest` が指すのは最新の *GitHub Release* であり、git タグではない。
`v0.1.0` タグはソースのみで、Release は作られていない(release-plz が
`GITHUB_TOKEN` でタグを push したため、cargo-dist が起動しなかった)。
次にタグ付きリリースを行うまで、`latest` は v0.0.2 を指したままの場合がある。

```bash
# Linux / macOS — GitHub Release のインストーラ(その時点の latest)
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/atriumnook/awswit/releases/latest/download/awswit-installer.sh | sh

# Windows (PowerShell)
irm https://github.com/atriumnook/awswit/releases/latest/download/awswit-installer.ps1 | iex

# ソースから(このリポジトリ。crates.io ではない)
cargo install --git https://github.com/atriumnook/awswit --locked
```

### シェル設定

使っているシェルの rc ファイルに次を追加する。バイナリを包む `awswit`
シェル関数が定義され、選択したプロファイルが現在のシェルの `AWS_PROFILE`
に反映される。

```bash
# ~/.bashrc
eval "$(awswit init bash)"

# ~/.zshrc
eval "$(awswit init zsh)"

# ~/.config/fish/config.fish
awswit init fish | source

# $PROFILE (PowerShell)
awswit init powershell | Invoke-Expression
```

### 使い方の概要

```text
awswit                       # 対話ピッカー — シェルの AWS_PROFILE を設定
awswit prod                  # 直接切り替え(存在しない名前なら、その名前で絞り込んだピッカーを開く)
awswit pick                  # 対話ピッカー — 選んだ名前を stdout に出力
awswit which                 # 現在有効なプロファイルを表示
awswit doctor                # ~/.aws/config と SSO トークンを点検
awswit exec prod -- aws s3 ls   # シェルを変えずに 1 コマンドだけ実行
awswit -l                    # プロファイル一覧(パイプ先なら TSV、端末なら表)
awswit -l --json             # プロファイル一覧を JSON で出力
awswit prompt                # 現在のプロファイル(シェルのプロンプト埋め込み用)
awswit -u                    # AWS_PROFILE・AWS_REGION・継承したクレデンシャルを解除
```

TUI の操作: 文字入力であいまい検索、`↑/↓`(または `Ctrl-k`/`Ctrl-j`)で
移動、`Enter` で選択、`*` か `Ctrl-F` でお気に入りの切り替え、`Ctrl-P`
でプレビューパネルの切り替え、`Ctrl-A`/`Ctrl-E` で検索欄の先頭/末尾へ
移動、`Ctrl-W` で直前の単語を削除、`Esc` でキャンセル。

## awswit が行うこと

プロファイルを選ぶと、awswit は `export` と `unset` の文を出力し、シェル
関数がそれを評価する。リージョンが `ap-northeast-1` のプロファイルで
`awswit prod` を実行した場合:

```sh
export AWS_PROFILE='prod'
unset AWS_DEFAULT_PROFILE
export AWS_REGION='ap-northeast-1'
unset AWS_DEFAULT_REGION
unset AWS_ACCESS_KEY_ID
unset AWS_SECRET_ACCESS_KEY
unset AWS_SESSION_TOKEN
unset AWS_SECURITY_TOKEN
unset AWS_CREDENTIAL_EXPIRATION
```

現在推奨の `AWS_PROFILE` / `AWS_REGION` を**設定**し、レガシーな
`AWS_DEFAULT_*` を明示的に**解除**する。CI イメージや社内の dotfile、
以前の `aws configure` が残した値によって、認証情報が黙って別アカウント
に向くことを防ぐためである。プロファイルにリージョンがなければ
`AWS_REGION` も解除し、SDK 自身の解決に任せる。

**継承したクレデンシャルの環境変数**(`AWS_ACCESS_KEY_ID`、
`AWS_SECRET_ACCESS_KEY`、`AWS_SESSION_TOKEN` など)も解除する。SDK の解決
順ではこれらが `AWS_PROFILE` より優先されるため、以前の `aws sso login`、
aws-vault のサブシェル、awsume が残した鍵やセッショントークンがあると、
`awswit prod` でプロファイルを設定しても `aws s3 ls` は古いアカウントに
アクセスし続ける。これらを解除するので、**`awswit prod` の後は、ただの
`aws ...` が prod プロファイルとして動く。** 同じ解除は `awswit -u` と
`awswit exec` の中でも行う。

対話ピッカーは stdout ではなく **stdin と stderr** が端末かどうかで起動を
判断する。ドキュメントどおりのラッパー(`eval "$(awswit init bash)"`)は
stdout を `$()` で取り込み、`$(awswit pick)` も同様である。TUI は stderr
に描画するので、取り込まれる出力は汚れない。`--fzf` も stderr を引き継ぐ
ため、ラッパー経由でも fzf の画面が端末に表示される。

存在しない名前を指定した場合、awswit は近い名前のプロファイルに勝手に
置き換えない。プロファイルはアカウントに対応するので、推測で選ぶと別の
アカウントに切り替わりうるためである。端末があればその名前で絞り込んだ
ピッカーを開いて Enter での確定を求め、端末がなければ候補を示して
エラーにする。

その先の認証情報の解決は AWS SDK が行う。IAM キー、SSO、ロールの引き受け、
`credential_process` など、設定済みの方式なら何でもよい。
**awswit 自身は認証情報を読まず、取得も保存もしない**。競合する変数を
解除し、残りは SDK に任せる。

## 比較

| | **awswit** | **aws-vault** | **awsume** | `export AWS_PROFILE` + `fzf` |
|---|---|---|---|---|
| プロファイルを対話的に選ぶ | ✅ TUI + fzf | —(`list` は表示のみ) | ✅ TUI | ✅ |
| あいまい検索 | ✅(内蔵) | — | ✅(内蔵) | ✅ |
| frecency ソート + お気に入り | ✅ | — | — | — |
| 打ち間違い時の候補提示 | ✅「did you mean…?」 | — | — | — |
| TUI プレビュー(リージョン / アカウント / ロール ARN / MFA / 最終使用) | ✅ | — | — | — |
| シェルを変えずに 1 回だけ `exec` | ✅ `awswit exec` | ✅ `aws-vault exec` | ✅ | — |
| `which`(現在のプロファイル + SSO トークンの状態) | ✅ | 一部 | 一部 | — |
| `doctor`(壊れた source チェーン、期限切れ SSO トークン、MFA 未設定) | ✅ | — | — | — |
| 機械可読な一覧出力(TSV / JSON / 名前のみ) | ✅ | 一部 | — | 該当なし |
| シェルのプロンプト連携 | ✅ `awswit prompt` | — | — | — |
| 認証情報 / STS / MFA の管理 | — 意図的に行わない | ✅ | ✅ | — |
| トークンのキャッシュ・自動更新 | — | ✅ | ✅ | — |
| 単一の静的バイナリ | ✅ Rust | ✅ Go | — Python | 該当なし |

awswit は**プロファイルの切り替え**に特化しており、プロファイルの選択・
確認・スクリプトからの利用を担う。STS / MFA / トークンのキャッシュは
行わない。それは AWS SDK、aws-vault、aws-sso-login の役割である。
組み合わせて使う:

```bash
# 認証(1 日 1 回): STS は aws-vault か aws sso login が担う
aws sso login --profile main

# 切り替え(1 日に何十回も): ピッカーは awswit が担う
awswit prod
aws s3 ls

# 別プロファイルで 1 回だけ、シェルを変えずに実行
awswit exec staging -- aws s3 ls s3://bucket
```

## CLI リファレンス

### サブコマンド

```text
awswit [PROFILE]                 プロファイルを選ぶ(PROFILE 省略かつ stdin+stderr が端末なら TUI)
awswit pick                      ピッカーを開き、選んだ名前を stdout に出力(シェルは変えない)
awswit exec PROFILE -- CMD ...   AWS_PROFILE を設定して CMD を実行(シェルは変えない)
awswit which                     現在のプロファイルと SSO の期限 / aws-vault の状態を表示
awswit doctor                    ~/.aws/config と SSO キャッシュを点検。エラーがあれば非ゼロで終了
awswit prompt [--format F]       シェルのプロンプト埋め込み用に現在のプロファイルを出力
awswit init <shell>              シェル連携のスクリプトを出力
awswit completions <shell>       タブ補完スクリプトを出力
```

### フラグ

| フラグ                    | 説明                                                                 |
| ------------------------ | -------------------------------------------------------------------- |
| `-v`, `--version`        | バージョンを表示                                                        |
| `-s`, `--shell-export`   | `set`/`unset` の行だけを出力する(`eval` 用)。シェルラッパーが使う          |
| `-u`, `--unset`          | awswit が管理する変数をすべて解除する。継承したクレデンシャルの環境変数も含む |
| `-l`, `--list`           | プロファイル一覧(パイプ先なら TSV、端末なら表)                            |
| `--json`                 | `-l` と併用し、JSON で出力                                              |
| `--names-only`           | `-l` と併用し、1 行 1 名で出力(補完用の高速経路)                          |
| `-n`, `--no-interactive` | ピッカーを使わず、PROFILE か `$AWS_PROFILE` から解決                       |
| `--fzf`                  | 内蔵 TUI の代わりに外部の `fzf` を使う                                   |
| `--region <REGION>`      | リージョンを上書き                                                      |
| `--config-file <PATH>`   | AWS 設定ファイルのパス                                                  |
| `--verbose`              | INFO レベルのログ                                                      |
| `--debug`                | DEBUG レベルのログ                                                     |

## 環境変数

| 変数                           | 効果                                                                  |
| ----------------------------- | --------------------------------------------------------------------- |
| `AWS_CONFIG_FILE`             | `~/.aws/config` の代わりに使うファイル                                    |
| `AWS_SHARED_CREDENTIALS_FILE` | `~/.aws/credentials` の代わりに使うファイル                               |
| `AWS_PROFILE`                 | PROFILE を省略した `-n` の既定値                                         |
| `AWS_VAULT`                   | 設定されていると、aws-vault のセッション内であることを警告する                 |
| `NO_COLOR`                    | 色を無効にする(事実上の標準)                                             |
| `AWSWIT_NO_FUZZY`             | 存在しない PROFILE で、絞り込んだピッカーを開かずにエラーにする                 |
| `AWSWIT_USE_FZF`              | 外部の `fzf` を使う(`--fzf` と同じ)                                     |
| `AWSWIT_FZF_OPTS`             | `fzf` への追加オプション(危険なオプションは取り除く)                        |
| `AWSWIT_SHELL`                | シェルの種類を指定(`bash` / `zsh` / `fish` / `powershell`)              |
| `AWSWIT_SSO_CACHE_DIR`        | AWS SSO トークンのキャッシュ先(既定は `~/.aws/sso/cache`)                  |
| `XDG_DATA_HOME`               | 履歴の保存先(既定は `~/.local/share/awswit/`)                           |

awswit は独自の設定ファイルを意図的に**読まない**。実行時の設定はすべて
CLI フラグか環境変数で行う。

## シェルのプロンプト連携

`awswit prompt` はシェルのプロンプトへの埋め込み用で、終了コードは 0、
末尾に改行を付けず、出力の書式を指定できる。

```bash
# bash — プロファイルをシアンの角括弧で表示する PS1 の例
PS1='\[\033[36m\]$(awswit prompt --format "[%s] " --default "")\[\033[0m\]\u@\h:\w\$ '

# zsh — precmd や右プロンプトに入れる
RPROMPT='%F{cyan}$(awswit prompt --format "(%s)" --default "")%f'

# starship.toml — カスタムコマンドのセグメント
[custom.awswit]
command = "awswit prompt --format '☁ {}'"
when = '[ -n "$AWS_PROFILE" ]'
```

(`--format` では `%s` と `{}` のどちらも使える。)

## 点検: `awswit doctor`

`awswit doctor` は `~/.aws/config` と AWS CLI の SSO トークンキャッシュを
調べ、実際によく起きる問題を報告する:

```text
ERROR [foo-role] source_profile = 'gone' references a profile that does not exist
ERROR [sso-prod] SSO token expired at 2026-05-12 03:14 UTC — run `aws sso login --profile sso-prod`
warn  [no-mfa] role profile has neither source_profile nor credential_source
warn  [legacy] mfa_serial = 'foo' does not look like an IAM MFA ARN

awswit doctor: 2 error(s), 2 warning(s)
```

終了コード: 問題がなければ 0、エラーがあれば 1。

## ファイル

- `$XDG_DATA_HOME/awswit/history.json` — 使用履歴とお気に入り。
  `< 0.1.0` から更新した後の初回実行で、旧来の `~/.awswit/history.json`
  から自動で移行する。秘密情報ではなく、umask に従って書き込む。

## スクリプトでの利用例

```bash
# プロファイル名を 1 行 1 つずつ出力
awswit -l | cut -f1

# 名前に "staging" を含む最初のプロファイルに切り替え
awswit -n "$(awswit -l | awk -F'\t' '/staging/ {print $1; exit}')"

# プロファイルのメタデータを JSON で確認
awswit -l --json | jq '.[] | select(.type == "Role")'

# 一時的なプロファイルでシェルを起動(その場限りの作業向け)
awswit exec prod -- bash

# pick を他のツールと組み合わせる(シェルは変えない)
# `pick` はキャンセル時に 130 で終了するので、空文字で実行しないよう || で止める
P=$(awswit pick) || exit 130
aws --profile "$P" sts get-caller-identity

# CI に組み込む: 期限切れの SSO トークンがあるプロファイルがあればジョブを失敗させる
awswit doctor
```

## ライセンス

MIT — [LICENSE](LICENSE) を参照。
