# awswit

awswit は fail-closed な高速 AWS profile selector である。Rust の単一 executable、built-in TUI、小さな生成 shell
hook で構成される。標準 AWS profile variables を切り替えるが、credential の保管、STS 呼び出し、SSO / AWS Login、
AWS CLI/SDK credential provider chain の代替は行わない。

[![CI](https://github.com/atriumnook/awswit/actions/workflows/ci.yml/badge.svg)](https://github.com/atriumnook/awswit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

[English](README.md) · [ドキュメント](docs/README.md) · [運用ランブック](docs/operations/runbook.md)

> awswit は現在 pre-1.0 である。executable と生成 shell hook は同じ version に揃えること。

## awswit を選ぶ理由

- built-in fuzzy TUI。fuzzy matching は visible candidates の filter にだけ使う。
- 非対話 activation と `exec` は case-sensitive exact match。typo から近い別 profile を選ばない。
- conflicting ambient AWS credential-provider variables を検出し、その値を表示せず fail-closed する。
- transactional allow-list shell protocol。profile 由来の text は data として代入し、shell code として評価しない。
- config-only、credentials-only、role、modern/legacy SSO、AWS Login（`login_session`）、`credential_process`、
  web identity、static-key profile の metadata を読み、access/secret/session-token values、login-session identity、
  process command text は保持しない。
- favorite と最近の利用による並び替え。履歴は concurrent-safe、bounded、best-effort。
- loaded hookはstatic CLI grammarとcurrently activatable profileのdynamic exact-name completionを合成する。
  `completions SHELL`はstatic grammarだけを生成する。
- prebuilt selector executable は Python、Node、AWS CLI、fzf、Nerd Font、daemon、language runtime を要求しない。

表示する account ID、role、provider label は local file 上の configured hint であり、検証済み AWS identity ではない。

## インストール

[GitHub Releases](https://github.com/atriumnook/awswit/releases) に公開済みの OS/architecture に合う archive があれば
SHA-256 と provenance を検証する。current release policy は binary-only で、このpackageをcrates.ioへ公開しない。
公開済みの `v0.0.2` はこの品質baselineより前の実装である。文書化したgateを証明する後続releaseまでは、
Rust 1.94以上で監査済みcheckoutからbuildする。

```bash
cargo install --locked --path .
```

checkout内のpackage metadataをregistryやrelease artifactの公開証拠として扱わない。

release target、checksum、SBOM、provenance の手順は
[install・検証ランブック](docs/operations/runbook.md#2-install) を参照する。release archiveはstartup fileを編集しない。
任意のcargo-dist installerはexecutable directoryを`PATH`へ追加し得るため、ランブックにPATH非変更/unmanaged installの
controlを記載する。どちらの経路も`awswit init`やcompletion lineを挿入しない。

## Shell hook を導入する

child executable は parent shell を変更できない。使用する shell の startup file から生成 hook を読み込む。

### Bash / Zsh

```bash
# ~/.bashrc
eval "$(awswit init bash)"

# ~/.zshrc
eval "$(awswit init zsh)"
```

### Fish

```fish
# ~/.config/fish/config.fish
awswit init fish | source
```

### PowerShell

```powershell
# $PROFILE
Invoke-Expression ((awswit init powershell) -join [Environment]::NewLine)
```

ここで読むのは installed executable が生成する static hook code である。activation response は検証対象の data frame
なので、`awswit activate` の出力を自分で eval してはならない。upgrade 後は shell を再起動または reload する。

## 使い方

```bash
awswit                                  # current shell で TUI 選択
awswit production                       # exact profile; hook の短縮形
awswit activate production              # explicit subcommand で同じ activation
awswit exec production -- aws sts get-caller-identity
awswit exec --profile=-h -- aws sts get-caller-identity  # 先頭が-のprofile
awswit list --format names
awswit doctor                            # offline local diagnostics
awswit unset                             # hook経由でawswitのprofile + region variablesを解除
```

binary 自身が受理するのは `awswit activate PROFILE` で、`awswit PROFILE` は hook の Interface である。automation では
`exec` または explicit subcommand を使う。profile 名が subcommand と同じ場合（例: `list`）は
`awswit activate list` と指定する。Bash/Zsh/Fishでprofile名が`-`から始まる場合（help/version flagそのものを含む）は、
例えば`awswit -- -h`または`awswit activate -- -h`と指定する。PowerShellはunquoted `--`を自身の
end-of-parameters tokenとして消費するため、`awswit activate --profile=-h`を使うか、literal separatorを
`awswit activate '--' -h`のようにquoteする。
`exec`では`awswit exec --profile=-h -- COMMAND...`という曖昧さのないnamed profile formを使う。PowerShell hookは
通常のunquoted separatorをcommand境界が一意なときだけ復元する。child executable自体が`-`から始まる場合は
`awswit exec PROFILE '--' -command`とquoteし、曖昧なままならhookは何も実行せず拒否する。

### Credential override protection

AWS credential environment variables は選択 profile より優先され得る。awswit は停止して conflict variable names を
示すが、値は表示しない。scope を確認したうえで明示的に解除する。

```bash
awswit production --clear-credential-overrides

# この command だけ変更するなら、こちらが安全:
awswit exec production --clear-credential-overrides -- aws sts get-caller-identity
```

前者は current shell、後者は command process だけから検出済み conflict を除く。`awswit unset` は credential や
`AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE` を解除しない。
IMDS endpoint overrideを許可するのはcomplete chainが`credential_source = Ec2InstanceMetadata`を明示する場合だけである。
alternate `AWS_LOGIN_CACHE_DIRECTORY`はexplicit AWS Login chainだけで許可する。`AWS_BEARER_TOKEN_BEDROCK`はBedrock
requestで選択profileのidentityを置換し得るため常にconflictとする。
一部AWS SDKが読むlegacy direct-key aliases（`AWS_ACCESS_KEY`、`AWS_SECRET_KEY`、`AMAZON_*` key/token names）も
standard namesと同じ保護対象である。`credential_source = Environment`を含め、presentなdirect-key variableはすべて
conflictにする。AWS CLI/SDKのenvironment-provider precedenceがそのcredentialを直接使い、選択roleを迂回し得るためである。
`--clear-credential-overrides`は選んだscopeから検出済みnameをすべて除けるが、awswitはvalueを読まず、aliasの優先順位も
暗黙決定しない。clearすれば必要なsourceも消えるため、`credential_source = Environment`を使うroleはawswitのgeneric
`AWS_PROFILE` contractでは意図的にactivatableとしない。consumerがselected profile経由で解決できるfile/process/SSO/
workload sourceを使う。`role_arn`のないstandalone `credential_source`も、現行SDKがrejectまたはignoreして別identityへ
fall throughし得るためdiagnostic-onlyとする。
Java SDK v1固有の`AWS_CREDENTIAL_PROFILES_FILE` path overrideは常にconflictとする。clearすれば同SDKのdefault pathへ戻るが、
Java v1はstandardなnon-default credentials-path variableを尊重しないため、non-default file利用時はEOL consumer側を
明示設定する。
AWS SDK for Go v1のdefault sessionはshared config内のrole/SSO/regionを読むために`AWS_SDK_LOAD_CONFIG=1`（または
`SharedConfigEnable`）を必要とする。awswitはこのapplication behavior flagをoverwrite/所有しない。EOL SDKをv2へ移行するか、
legacy consumerを明示設定してidentityを検証する。

AWS Tools for PowerShellのvisibleかつnon-nullな`$StoredAWSCredentials` session credentialは`AWS_PROFILE`より優先する。
PowerShell hookはこの状態でcurrent-shell activationを拒否するため、sessionを確認して`Clear-AWSCredential`を実行後に
再試行する。Windowsでは.NET SDK Storeの同名profileもshared-file profileより優先し得る。.NET consumerは
`AWSConfigs.AWSProfilesLocation`を意図したshared credentials fileへ設定する（またはapplication startupでlegacy
persistence storeを無効化する）うえ、`GetCallerIdentity`でprincipalを検証する。詳細は
[Windows consumer runbook](docs/operations/runbook.md#54-windows-powershell-and-net-consumer-precedence)を参照する。

## TUI キー

| キー | 動作 |
|---|---|
| 文字入力 | existing profile names を fuzzy filter |
| `Enter` | focused exact profile を選択 |
| `↑` / `↓`, `Ctrl+K` / `Ctrl+J` | 移動 |
| `PageUp` / `PageDown`, `Home` / `End` | 大きく移動 |
| `*` または `Ctrl+F` | favorite 切替 |
| `Ctrl+P` | configured-metadata preview 切替 |
| `Ctrl+U` | query clear |
| `Esc` / `Ctrl+C` | cancel / interrupt |

current profile がない場合、起動直後は row が armed されず bare Enter で先頭を誤選択しない。0 match は確定できない。
`NO_COLOR`、狭い terminal、Unicode name、通常 font に対応する。

## CLI

```text
awswit
awswit activate [PROFILE] [OPTIONS]
awswit exec PROFILE [OPTIONS] -- COMMAND [ARG...]
awswit list [--format human|names|json] [SOURCE OPTIONS]
awswit doctor [--format human|json] [SOURCE OPTIONS]
awswit unset
awswit init bash|zsh|fish|powershell
awswit completions bash|zsh|fish|powershell
```

主な option:

| Option | 意味 |
|---|---|
| `--region REGION` | profile の configured region を override |
| `--clear-credential-overrides` | activation/command scope で detected conflicts を明示解除 |
| `--config-file PATH` | この selection の `AWS_CONFIG_FILE` を override |
| `--credentials-file PATH` | この selection の `AWS_SHARED_CREDENTIALS_FILE` を override |

source precedence は CLI option、AWS path environment variable、`~/.aws/config` / `~/.aws/credentials` の順。missing
default file は empty、missing explicit path は error。relativeなnon-default pathはinvocation時のlexical absolute pathへ
固定するため、後続のdirectory変更で別fileを暗黙選択しない。command 別詳細は `awswit COMMAND --help` で確認する。

loaded `init SHELL` artifactはfull command/option completionとcurrently activatable profileの安全なruntime feedを合成する。
display/insertion valueを分離できないshell menuではzero-cell Unicode scalarを含むnameを候補から除くが、visible escape付き
TUI、exact activation、raw `list --format names`からは利用できる。standalone `completions SHELL` artifactはruntime profile
discoveryを持たない同じstatic grammarである。

## 設定される値

activation 成功時は次を coherent に保つ。

```text
AWS_PROFILE=production
AWS_DEFAULT_PROFILE=production
AWSWIT_PROFILE=production
AWS_REGION=ap-northeast-1
AWS_DEFAULT_REGION=ap-northeast-1
```

`--region` も profile region もなければ region pair を解除し、以前の awswit region の持越しを防ぐ。command-line
source pathとrelativeなenvironment-selected source pathはlexical absolute pathとして対応する標準AWS path variablesで
伝播する。
hookはcapture対象のactivation/unsetとdoctor subprocessだけに`AWSWIT_HOOK=1`と`AWSWIT_SHELL`をscopeする。
unrelated child shellへintegration markerをexportしない。

## Security / responsibility boundary

awswit は次を行わない。

- access key/token の保管、表示、refresh、rotation;
- STS 呼び出し、MFA prompt、IAM Identity Center / AWS Login、`~/.aws/sso/cache` / `~/.aws/login/cache` 管理;
- discovery/`doctor` での `credential_process` 実行;
- current AWS principal、permission、login、token expiry、network の検証;
- shell startup file へのhook/completion lineの自動挿入、daemon、telemetry。

credential lifecycle は `aws sso login`、`aws login --profile PROFILE`、通常の SDK/CLI flow、aws-vault などに委譲する。
offline catalog issue は `awswit doctor`（automationでは`--format json`）で確認し、authentication failure は provider
所有者の runbookへ引き継ぐ。

## ドキュメント

- [製品要件](docs/requirements/product-requirements.md)
- [競合・AWS標準調査](docs/requirements/competitive-research.md)
- [アーキテクチャ](docs/design/architecture.md)
- [詳細仕様](docs/design/specification.md)
- [install、upgrade、障害対応、rollback、incident runbook](docs/operations/runbook.md)

## ライセンス

MIT — [LICENSE](LICENSE) を参照。
