# awswit operations runbook

対象: awswit pre-1.0
更新日: 2026-08-25

## 1. Operating model

awswit は local-only profile selector である。通常動作と `doctor` は AWS API、STS、SSO endpoint、instance
metadata、telemetry endpointへ接続しない。AWS authentication は、activation 後の AWS CLI / SDK または
`exec` した command が行う。明示的に `exec` した command の network access は awswit の offline guarantee 外である。

運用上は次の3 artifact を区別する。

1. platform-specific `awswit` executable;
2. executable が `init SHELL` で生成する current-shell hook;
3. non-authoritative preference history (`history.json`)。

AWS `config` / `credentials`、SSO cache、AWS Login cache は awswit の管理対象ではない。backup、権限、rotation は
AWS CLI / SDK側の運用に従う。

## 2. Install

### 2.1 Published prebuilt release（利用可能な場合は推奨）

2026-08-25 の外部状態確認では、latest public `v0.0.2` は本baselineより前の実装であり、
`v0.0.1` の古いdraftも残っている。権限を持つmaintainerがこのdraftを確認・整理し、後続releaseで
本章と8章のgateを実証するまでは、2.2の監査済みsource buildを使う。

1. [GitHub Releases](https://github.com/atriumnook/awswit/releases) から OS/architecture に一致する archive と
   同名の `.sha256` を取得する。対象は Linux x86-64/ARM64、macOS Intel/Apple Silicon、Windows x86-64。
2. 公開 release に target artifact、checksum、CycloneDX SBOM、GitHub provenance が揃っていることを確認する。
   source に workflow があるだけでは公開済み artifact の存在証明にならない。
3. archive hash を比較してから展開する。Unix の例:

   ```bash
   archive=awswit-x86_64-unknown-linux-gnu.tar.xz
   expected=$(awk 'NF {print $1; exit}' "$archive.sha256")
   actual=$(sha256sum "$archive" | awk '{print $1}')
   test "$actual" = "$expected"
   ```

   macOS で `sha256sum` がなければ `shasum -a 256` を使う。PowerShell の例:

   ```powershell
   $Archive = 'awswit-x86_64-pc-windows-msvc.zip'
   $Expected = (Get-Content "$Archive.sha256" | Select-Object -First 1).Split()[0]
   $Actual = (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
   if ($Actual -ne $Expected.ToLowerInvariant()) { throw 'awswit checksum mismatch' }
   ```

4. GitHub CLI を利用できる環境では provenance も検証する。

   ```bash
   gh attestation verify "$archive" --repo atriumnook/awswit
   ```

   GitHub provenance は Apple notarization / platform code-signing certificate と同じではない。current workflow は
   それらを保証しない。Gatekeeper/SmartScreenや組織policyを無効化して回避せず、platform signature必須環境では
   承認済みsource build/packageを使う。

5. archive を staging directory へ展開し、`awswit --version` と `awswit --help` が成功してから PATH 上の
   executable を置換する。system-wide path では管理者権限と組織 policy に従う。

後続releaseが生成する`awswit-installer.sh` / `awswit-installer.ps1`をarchiveの代わりに使う場合、standard modeは
install先が未登録ならUnixでgenerated env scriptと複数shell dotfileを編集・作成し、Windowsでuser
`Environment.Path`へ追加し得る。これはexecutable discoveryの設定であり、awswit hook/completionの導入ではない。
PATHやdotfileを変更させない場合はinstallerをdownload・検証してから次のように実行する。

```bash
AWSWIT_NO_MODIFY_PATH=1 sh ./awswit-installer.sh
# CI等でreceipt/updater/PATH変更を持たないflat layoutにする場合:
AWSWIT_UNMANAGED_INSTALL=/approved/bin sh ./awswit-installer.sh
```

```powershell
$env:AWSWIT_NO_MODIFY_PATH = '1'
& .\awswit-installer.ps1
# または: $env:AWSWIT_UNMANAGED_INSTALL = 'C:\approved\bin'
```

`AWSWIT_INSTALL_DIR`はinstall baseを指定するが、それ自体はPATH変更を禁止しない。manual archive installationは
startup fileを編集しない。0.31の正確なUnix対象は[pinned installer template](https://github.com/axodotdev/cargo-dist/blob/v0.31.0/cargo-dist/templates/installer/installer.sh.j2#L728-L747)を基準にし、一般contractは[cargo-dist usage](https://axodotdev.github.io/cargo-dist/book/installers/usage.html#path)、
[PowerShell](https://axodotdev.github.io/cargo-dist/book/installers/powershell.html#adding-things-to-path)を参照する。

### 2.2 Build from an audited checkout

current `Cargo.toml` と `release-plz.toml` は `publish = false` であり、crates.io publication はrelease contractに
含まれない。`release-plz.toml`の`git_only = true`により、release PRのversion baselineはcargo registryではなく
cargo-distが公開したGit tagから得る。
repository checkout から検証用に入れる場合:

```bash
cargo install --locked --path .
```

build には Rust 1.94 以上が必要である。prebuilt executable の利用に Rust/Python/Node/AWS CLI は不要である。
AWS command を実際に使うには、その command 自身（例: AWS CLI）が別途必要になる。

### 2.3 Install the shell hook

Hook は child executable から変更できない parent shell に、検証済み activation frame を適用する。`awswit init`は
rc fileへhook lineを自動挿入しない。installerによるPATH設定とは別に、使用するshellのstartup fileへ一つだけ追加する。

#### Bash

```bash
# ~/.bashrc
eval "$(awswit init bash)"
```

#### Zsh

```zsh
# ~/.zshrc
eval "$(awswit init zsh)"
```

#### Fish

```fish
# ~/.config/fish/config.fish
awswit init fish | source
```

#### PowerShell

```powershell
# $PROFILE
Invoke-Expression ((awswit init powershell) -join [Environment]::NewLine)
```

ここで評価するのは installed executable が生成する static hook code である。profile selection 後の response を
`eval` / `Invoke-Expression` してはならない。hook 自身が response を allow-list data frame として検証する。
PowerShell native commandのstdoutはline object列になるため、hook全体をnewlineでjoinしてから一度だけ評価する。単純な
`awswit init powershell | Invoke-Expression`は行単位評価になり、functionを正しく定義しないため使用しない。

startup file を reload するか新しい shell を開き、次を確認する。

```bash
type -a awswit
awswit --version
awswit doctor
```

PowerShell は `Get-Command awswit -All` を使う。hookはcapture対象のactivation/unsetとdoctor subprocessだけに
`AWSWIT_HOOK=1`とshell名の`AWSWIT_SHELL`をscopeし、unrelated childへexportしない。`doctor` の
`hook: detected` はwrapper-scoped markerだけを意味し、shell名やhook/executableのversion一致までは検証しない。

### 2.4 Optional static completions

```bash
mkdir -p ~/.local/share/bash-completion/completions
awswit completions bash > ~/.local/share/bash-completion/completions/awswit

mkdir -p ~/.zfunc
awswit completions zsh > ~/.zfunc/_awswit

mkdir -p ~/.config/fish/completions
awswit completions fish > ~/.config/fish/completions/awswit.fish
```

PowerShell は生成内容を profile/setup 方針に合わせて保存・読み込む。`completions SHELL` は static command/option
grammar だけを生成する。`init SHELL` hook は別途、completion 時にinternal completion feedを呼んで current catalog
のactivatable profile namesを追加するため、通常はhookをloadすればdynamic profile completionも有効になる。catalogを読めない
場合はprofile candidatesを返さず、activation時と同じexact resolutionは変わらない。completion query はbare
`list` のため、その時点のenvironment/default sourcesを使い、入力途中のcommand-local source optionsまでは解釈しない。
shell menuがdisplay labelとexact insertion valueを安全に分離できないため、zero-cell scalarを含むnameはcompletion候補から
除外する。raw `list --format names`、exact activation、visible escape付きTUIからは利用できる。

## 3. Upgrade

### 3.1 Preflight

1. 現在の executable path と version を記録する。

   ```bash
   command -v awswit
   awswit --version
   ```

2. 現 executable を rollback 用に別名または package-manager cache で保持する。
3. release notes で activation protocol/history schema の変更を確認する。
4. 新 archive の checksum/provenance/SBOM を Install 手順どおり確認する。

### 3.2 Replace and validate

executable は同一 filesystem 上の staging path で検証し、できるだけ atomic rename/package-manager transaction で
置換する。置換後:

```bash
awswit --version
awswit --help >/dev/null
awswit list --format names | head
awswit doctor
```

`head` が早く pipe を閉じても awswit は broken pipe を正常終了として扱う。

source checkout install の upgrade:

```bash
cargo install --locked --force --path .
```

同じaudited checkoutとlockfileを使い、build前後のversionを記録する。

### 3.3 Reload the hook

pre-1.0 では executable と hook を version-matched artifact として扱う。startup file が毎回
`awswit init SHELL` を評価する構成なら、新しい shell を開く。hook を generated file として固定している場合は、
新 executable から再生成して source/reload する。

古い hook が未知 protocol を受けた場合は、部分適用せず拒否する設計である。それでも upgrade 完了条件は
「新 shell で bare `awswit` を起動し、cancel/selection 後も terminal と環境が正常」である。

## 4. Normal health checks

### 4.1 Local catalog

```bash
awswit list --format names
awswit list --format human
```

- names が exact、重複なし、決定的順序であること;
- `[sso-session ...]` / `[services ...]` が profile として出ないこと;
- expected valid config-only / credentials-only profile が存在すること。

custom sources を調べる場合は両方を明示できる。

```bash
awswit doctor \
  --config-file /approved/path/config \
  --credentials-file /approved/path/credentials
```

CLI path は `AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE` より優先する。missing explicit path は意図違いを
隠さないため hard error である。non-defaultなrelative pathはこのcommandのworking directoryに対するlexical absolute
pathへ固定される。command-line pathは常に、relative environment pathは絶対化した値だけがactivation/execへ伝播され、
後続の`cd`で別fileを参照しない。

### 4.2 Offline doctor

```bash
awswit doctor
awswit doctor --format json
```

確認項目:

- `offline: true` — remote validation ではない;
- source path と origin（`default` / `environment` / `command_line`）;
- total parsed profile-record count（invalid recordsを含み、`list`件数とは異なり得る）;
- current `AWS_PROFILE` / `AWS_DEFAULT_PROFILE`;
- hook marker detected/not detected;
- credential override **variable names**;
- catalog issue count/details。

exit 0 は local scan が完了したという意味で、AWS login、account、permission、token freshness、network、clock、
`credential_process` の成功を保証しない。JSON は secrets を出さない設計だが、path、profile name、account ID、role
ARN、SSO URL は組織 metadata になり得る。ticket/chat へ添付する前に review/redact する。

### 4.3 Activation smoke

non-production test profile で実施する。

```bash
awswit test-profile
test "$AWS_PROFILE" = test-profile
awswit unset
```

`unset` はloaded hook経由でのみ成功し、profile/region 5変数だけを解除する。direct executable invocationは
`HOOK_REQUIRED`となりprotocolをterminalへ表示しない。`AWS_CONFIG_FILE`、`AWS_SHARED_CREDENTIALS_FILE`、credential
provider variables は解除しない。すべてを消す意図で `unset` を過大評価しない。

child-only smoke:

```bash
awswit exec test-profile -- sh -c 'printf "%s\n" "$AWS_PROFILE"'
```

untrusted input を扱う automation では `sh -c` を使わず、`awswit exec PROFILE -- executable arg...` と argv を直接
渡す。

PowerShellではfunction callに対するunquoted `--`がshell自身に消費される。通常の
`awswit exec test-profile -- aws sts get-caller-identity`はhookがclosed grammarから一意な境界を復元する。先頭が`-`の
profileは`awswit activate --profile=-h`を使う。separator formや先頭が`-`のchild executableを意図する場合は、literalを
quoteする。

```powershell
awswit activate --profile=-h
awswit activate '--' -h
awswit exec test-profile '--' -option-shaped-command
```

最後の形をunquotedで渡すとhookはexit 2と固定messageで拒否し、commandを実行しない。これは未知の境界を推測して別commandを
動かさないためのfail-closed contractである。

## 5. Troubleshooting

### 5.1 Diagnostic decision table

| Code/symptom | Meaning | Operator action |
|---|---|---|
| `HOOK_REQUIRED` | executable cannot modify parent shell | correct `init SHELL` lineをloadし、新 shellで再試行。one commandなら`exec` |
| `CLI_INVALID` | command line grammar error | raw argumentは再表示されない。`awswit --help` / `awswit COMMAND --help`で構文を確認 |
| `TTY_REQUIRED` | interactive stdin/stderr is not a terminal | exact `awswit activate PROFILE`をhook経由で使うか、automationでは`exec` |
| `PROFILE_NOT_FOUND` | case-sensitive exact nameがcatalogにない | `list --format names`で確認。typoを自動補正しない |
| `NO_PROFILES` | parsed recordが0件、またはinteractive pickerにactivatable profileが0件 | source pathsとdoctor issuesを確認 |
| `CONFIG_READ` | explicit source missing、open/read/parser hard failure | doctorのpath/origin、file type、permissionを確認。secret値をticketへ貼らない |
| `SOURCE_PATH_INVALID` | current directoryに対してsource pathを絶対化できない | accessible working directoryから再実行するかabsolute source pathを指定 |
| `PROFILE_CONFIG_INVALID` | selected provider graph/required metadataが安全に使えない | doctor JSONのissueを修正し、別profileへ暗黙fallbackしない |
| `CREDENTIAL_OVERRIDE` | ambient provider variablesがprofileに優先し得る | namesをreview。意図的ならscopeを理解して`--clear-credential-overrides` |
| `PATCH_INVALID` | executableがunsafe/non-Unicode valueまたは内部frame不整合を検出 | executable/hookを同versionでreload。profile/pathのcontrol charactersを除去 |
| `TERMINAL_FAILURE` | TTY state/event I/O failure | terminal復旧手順後、新terminalで再試行 |
| `HOME_UNAVAILABLE` | default source pathを決めるOS user directoryがない | OS user dirsを修正、または両source pathsを明示 |
| `HISTORY_DISABLED` / `HISTORY_READ` / `HISTORY_WRITE` warning | preference state unavailable | activation safetyには影響しない。OS user dirs/path/permission/schema/corrupt backupを確認 |
| `OUTPUT_FAILED` | stdoutへ書けない（broken pipe以外） | output target、filesystem/pipe、callerを確認。broken pipe自体はsuccess |
| `COMMAND_NOT_FOUND` / exit 127 | executableをPATHまたは指定pathで発見できない | PATHとexact executable pathを確認 |
| `COMMAND_NOT_EXECUTABLE` / exit 126 | permissionまたはexecutable formatにより直接起動できない | permission、file type、scriptのvalid shebangを確認 |
| `COMMAND_FAILED` | OSがcommand startに失敗 | sanitized OS error、resource limit、runtime policyを確認 |
| `INTERNAL` | executable invariantが成立しない予期しないfailure | version、OS、再現手順、sanitized stderrを添えてissue報告。別profileへ暗黙fallbackしない |

実際の code name は対象 executable の stderr と `--help` を優先する。pre-1.0 では追加され得る。
profile名が`-`で始まる場合、Bash/Zsh/Fishでは`awswit -- PROFILE`または`awswit activate -- PROFILE`を使う。
PowerShellでは`awswit activate --profile=NAME`を優先し、separator formなら`awswit activate '--' PROFILE`のように
literalをquoteする。特に`-h` / `--help` / `-V` / `--version`はexplicit separatorより前だとdisplay requestになる。
`exec`ではcommand用separatorと競合させず、`awswit exec --profile=-h -- COMMAND...`のnamed formを使う。PowerShellで
child executableが`-`から始まる場合は`'--'`をquoteする。曖昧なunquoted formは実行せずexit 2になる。
hook側がcaptured frameを拒否する場合はcodeなしのfixed message
`awswit: rejected an invalid activation response; environment unchanged`を出す。frame内容は表示せず、parent environmentは
変更しない。これはexecutableが出す`PATCH_INVALID`とは別のvalidation layerである。

Unix `exec` は unknown executable format を shellへfallbackしない。script を実行する場合は executable permission と
valid shebang（例: `#!/usr/bin/env bash`）を付けるか、意図した interpreter を明示的 executable として渡す。
Unixで`PATH`自体が未設定ならbare executableは127になる。CWDを暗黙検索しない。明示的なempty `PATH`はPOSIXのempty
componentとしてCWDを検索するため、untrusted directoryでは設定しない。Windowsでは`.bat`/`.cmd`を直接渡すと126で
拒否する。batch semanticsが意図的な場合だけ`awswit exec PROFILE -- cmd.exe /d /s /c ...`のようにshellを明示し、
quoting/injectionの責任境界が`cmd.exe`側へ移ることを理解する。

### 5.2 Hook is installed but switching does not persist

1. `type -a awswit` / `Get-Command awswit -All` で shell function が executable より優先されることを確認する。
2. `awswit doctor` の hook marker を確認する。
3. startup file 内の duplicate/old hook を除く。
4. `command awswit activate PROFILE` を直接実行しても parent shell は変わらないのが正常である。出力 frame を手で
   eval しない。
5. readonly、integer、array、case-transform等のspecial attributesを持つtarget `AWS_*` variableがあるBash/Zsh
   sessionでは、hookは全件適用前に拒否する。宣言元と理由を確認し、plain/exported scalarへ戻すかclean shellで試す。

Fishでpersistent universal `AWS_*` variableが存在する場合、hookの`UNSET`はそれを永続削除せず、current sessionに
zero-element unexported global shadowを置いてchildへのexportを止める。永続設定を削除する意図がある場合はawswitの
範囲外として、定義元を確認してFishの設定管理手順で行う。

### 5.3 Credential override rejection

`doctor` で names だけを確認する。CI、credential manager、container runtime、IDE、parent terminal が注入している
可能性がある。値を print する調査 command は runbook の標準手順にしない。

`--clear-credential-overrides` の scope:

- `awswit PROFILE --clear-credential-overrides`: current shell から検出済み conflicts を除く;
- `awswit exec PROFILE --clear-credential-overrides -- CMD`: child だけから除く。

前者はその shell で後続 command 全体に影響する。迷う場合は child-only `exec` を選ぶ。

`AWS_ACCESS_KEY_ID` / `AWS_ACCESS_KEY` / `AMAZON_ACCESS_KEY_ID`、対応するsecret names、session/security token
namesは、complete/partial/duplicateの別や`credential_source = Environment`の有無にかかわらず、presentなnameをすべて
conflictとして扱う。AWS CLI/SDKのenvironment-provider precedenceがdirect credentialを使ってselected role assumptionを
迂回し得るためである。必要なdirect credentialをrole sourceとして「許可」する設定は設けない。使用範囲を確認したうえで
`--clear-credential-overrides`により全detected namesを除くか、clean environmentから実行する。

`AWS_EC2_METADATA_SERVICE_ENDPOINT`はcomplete source chainが
`credential_source = Ec2InstanceMetadata`を明示する場合だけintent-provenとして許可される。endpoint valueや到達先identity
をawswitが検証するわけではない。`AWS_LOGIN_CACHE_DIRECTORY`はcomplete chainがexplicit `login_session` providerに
到達する場合だけ許可されるが、cache pathや内容を検証するわけではない。`AWS_BEARER_TOKEN_BEDROCK`はBedrock requestで
profile credentialを置換し得るため、profile種別にかかわらずconflictになる。必要な場合もvalueをticketへ貼らず、
child-only `exec`での明示clearを優先する。

Java SDK v1固有の`AWS_CREDENTIAL_PROFILES_FILE`はcatalogとは別fileを選び得るため常にconflictになる。clear後はJava v1の
default credentials pathへ戻る。同SDKはstandard `AWS_SHARED_CREDENTIALS_FILE`を読まないので、awswitのCLI/environment
source overrideをJava v1へ伝播できるとは扱わない。non-default fileが必要ならJava application/providerを明示設定するか、
[support終了済みのJava SDK v1](https://aws.amazon.com/blogs/developer/announcing-end-of-support-for-aws-sdk-for-java-v1-x-on-december-31-2025/)から
supported SDKへ移行する。

### 5.4 Windows PowerShell and .NET consumer precedence

AWS Tools for PowerShellで`Set-AWSCredential`を`-StoreAs`なしに実行すると、session credentialが
`$StoredAWSCredentials`へ設定され、`AWS_PROFILE`より先に使われる。generated PowerShell hookはvisibleかつnon-nullな
同variableをpresenceだけで検出し、bare TUI、shortcut、explicit `activate`をbinary起動前に拒否する。credential objectの
field/valueは読まない。次を実行してsession overrideだけを解除し、再試行する。

```powershell
Clear-AWSCredential
awswit PROFILE
Get-STSCallerIdentity
```

`Clear-AWSCredential`は引数なしならcurrent shellのdefault credentialを解除し、stored profileを削除しない。
[AWS cmdlet reference](https://docs.aws.amazon.com/powershell/v5/reference/items/Clear-AWSCredential.html)
`$StoredAWSCredentials`をcallerの`Private` scopeへ置くとhook functionから見えない。AWSと無関係なnon-null valueを同名に
置いた場合は安全側のfalse positiveになる。`exec`のexternal childはPowerShell objectを継承しないためguard対象外である。
最終principalは`Get-STSCallerIdentity`で確認する。credential objectやそのpropertyをticketへ貼らない。

WindowsのAWS SDK for .NET / AWS Toolsは、location未指定時に同名profileをSDK Store
（`%USERPROFILE%\AppData\Local\AWSToolkit\RegisteredAccounts.json`）から先に探す。awswitはこのstoreをcatalog化せず、
name listing APIもcredential materializationの可能性があるため呼ばない。shared fileを確実に使うconsumerでは、次の
いずれかをapplication startup/client construction前に設定する。

- `AWSConfigs.AWSProfilesLocation`または`CredentialProfileStoreChain.ProfilesLocation`を、awswitが選択したabsolute
  shared credentials pathへ設定する;
- `AWSConfigs.DisableLegacyPersistenceStore = true`としてlegacy SDK Store lookupを無効化する（直接
  `NetSDKCredentialsFile`を使うcodeには適用されない）。

[.NET profile resolution](https://docs.aws.amazon.com/sdk-for-net/v4/developer-guide/creds-assign.html)と
[AWSConfigs API](https://docs.aws.amazon.com/sdkfornet/v4/apidocs/items/Amazon/TAWSConfigs.html)をconsumer versionに合わせて
確認する。同名collisionの有無をawswitの成功から推測せず、non-productionで`GetCallerIdentity`を実行してからproductionへ
進む。

### 5.5 Legacy AWS SDK for Go v1

AWS SDK for Go v1の`session.New` / default `NewSession`は、`AWS_SDK_LOAD_CONFIG`がtruthyでない限りshared
`config`を読み込まず、shared `credentials`だけを読む。`AWS_PROFILE`が正しく切り替わっても、config側のrole、SSO、regionを
無視して別providerへfall throughし得る。awswitはconsumer-specific behavior flagを親shellで勝手に所有・上書きしない。

可能なら[support終了済みのGo SDK v1](https://aws.amazon.com/blogs/developer/announcing-end-of-support-for-aws-sdk-for-go-v1-on-july-31-2025/)から
v2へ移行する。移行までの明示的なlegacy controlは、application側の
`session.NewSessionWithOptions(session.Options{SharedConfigState: session.SharedConfigEnable})`を優先する。environmentで
管理する場合は、そのapplicationの起動scopeに限定して次を設定する。

```bash
AWS_SDK_LOAD_CONFIG=1 awswit exec PROFILE -- ./legacy-go-application
```

[Go v1 shared-config contract](https://docs.aws.amazon.com/sdk-for-go/v1/developer-guide/sessions.html)を確認し、
non-productionの`GetCallerIdentity`でprincipalを検証する。`awswit unset`は`AWS_SDK_LOAD_CONFIG`を解除しない。

### 5.6 Terminal left in an abnormal state

SIGKILL、host crash、terminal loss は cleanup 不能である。別 terminal が使えるなら対象 process の状態を確認してから、
元 terminal で次を実行する。

```bash
stty sane
reset
```

画面に入力が見えない場合も command を入力して Enter できることがある。復旧後、再現時の OS、terminal、shell、
awswit version、signal、sanitized doctor output を採取する。credential values や full credentials file は採取しない。

### 5.7 History corruption or permission failure

Typical history paths:

- Linux: `${XDG_STATE_HOME:-$HOME/.local/state}/awswit/history.json`
- macOS: `~/Library/Application Support/awswit/history.json`
- Windows: `%LOCALAPPDATA%\awswit\history.json`

OS user-directory resolutionにより異なる場合がある。lock は sibling `history.json.lock`、automatic corrupt backup は
`history.json.corrupt-*` である。

Malformed/oversized history は退避に成功すれば自動退避後、empty history から継続する。退避できなければ original を
残して warning とし、新しい schema も古い executable が上書きしない。
手動対応が必要なら、実行中の awswit を終了し、history と `.lock` の owner/ACLを確認し、削除より先に timestamp 付き
backup directoryへ copy/moveする。history は favorite/usageだけなので、失っても AWS config/credentials は失われない。

Unix file は awswit が `0600` を強制するが、parent directory ACL は operator responsibility である。shared account や
network filesystem の lock/rename/fsync semantics は platform guarantee を確認する。
historyまたは`.lock`がFIFO/device/symlink等のspecial fileならawswitはblockingせず、historyを無効化してbest-effort
warningを出す。書込み対象なのでhistoryのsymlinkは許容しない。
AWS config/credentialsはread-only sourceである。Unixではregular fileへのfinal symlinkを許容するが、direct
FIFO/deviceおよびそれらへのsymlinkはnon-blockingに`CONFIG_READ`として拒否する。Windowsではreparse pointと
direct device/custom namespace（named pipeを含む）をopen前に拒否し、通常のdrive/UNC pathは許可する。
許容外ならapproved regular fileへ置換して再試行する。

### 5.8 Catalog issue

Human doctor は summary、JSON は issue details 用である。path + line + fixed issue code から section を調べる。
`list` / completion / TUIはactivatable profilesだけを出す一方、doctorのprofile countとissuesはinvalid recordsも含む。
exact nameがcatalogに存在してもinvalidなら`PROFILE_CONFIG_INVALID`になる。

- malformed section/property は次 section まで quarantine される;
- duplicate section/metadata は順序依存設定を除去する;
- missing/cyclic `source_profile` を切る;
- modern SSO の referenced `[sso-session]` と start URL/SSO region を確認する;
- modern SSO のaccount ID/role nameは両方を設定するか、bearer-only用途では両方とも省略する;
- legacy SSO の required inline fields を確認する;
- static key tuple の片側だけを直すか削除する。access/secretをconfigとcredentialsへ分割して相互補完させず、
  tupleは一つのphysical source file内で完結させる。一方がcompleteでも他方にpartial keyが残ればissueなので、不要な
  partial fieldsを削除する。
- SSO、AWS Login (`login_session`)、role、`credential_process`、static keys、`credential_source`等を同じprofileで
  競合させない。
- `source_profile_not_credential_capable`では、参照先をcomplete static keys、`credential_process`、AWS Login、
  credential-form SSO、valid web-identity role、またはvalid non-self role chainにする。config-only、bearer-only SSO、
  self-static roleはdirect selectionできても別roleのsourceにはできない。
- `credential_source_without_role_arn`では、そのfieldを削除するか同じprofileへintended `role_arn`を設定する。
  standalone `credential_source`はSDKごとにreject/ignoreされ別identityへfall throughし得るため、AWS公式guideのsource例より
  現行AWS CLI/botocore/Go SDK挙動を優先してdirect/upstream selectionの両方を止める。
- `environment_credential_source_cannot_be_selected_safely`では、roleのsourceをfile上の`source_profile`、
  `credential_process`、credential-form SSO、ECS/IMDS等へ変更する。generic `AWS_PROFILE` selectionではEnvironment keysを
  残すとrole bypass、clearするとsource喪失になるため、このshapeをforce-enableしない。

secretを貼り付けた issue report を作らない。awswit は token expiry/login stateを確認しないため、catalog修復後の auth failure
は AWS CLI/SDK/IdP のrunbookへhandoffする。

### 5.9 AWS Login profile authentication

`list` / TUIの`AWS Login` labelは`login_session` keyが存在するというlocal metadataであり、login済み、cache有効、または
consumer SDKがprovider対応済みという証明ではない。AWS Login profileのauthentication failureでは、AWS CLIのversionと
組織policyを確認し、対象名をexactに指定してprovider所有のflowを実行する。

```bash
aws login --profile PROFILE
awswit exec PROFILE -- aws sts get-caller-identity
```

既定cacheはUnixの`~/.aws/login/cache`またはWindowsの`%USERPROFILE%\.aws\login\cache`であり、
`AWS_LOGIN_CACHE_DIRECTORY`により変更され得る。awswitはこのdirectoryと`login_session` identityを読まず、cacheの削除・
refresh・expiry判定をしない。logout/incident responseはAWS CLIの`aws logout --profile PROFILE`または組織runbookに従い、
awswit historyと混同しない。

## 6. Rollback

Rollback trigger examples:

- new executable fails `--help`/`doctor` or cannot read previously valid catalog;
- official hook rejects every frame after reload;
- terminal restoration regression;
- artifact checksum/provenance mismatch;
- incident response がbinary integrityを否定できない。

Procedure:

1. 現在の shell で新しい activation を止める。必要なら `awswit unset` を使うが、credential/path variables は別管理で
   あることに注意する。
2. 新 executable とその checksum/provenance evidence を quarantineし、上書き削除しない。
3. 検証済みのprevious executableをPATH上へrestoreする。
4. `--version` / `--help`を確認する。
5. previous executableからhookを再生成し、新shellでloadする。new hookとold executableを混在させない。
6. `doctor`、`list --format names`、non-production activation/exec smokeを実施する。
7. history warning がある場合も、新しいschema fileを手動downgradeしない。older executableはunsupported schemaを
   untouchedにする設計なので、復旧まではrankingなしで運用できる。

RollbackはAWS config/credentials/SSO/AWS Login cacheを変更しない。これらを同時に戻す必要がある場合は、awswitとは別changeとして
承認・監査する。

## 7. Security incident response

### 7.1 Scope and first actions

Suspected cases:

- checksum/provenance不一致、unknown binary/hook;
- shell startup file の無承認変更;
- terminal/profile metadata injection;
- awswit output/historyにcredential値が現れた疑い;
- config/credentials/history permissionまたはowner異常。

First actions:

1. 影響 shell で awswit と AWS command の新規実行を止める。
2. startup file が疑わしい場合、rc fileを読まないclean shell/別hostから調査する。
3. binary、hook/startup file、release checksum/provenance、filesystem metadata、sanitized doctor outputをpreserveする。
4. credential漏えいの可能性があれば、awswitのlocal-only設計を理由に無害と仮定せず、AWS/IdP側で該当credential/tokenを
   revoke/rotateし、CloudTrail等のauthoritative auditを確認する。
5. compromised hostでは同host上のhash tool/outputも信頼せず、known-good environmentでartifactを検証する。

### 7.2 Evidence collection

Collect:

- `awswit --version` and resolved executable path;
- release archive/checksum/provenance verification result;
- OS/architecture, shell/version, terminal, install/upgrade time;
- sanitized `doctor --format json` after reviewing organizational metadata;
- error code, exit status, exact reproduction shape with placeholder profile/path values;
- history/config/credentials file owner, mode/ACL, timestamps—not credential contents.

Do not collect raw environment dumps, `set`, `env`, full credentials files, SSO/AWS Login cache, `credential_process` output,
or screen recordings that expose secrets.

### 7.3 Containment and recovery

- remove/quarantine untrusted executable and hook from PATH/startup;
- deploy a verified known-good release and regenerate the hook from that executable;
- restore least-privilege owner/ACL; on Unix history files should be `0600`;
- rotate/revoke secrets in their authoritative system;
- validate with offline doctor, then a non-production AWS identity check using the organization-approved AWS CLI runbook;
- compare incident behavior against the guarantee boundary: profile metadata leak, credential value leak, command execution,
  and remote credential misuse are separate severities.

### 7.4 Reporting

A report should state the affected awswit version/commit or release tag, verified artifact identity, platform, reproduction,
expected/actual behavior, and redaction performed. Do not include access keys, tokens, raw provider output, or private config.

## 8. Release operations

### 8.1 Workflow ownership and contract

`.github/workflows/release-plz.yml` runs `release-plz release-pr` on `main`. It creates/updates the version/changelog PR and
explicitly dispatches CI for a created or updated release PR because events produced by `GITHUB_TOKEN` do not trigger the normal
push/PR workflow. `publish = false`、`git_only = true`、`git_tag_enable = false`、`git_release_enable = false`を固定し、
crates.io publish、tag、GitHub Release、publishing workflow dispatchは行わない。`Cargo.toml`も`publish = false`と
`package.metadata.dist.dist = true`を併記し、binary-only packageをregistryから分離しつつcargo-dist対象には残す。

`.github/workflows/release.yml` is cargo-dist 0.31.0 generated output with a deliberately small, CI-audited hardening delta:

1. least-privilege default `contents: read` permissions;
2. per-tag non-cancelling release concurrency;
3. `main` + `v`-prefixed SemVer input guard (`dry-run` excepted);
4. publishing plan、host、transaction直前の3地点でImmutable Releasesとrelease tag rulesetを再検証;
5. generated shell snippets are confined to three exact marker regions whose quoting and grouped output redirection are
   ShellCheck-clean while normalizing byte-for-byte to cargo-dist output;
6. four verified cargo-dist install regions—plan, platform build, global build, and host—that download the pinned 0.31.0
   archive and reject a SHA-256 mismatch before execution;
7. a native-runner smoke region that extracts each of the five packaged executables and executes `--version` / `--help` before
   upload;
8. no inherited repository/organization secrets for the local reusable artifact gate;
9. dispatched release commitと同じ`GITHUB_SHA`をcheckoutし、Rust 1.94 fmt/clippy/test/performance、cargo-deny、
   pinned actionlint + ShellCheck、workflow/docs contract、4-shell parser/runtimeを再実行するreusable release gate。ordinary CIとrelease gateの
   marker-delimited 4-shell conformance bodyはbyte-identical contractとして検査する;
10. canonical build set 21 filesとpublish直前set 17 filesのexact validation、個別/統合checksum、全archiveのpath/type/
   duplicate/member-count/uncompressed-size safety、installer syntax、cargo-dist/package/tag manifest identity、
   CycloneDX namespace/package/dependency-component identityを検証する。source archiveはreview済みのcanonical
   source set全体とmember/directory集合がexactで、各fileがrelease checkoutとbyte-identicalでなければならない;
11. a fail-closed host condition that requires plan, every local build, global build, and custom gate to succeed before the
   write/id-token job can run; and
12. expected commitへのatomic Git ref creation、numeric draft release ID ownership、exact remote asset-name comparison、
    publication、ownership-aware failure cleanupからなるtransaction。

Hardened regions are marker-delimited and the host condition is matched as one exact expression.
`Cargo.toml` additionally carries `allow-dirty = ["ci"]` so cargo-dist generation can run while its own generated workflow is
under comparison. CI temporarily removes that setting, regenerates the pristine workflow, restores `Cargo.toml`, normalizes
every audited delta above to its generated equivalent, and requires an exact byte-for-byte match. It also checks the exact
five-target plan、binary-only Cargo/release-plz booleans、action pin、canonical asset list。Do not hand-edit any other generated
section. To change cargo-dist configuration, edit `Cargo.toml`, regenerate, then reapply/review the audited delta. A marker
missing, duplicated, or absorbing generated content, a missing host condition, a cargo-dist checksum/version mismatch, or any
residual generated-workflow drift is a release-contract failure.

Canonical 15 pre-host assets are six archives（five platform archives + `source.tar.gz`）、their six individual `.sha256`
files、`sha256.sum`、Bash installer、PowerShell installer。The build gate additionally requires six granular dist manifests,
for an exact 21-file scratch set. Host removes those granular manifests and adds `dist-manifest.json` + `awswit.cdx.xml`,
yielding the exact 17 files permitted for attestation/upload. An unexpected file is a failure, not an extra artifact to
publish.

Archive validation permits only regular files/directories, rejects absolute/traversal/backslash/colon/control-character/
empty/dot components and duplicate members, and caps each archive at 8,192 members and 512 MiB uncompressed.
These structural budgets are checked before ZIP CRC expansion or source-member reads.
`source.tar.gz` must contain exactly the reviewed package-version directory tree: release/workflow policy、Cargo metadata、
license/README、all requirements/design/operations documents、all Rust and shell sources、and all contract fixtures. Every
file must be non-empty UTF-8 and byte-identical to the checked-out release SHA. Dist manifests
must declare cargo-dist `0.31.0`, tag `v{Cargo package version}`, and exactly one matching app. The final CycloneDX document
must identify the same package/version and contain dependency components. Parsing XML/JSON alone is not sufficient evidence.

### 8.2 Repository protection prerequisite

Publishing is permitted only when both repository controls are already active:

1. GitHub **Immutable Releases** is enabled;
2. one or more active tag-target rulesets collectively apply to `refs/tags/v*`, restrict both update and deletion, and have
   an empty `bypass_actors` list. Protection may be split across rulesets, but every contributing ruleset must be active,
   apply to the exact release tag, and provide no bypass actor.

As of 2026-08-25, `atriumnook/awswit` reports Immutable Releases `enabled=false` and no tag rulesets. Therefore a real release
is intentionally fail-closed until a repository owner installs these controls. This runbook does not authorize awswit or its
workflow to mutate repository settings.

After the owner applies the settings through approved repository change control, verify with a token that can read repository
rulesets:

```bash
export GITHUB_REPOSITORY=atriumnook/awswit

gh api \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2026-03-10' \
  "repos/$GITHUB_REPOSITORY/immutable-releases" | jq -e '.enabled == true'

gh api --paginate --slurp \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2026-03-10' \
  "repos/$GITHUB_REPOSITORY/rulesets?targets=tag&includes_parents=true&per_page=100"

# For each active tag ruleset ID returned above:
gh api \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2026-03-10' \
  "repos/$GITHUB_REPOSITORY/rulesets/RULESET_ID?includes_parents=true"
```

Do not infer compliance from a ruleset name. Confirm `target=tag`、`enforcement=active`、include pattern coverage for the
exact `refs/tags/vX.Y.Z`、no matching exclusion、empty `bypass_actors`、and rule types `update` + `deletion`。The workflow runs
the same semantic validator during plan、host、and immediately before tag creation.

### 8.3 Publishing procedure

1. Review and merge the release-plz PR only after its explicitly dispatched CI succeeds.
2. Complete section 8.2 and retain the settings/change-review evidence. A failed policy check is a stop condition, not a
   reason to bypass a gate.
3. Confirm the merge commit/version on `main`. Do not pre-create the tag or GitHub Release; the transactional step rejects an
   existing tag/release.
4. In GitHub Actions, manually run the **Release** workflow from ref `main` with `tag = vX.Y.Z` (or valid v-prefixed SemVer
   prerelease). Use `dry-run` to exercise planning/build/gates without publishing.
5. Wait for all five platform builds, global artifacts, and the same-SHA custom release gate. Confirm exact 21-file build
   validation and the Rust/security/dependency/shell gates—not merely successful archive jobs.
6. Host removes granular manifests, adds the final manifest/SBOM, and validates the exact 17-file set **before** attestation.
   Only after all upstream gates succeed does this job hold write/id-token permissions.
7. Immediately before mutation, policy is rechecked. The transaction creates `refs/tags/vX.Y.Z` atomically at
   `GITHUB_SHA`; a pre-existing or concurrently-created ref fails without granting cleanup ownership. It then creates a draft
   release, records its numeric ID, uploads the exact assets, compares sorted local and remote names, reverifies tag SHA, and
   publishes by that ID.
8. Verify the public tag target、immutable published state、exact 17 assets、each checksum、installer、SBOM、and GitHub artifact
   attestations before announcement.

On handled failure, cleanup reads the owned numeric release ID and deletes it only if it is still the expected-tag draft. It
never deletes a published, changed, or merely same-named release. The no-bypass deletion ruleset normally rejects the
best-effort tag cleanup; a correct-SHA orphan tag can therefore remain after the owned draft is removed. This is the intended
tradeoff: protect published tag immutability instead of granting the workflow a standing deletion bypass.

For an orphan, stop retries and inspect both state and ownership:

```bash
release_tag=vX.Y.Z
git ls-remote --exit-code --refs origin "refs/tags/$release_tag"
gh release view "$release_tag" --json databaseId,isDraft,isPrerelease,tagName 2>/dev/null || true
```

If the ref differs from the approved commit or any unowned/published release exists, treat it as a release/security incident;
do not move or delete it automatically. If it is the expected-SHA orphan with no release, a repository owner may use an
approved, time-bounded, audited ruleset recovery change to delete that exact ref. Restore the active no-bypass protection and
rerun the section 8.2 verifier before retrying. Never add a standing workflow bypass or force-move a release tag.

### 8.4 Operator checklist

- [ ] version/tag points at approved `main` commit;
- [ ] release-plz PR CI was explicitly dispatched and passed;
- [ ] Immutable Releases is enabled and the verification response is retained;
- [ ] active/no-bypass tag rulesets cover exact `refs/tags/vX.Y.Z` and restrict update + deletion;
- [ ] format, clippy, unit/integration, Linux PTY, shell parser, advisory/license/source gates pass;
- [ ] reusable release gate checked out the dispatched `GITHUB_SHA` and passed independently of earlier CI;
- [ ] exact 21-file scratch set and exact 17-file final set passed validation;
- [ ] ordinary CI and same-SHA release gate used byte-identical four-shell conformance bodies;
- [ ] all five target archives plus source archive exist and their individual/unified SHA-256 values match;
- [ ] every archive passed safe path/type/duplicate and 8,192-member / 512-MiB uncompressed budgets;
- [ ] source archive member/directory set is canonical and every file matches the checked-out release SHA;
- [ ] Linux archive passes `--version` and `--help` smoke;
- [ ] dist manifest identifies cargo-dist 0.31.0 and the exact package/version/tag;
- [ ] CycloneDX SBOM namespace, package/version, and non-empty dependency components validate;
- [ ] GitHub artifact attestations exist;
- [ ] shell + PowerShell installers exist;
- [ ] installer PATH behavior and `AWSWIT_NO_MODIFY_PATH` / `AWSWIT_UNMANAGED_INSTALL` controls match this runbook;
- [ ] generated hook and executable are from the same release;
- [ ] release notes call out protocol/history/JSON compatibility changes;
- [ ] rollback executable and instructions are available;
- [ ] publishing dispatch is from `main` with the approved `v`-prefixed SemVer tag; `dry-run` never publishes;
- [ ] no tag/release existed before dispatch; a failed run was inspected for owned draft and protected orphan tag state;
- [ ] no release is announced before the custom artifact gate succeeds。

User-visible behavior and guarantee limits are in [the detailed specification](../design/specification.md).
