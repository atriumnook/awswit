# 競合・標準仕様調査: awsume / aws-vault / AWS 共有設定

調査日: 2026-08-13
対象: `awsume`（Trek10、commit `2597f00`）、`aws-vault`（99designs、commit `74e2f7a`）、AWS CLI v2 / AWS SDKs and Tools の公式仕様。二次資料は使用していない。
補足調査: 2026-08-23 — AWS Login credentials provider、IMDS endpoint override、Amazon Bedrock bearer credential。
補足調査: 2026-08-24 — shared-file static tupleのSDK差、PowerShell function argument separator。

この文書は一次資料から設計要求を導く調査記録であり、awswit の実装済み機能一覧ではない。現行契約と
検証状況は [詳細仕様](../design/specification.md) と
[製品要件](product-requirements.md#7-verification-status) を参照する。

> 注記: 99designs の公式リポジトリは現在「abandoned」と明記し、ByteNess fork を後継として案内している。この文書では広く利用されてきた 99designs 実装を比較基準として固定し、実装時には後継 fork の差分も回帰対象に含める。根拠: [99designs/aws-vault README](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/README.md#L6-L12)

## 結論

`awswit` は credential manager ではなく、共有設定にある具体的な profile を安全に選び、標準の `AWS_PROFILE` を適切なスコープへ適用する selector とするべきである。

- `awsume` の強みは「現在のシェルを一操作で切替」、一覧、補完である。一方で Python / pipx、sourced wrapper、環境への一括 credential export、独自キャッシュ、任意 profile への曖昧自動補正まで責務が広い。[導入と alias](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/docs/general/quickstart.md#L3-L62)、[環境変数・キャッシュ](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/docs/general/overview.md#L3-L60)
- `aws-vault` の強みは exact profile を指定した child process / subshell、OS keystore、短期 credential、SSO / `credential_process` 連携である。ただし profile switching より credential custody が主目的である。[README](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/README.md#L10-L106)、[`credential_process` 連携](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/USAGE.md#L95-L115)
- AWS の標準契約は `AWS_PROFILE`、`--profile`、共有 `config` / `credentials`、各 SDK の credential provider chain である。SDK は有効な provider を見つけると探索を止め、標準 provider の期限切れ credential は自動更新する。[共有ファイルと profile](https://docs.aws.amazon.com/sdkref/latest/guide/file-format.html)、[credential provider chain](https://docs.aws.amazon.com/sdkref/latest/guide/standardized-credentials.html)
- AWS Loginは`aws login`がprofileへ`login_session`を設定し、CLI/対応SDKがlocal cacheから短期credentialを読む標準providerである。awswitは設定上の存在だけを分類し、login実行、identity value、cacheを所有しない。[AWS Login credentials provider](https://docs.aws.amazon.com/sdkref/latest/guide/feature-login-credentials.html)
- よって、鍵保管、STS 呼び出し、MFA / SSO token cache、credential refresh、background daemon は AWS CLI / SDK / aws-vault 等に委譲する。これは機能不足ではなく、秘密情報を扱う面積と provider 間の非互換を減らす責務分離である。

## 比較

| 観点 | awsume | aws-vault | AWS CLI / SDK の標準 | awswit への要求 |
|---|---|---|---|---|
| profile switching UX | `awsume PROFILE` で親シェルを書換え。`-l` と shell completion を提供 | `aws-vault exec PROFILE -- CMD`、または専用 subshell。`list` と completion を提供 | 1 command は `--profile PROFILE`、継続利用は `AWS_PROFILE` | TUI 選択、親シェル用 `activate`、副作用を child に閉じる `exec` の両方を提供 |
| shell integration | Unix では `. awsume` alias が必須。installer が rc file へ書込みを試みる | 親シェルは変更せず child process / subshell に適用 | `AWS_PROFILE` は shell session の標準 selector | binary 単体では親環境を変更できないことを明示。`init <shell>` はscriptをstdoutへ生成しhook lineを自動挿入しない。配布installerのPATH管理は別責務として明示する |
| exact / fuzzy | fuzzy は既定 off。ただし有効時、exact miss を prefix → longest-contains → Levenshtein で一意候補へ自動置換 | profile positional argument は必須で、補完 hint は出すが解決は exact section name | `--profile` / `AWS_PROFILE` は named section を指定 | fuzzy は対話 TUI の候補絞込みだけ。決定時は画面上の具体的 profile を選ばせる。非対話指定は exact only、miss は非0終了 |
| SSO / AWS Login | 現行 source 上は SSO-only profile の credential 解決 branch がない（下記参照） | modern `sso_session` と legacy inline SSO fields を読む | SSOに加えて`login_session`によるAWS Login providerを定義 | modern / legacy SSOとAWS Login presenceを分類するが、login・token cacheは所有しない |
| credential 管理 | credential export、STS、MFA cache、role chain、output profile、auto refresh | OS keystore、STS、session cache、rotation、SSO / process cache | provider chain が環境・共有ファイル・SSO・process・workload role 等を解決 | credential 値を保管・表示・複製しない。profile selection と provider diagnostics に限定 |
| 配布 | Python 3.5+ と pip / pipx、複数 Python dependencies | platform 別 Go executable と package manager、SHA-256 release artifacts | AWS CLI 自体の導入が必要だが SDK は各言語 runtime | Rust の単一 executable。Python / AWS CLI / shell framework を必須依存にしない |

### profile switching と shell

AWS の公式仕様では、未指定時は `default`、単発 command は `--profile`、shell 内の複数 command は `AWS_PROFILE` を使い、`--profile` が `AWS_PROFILE` に優先する。[AWS CLI named profiles](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-files.html#cli-configure-files-using-profiles)

`awsume` は child process が親 shell を変更できないため sourced shell wrapper を採用する。[awsume architecture](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/docs/general/overview.md#L48-L60) `aws-vault` は逆に command / subshell を作り、temporary credential をその child にだけ渡す。[aws-vault exec](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/cli/exec.go#L69-L186)

`awswit` は次の二経路を持つのが安全である。

1. `activate`: shell hook が TUI の確定結果を受け、現在の shell に `AWS_PROFILE` を export する。
2. `exec`: `awswit exec PROFILE -- CMD...` が child environment だけに適用し、exit code と signal を忠実に伝播する。

stdout は machine output 専用、diagnostic は stderr 専用とする。キャンセル時は stdout を空にし、親 shell を一切変更しない。profile 名は設定ファイル由来の不信入力として扱い、shell-neutral な allow-list data protocol で渡す。不正名を `eval` 可能な文字列へ連結してはならない。

PowerShellはPowerShell commandに対する`--`をend-of-parameters tokenとして解釈する一方、external commandにはliteral
argumentとして渡す。generated hookが`awswit`をfunctionとしてshadowするため、unquoted separatorはhook到達前に消える。
したがってcommon `exec`形はclosed grammarでcommand境界が一意な場合だけ復元し、option-shaped executable等の曖昧な形は
quoted literal `'--'`を要求する。activationのleading-hyphen profileは`--profile=NAME`を優先する。
[PowerShell parsing contract](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_parsing?view=powershell-7.5#the-end-of-parameters-token)

### exact / fuzzy selection safety

`awsume` は fuzzy が既定では無効だが、任意で有効化できる。[default](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/awsume/awsumepy/lib/config_management.py#L16-L20) 有効時は exact miss を複数アルゴリズムへ渡し、一意の最寄り候補を確認なしで返す。[仕様](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/docs/advanced/fuzzy-matching.md#L1-L55)、[実装](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/awsume/awsumepy/lib/profile.py#L301-L357) production / admin profile の誤選択では影響が大きいため、この自動補正は踏襲しない。

要求は以下とする。

- fuzzy query は候補の表示順位・filter にだけ使い、確定値は常に既存 profile の完全な名前とする。
- 0 件では Enter を無効化、複数件では現在行を明示、確定後は選択 profile を再表示する。
- `--profile NAME`、`exec NAME`、script / CI mode は exact match のみ。候補 suggestion は stderr に出しても自動実行しない。
- 非対話で `default` を含む profile を暗黙選択しない。対話 TUI も current profile がない起動直後は row を armed にせず、production profile を bare Enter で偶発選択させない。

`aws-vault` の positional profile は補完候補を持つが exact string として config loader に渡される。[argument](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/cli/exec.go#L107-L149)、[exact section lookup](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/vault/config.go#L193-L214) この fail-closed 側を非対話 API の基準とする。

### AWS shared config / credentials compatibility

最低限の互換契約は次の通り。

- 既定 path は `~/.aws/config` と `~/.aws/credentials`。`AWS_CONFIG_FILE` と `AWS_SHARED_CREDENTIALS_FILE` を尊重する。[AWS CLI environment variables](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-envvars.html)
- awswit 独自の explicit path option を設ける場合の precedence は `CLI option > environment > default` とする。AWS の一般 precedence も command / code、environment、credentials、config、default の順である。[AWS settings precedence](https://docs.aws.amazon.com/sdkref/latest/guide/settings-reference.html#precedenceOfSettings)
- `config` の profile は `[default]` / `[profile NAME]`、`credentials` は `[default]` / `[NAME]`。`credentials` では `profile ` prefix を付けない。[file format](https://docs.aws.amazon.com/sdkref/latest/guide/file-format.html)
- 同名 profile のidentity/metadataは統合する。credential key が両ファイルにある場合の一般的な優先順位は
  `credentials` 側だが、片方のfileにaccess key、他方にsecret keyを置くsplit partial tupleをportableなproviderとみなしては
  ならない。[AWS shared-file precedence](https://docs.aws.amazon.com/sdkref/latest/guide/file-format.html#file-format-creds)
  botocoreはshared-credentials providerとconfig providerを別段で試し、Go SDK v2はaccess/secretを同一file内に要求して
  incomplete groupを無視する。したがってawswitはcredential valuesを解決せず、config/credentials各sourceのknown-key
  presenceを独立にcomplete/partial診断する。[botocore provider order](https://github.com/boto/botocore/blob/2fd71ea25993e2167f5a530e80cd898960fcdf67/botocore/credentials.py#L169-L250)、
  [Go SDK v2 shared-config contract](https://github.com/aws/aws-sdk-go-v2/blob/bbecb94b8f4abeab32d24a18f8e469421b4ec603/config/shared_config.go)
- STS session token sizeは固定されず、AWSは最大値を仮定しないよう明記している。典型値の4 KiBを
  presence判定の上限に流用せず、awswitは値を保持しないままphysical-line 64 KiB budgetまで受理する。
  [AWS IAM temporary credential guidance](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_credentials_temp_request.html)
- `[sso-session NAME]` と `[services NAME]` は selectable profile ではない。profile から参照される metadata section として扱う。
- unknown key は保持不能でも無視して前方互換にし、unknown section / malformed input は file と line を secret 非表示で診断する。1 profile の異常で全一覧を失わない。
- profile 名の dedup、表示順、filter は決定的にする。locale や hash iteration で初期選択が変化してはならない。

### SSO / IAM Identity Center

AWS が推奨する modern configuration は次の分離を持つ。[IAM Identity Center credential provider](https://docs.aws.amazon.com/sdkref/latest/guide/feature-sso-credentials.html#sso-token-config)

```ini
[profile dev]
sso_session = my-sso
sso_account_id = 111122223333
sso_role_name = SampleRole
region = ap-northeast-1

[sso-session my-sso]
sso_region = us-east-1
sso_start_url = https://example.awsapps.com/start
sso_registration_scopes = sso:account:access
```

- profile 側: `sso_session`, 通常は `sso_account_id`, `sso_role_name`、および service 用 `region`。
- session 側: `sso_region`, `sso_start_url`, `sso_registration_scopes`。`sso_region` と service `region` は別物である。
- 1つの `[sso-session]` を複数 profile から再利用できる。
- bearer authentication 専用では `sso_account_id` / `sso_role_name` が不要な場合があるため、欠落だけで profile を破棄しない。
- legacy configuration は `sso_start_url`, `sso_region`, `sso_account_id`, `sso_role_name` を profile 直下に置き、自動 token refresh を持たない。[legacy configuration](https://docs.aws.amazon.com/sdkref/latest/guide/feature-sso-credentials.html#sso-legacy)
- AWS の SSO authentication token は `~/.aws/sso/cache` に保存される。awswit はこの directory を読まず、書かず、削除せず、期限判定もしない。

`aws-vault` は modern / legacy の両 field と `sso_registration_scopes` を実装している。[config model](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/vault/config.go#L125-L157)、[session merge](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/vault/config.go#L316-L374) 一方、`awsume` 現行 source は非-role profile に access keys / `credential_source` / `credential_process` を要求し、credential flow に SSO branch がない。このため source review 上、SSO-only profile は first-class に解決されない。[validation](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/awsume/awsumepy/lib/profile.py#L52-L96)、[credential dispatch](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/awsume/awsumepy/default_plugins.py#L618-L677)

### AWS Login credentials provider

AWS LoginはAWS CLI v2の`aws login --profile NAME`でbrowser-based authenticationを行い、profileへ
`login_session`を設定する。短期credentialとrefresh tokenは既定で`~/.aws/login/cache`（Windowsでは
`%USERPROFILE%\.aws\login\cache`）に保存され、`AWS_LOGIN_CACHE_DIRECTORY`で場所を変更できる。SDKごとにsupport差が
あるため、profileがlocally coherentでもconsumerがこのproviderを利用できるとは限らない。[providerとsupport matrix](https://docs.aws.amazon.com/sdkref/latest/guide/feature-login-credentials.html)、[AWS CLI login workflow](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sign-in.html)

awswitは`login_session` keyのpresenceだけを`AWS Login` labelへ変換し、そのvalue（通常はidentity ARN）を保持・表示しない。
`aws login` / `aws logout`、cache read/write/delete、expiry判定は行わず、実際のauthenticationはAWS CLI/SDKへ委譲する。

### role source capability とAWS CLI互換

`source_profile`の参照先は、単にINI sectionとしてwell-formedなだけでなく、STS requestをsignできるcredential providerを
供給できなければならない。AWS CLI/botocoreのsource-profile builderが扱うのはcomplete static keys、
`credential_process`、credential formのSSO、web identity/role chain等であり、standalone
`credential_source`やbearer-only SSO、region-only profileはそのままsource providerにならない。
[botocore source-profile providers](https://github.com/boto/botocore/blob/2fd71ea25993e2167f5a530e80cd898960fcdf67/botocore/credentials.py#L169-L250)、
[source resolution](https://github.com/boto/botocore/blob/2fd71ea25993e2167f5a530e80cd898960fcdf67/botocore/credentials.py#L1659-L1732)

AWSのrole guideにはBを`credential_source = Ec2InstanceMetadata`だけで構成しAから`source_profile = B`とする例があるが、
現行AWS CLI/botocoreの実際のprovider builderとは整合しない。
[AWS assume-role guide](https://docs.aws.amazon.com/sdkref/latest/guide/feature-assume-role-credentials.html)
awswitはこの差を隠さず、standalone `credential_source`であるBをparsed diagnosisへ残しつつ
`credential_source_without_role_arn`としてnon-activatableにし、Aもtransitively invalidにする。現行Go SDK v2はこのshapeを
明示rejectし、botocoreはsource providerとして解決しないため、Bを選んで別identityへfall throughする可能性をsuccessと
扱わない。[Go SDK v2 validation](https://github.com/aws/aws-sdk-go-v2/blob/bbecb94b8f4abeab32d24a18f8e469421b4ec603/config/shared_config.go#L1361-L1378)
一方、config-onlyとbearer-only SSOは正当なdirect useがあるためdirect selectionを保つが、別roleのsourceには
しない。complete static keysを同じrole profileへ置き`source_profile`を自己参照するAWS CLI互換の例外は、そのprofileを
top-levelで選ぶ場合だけ許可し、さらに別profileから参照するsourceにはしない。
[botocore self-source exception](https://github.com/boto/botocore/blob/2fd71ea25993e2167f5a530e80cd898960fcdf67/botocore/credentials.py#L1622-L1658)

### environment precedence と衝突検出

profile を選んでも、`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` が環境にあれば共有 profile の credential より優先される。`AWS_PROFILE` と直接 credential env が同時にある場合も直接 credential が勝つ。[AWS CLI credential precedence](https://docs.aws.amazon.com/cli/latest/topic/config-vars.html#credentials) 一般設定も `command / code > environment > credentials file > config file > default` であり、一部 SDK 固有差がある。[AWS settings precedence](https://docs.aws.amazon.com/sdkref/latest/guide/settings-reference.html#precedenceOfSettings) さらにGo SDK v2とJava SDK v1は`AWS_ACCESS_KEY` / `AWS_SECRET_KEY`、Ruby SDK v3とAWS CDK CLIは`AMAZON_ACCESS_KEY_ID` / `AMAZON_SECRET_ACCESS_KEY` / `AMAZON_SESSION_TOKEN`も読むため、selectorの安全境界ではこれらの現役aliasesも検査対象にする。[Go SDK v2 env config](https://github.com/aws/aws-sdk-go-v2/blob/bbecb94b8f4abeab32d24a18f8e469421b4ec603/config/env_config.go#L23-L29)、[Java SDK v1 provider](https://github.com/aws/aws-sdk-java/blob/d866126817fcc10595a3e7cd4b40efe626f05a7c/aws-java-sdk-core/src/main/java/com/amazonaws/auth/EnvironmentVariableCredentialsProvider.java#L22-L25)、[Ruby SDK v3 chain](https://github.com/aws/aws-sdk-ruby/blob/ae7b791f2474e2400d2bce94ec0b923866c5bd23/gems/aws-sdk-core/lib/aws-sdk-core/credential_provider_chain.rb#L137-L145)、[AWS CDK CLI compatibility](https://github.com/aws/aws-cdk-cli/blob/7ff50e77f314548128df33ec9042c17fa0da6c1c/packages/@aws-cdk/toolkit-lib/lib/api/aws-auth/awscli-compatible.ts#L260-L276)

したがって「profile 名は切り替わったが実際の identity は古いまま」を成功扱いしてはならない。

- activation / exec 前に、少なくとも直接 access key tuple、web identity、container credential endpoint、IMDS endpoint、Bedrock bearer credentialに関係する`AWS_*`変数の存在を検査する。値は絶対に表示・記録せず変数名だけ示す。
- direct access/secret/session variablesは、complete tupleかつ`credential_source = Environment` chainでも常にconflictとする。AWS CLI/botocoreのenvironment-driven selectionではEnvProviderがAssumeRoleProviderより先に解決されるため、これをrole sourceとして許可するとselected roleを迂回し得る。[botocore credential resolver order](https://github.com/boto/botocore/blob/2fd71ea25993e2167f5a530e80cd898960fcdf67/botocore/credentials.py#L80-L165)
- `role_arn + credential_source = Environment`は、direct keysを残せばrole bypass、clearすればsource喪失となり、generic `AWS_PROFILE` child contractでは安全に表現できない。SDK固有のexplicit-profile APIならprecedenceを変え得るがawswitはconsumer codeを制御・証明できないため、このshapeはparsed diagnosisへ残しつつactivatable subsetから除く。これはAWS configuration全般をinvalidとする主張ではなく、selectorの保証境界である。
- completeかつacyclicなtarget source chainが`credential_source = EcsContainer`を明示する場合はcontainer provider variablesを、`credential_source = Ec2InstanceMetadata`を明示する場合は`AWS_EC2_METADATA_SERVICE_ENDPOINT`を、`login_session` providerを明示する場合は`AWS_LOGIN_CACHE_DIRECTORY`を意図的なものとして区別する。endpoint/cache value自体は検証しない。ambient variablesだけからprovider intentを推測せず、mismatched providerは許可しない。[IMDS credential provider settings](https://docs.aws.amazon.com/sdkref/latest/guide/feature-imds-credentials.html)、[AWS Login cache setting](https://docs.aws.amazon.com/sdkref/latest/guide/feature-login-credentials.html)
- `AWS_BEARER_TOKEN_BEDROCK`はBedrock requestでprofile credentialと別のservice-specific bearer credentialとして使われるため、profile graphから例外を推測せず常にconflictとする。[Bedrock API key environment variable](https://docs.aws.amazon.com/bedrock/latest/userguide/api-keys-use.html)
- Java SDK v1固有の`AWS_CREDENTIAL_PROFILES_FILE`は別credentials fileへ切り替えるため、catalog inputと一致する証明がなく常にconflictとする。一方、Java v1はstandard `AWS_SHARED_CREDENTIALS_FILE`を消費しないため、non-default path互換までawswitが偽って保証せずconsumer側の明示設定へ委譲する。Java SDK v1は2025-12-31にsupport終了済みであり、新たな互換layerを追加するよりv2への移行を優先する。[Java SDK v1 location provider](https://github.com/aws/aws-sdk-java/blob/d866126817fcc10595a3e7cd4b40efe626f05a7c/aws-java-sdk-core/src/main/java/com/amazonaws/profile/path/cred/CredentialsEnvVarOverrideLocationProvider.java#L24-L41)、[AWSによるend-of-support告知](https://aws.amazon.com/blogs/developer/announcing-end-of-support-for-aws-sdk-for-java-v1-x-on-december-31-2025/)
- AWS SDK for Go v1のdefault sessionは`AWS_SDK_LOAD_CONFIG`がtruthyでなければshared configを読まず、role/SSO/regionを無視し得る。全consumerへbehavior flagを強制しunset時にuser stateを破壊するより、EOL consumerのapplication codeで`SharedConfigEnable`を明示するか起動scopeだけ`AWS_SDK_LOAD_CONFIG=1`とし、v2移行を優先する。[Go v1 session contract](https://docs.aws.amazon.com/sdk-for-go/v1/developer-guide/sessions.html)、[AWSによるend-of-support告知](https://aws.amazon.com/blogs/developer/announcing-end-of-support-for-aws-sdk-for-go-v1-on-july-31-2025/)
- それ以外の競合は既定で fail-closed とし、明示 option によってのみ child / shell から unset する。対話中でも黙ってユーザーの親環境を消さない。
- `AWS_PROFILE` を主要 selector とする。`AWS_DEFAULT_PROFILE` を互換目的で併記する場合は、両者を同じ exact name に保ち、その挙動と `unset` scope を明示する。
- `doctor` / status 表示は「選択 profile」「override している変数名」「参照 config path」を示し、account identity のネットワーク照会は明示操作に限定する。

### Windows consumer固有のprecedence

AWS Tools for PowerShellはcommand parameter、`-Credential`、`Set-AWSCredential`で設定したsession credential、direct
environment credentials、`AWS_PROFILE`の順に探索する。したがって`$StoredAWSCredentials`が残ったsessionでは、環境変数だけを
正しく切り替えても古いidentityが使われる。[PowerShell credential search order](https://docs.aws.amazon.com/powershell/v5/userguide/creds-assign.html)、
[`Set-AWSCredential` contract](https://docs.aws.amazon.com/powershell/v5/reference/items/Set-AWSCredential.html)
PowerShell hookはvisible/non-nullというpresenceだけを調べ、current-shell activationを止める。credential objectは展開しない。
PowerShellのPrivate scopeはchild functionから見えないため完全な証明ではなく、`Clear-AWSCredential`と
`Get-STSCallerIdentity`によるconsumer側確認を残す。external processへsession objectを継承しない`exec`は対象外である。

WindowsのAWS SDK for .NET / AWS Toolsは、profile locationを明示しないと同名profileについてSDK Storeをshared
credentials fileより先に探索する。[.NET profile resolution](https://docs.aws.amazon.com/sdk-for-net/v4/developer-guide/creds-assign.html)、
[PowerShell store order](https://docs.aws.amazon.com/powershell/v5/userguide/creds-assign.html)
安定したcredential-blind/name-only APIなしにSDK Storeをenumerateするとsecret materializationの境界を越えるため、awswitは
storeを読まない。.NET consumerで`AWSConfigs.AWSProfilesLocation`を明示するか、application startupで
`AWSConfigs.DisableLegacyPersistenceStore = true`を設定し、実identityを検証する。
[AWSConfigs API](https://docs.aws.amazon.com/sdkfornet/v4/apidocs/items/Amazon/TAWSConfigs.html)

### credential management との責務分離

aws-vault は OS keystore、STS、session cache、access-key rotation までを所有する。[vault backends / STS](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/README.md#L36-L106)、[credential operations](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/USAGE.md#L369-L424) awsume も credential export / cache / auto-refresh を所有する。awswit の目的にはこれらは不要で、以下を明示的な非目標とする。

- access key / secret / session token の追加、編集、保存、出力、rotation
- STS `GetSessionToken` / `AssumeRole` の独自実行、MFA prompt、credential cache
- SSO / AWS Login、OIDC client registration、`~/.aws/sso/cache` / `~/.aws/login/cache` の管理
- local EC2 / ECS metadata credential server、background refresher / daemon
- plugin runtime や Python module による profile provider 拡張

`credential_process = aws-vault ...`、SSO、AWS Login、web identity、workload role などは共有 profile にそのまま残し、選択後に AWS CLI / SDK の provider chain が解決する。これにより aws-vault 利用者とも競合せず併用できる。

### single-binary distribution

awsume は Python と pip / pipx、および複数 dependency を要求する。[quickstart](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/docs/general/quickstart.md#L3-L29)、[setup.py](https://github.com/trek10inc/awsume/blob/2597f003ca1976604f87cafc13fbea505de5d1ed/setup.py#L18-L44) aws-vault は platform 別 executable と SHA-256 を release target にしている。[Makefile](https://github.com/99designs/aws-vault/blob/74e2f7ac256f4da1efbc8a48a4c0c364e454acd4/Makefile#L1-L40)

awswit の配布 acceptance criteria:

- 各対応 OS / architecture で実行に Python、AWS CLI、Node、shell plugin manager を要求しない単一 executable。
- Linux x86_64 / arm64、macOS x86_64 / arm64、Windows x86_64 を最低 release matrix とする。
- release ごとに SHA-256、署名または provenance、SBOM を公開し、`--version` から package version を確認できる。commit identity は release provenance / repository tag で追跡する。
- bash / zsh / fish / PowerShell のintegration scriptはexecutable自身が生成し、hook/completion lineを自動挿入しない。
  cargo-dist 0.31 installerはstandard modeで、Unixではgenerated env scriptと複数shell dotfileを編集・作成し得て、
  Windowsではuser `Environment.Path`へ追加し得る。この挙動と`AWSWIT_NO_MODIFY_PATH` /
  `AWSWIT_UNMANAGED_INSTALL`をinstall docsで明示する。公式bookのshell説明は`.profile`中心で0.31 templateより狭いため、
  pinned implementationも監査根拠にする。[installer PATH contract](https://axodotdev.github.io/cargo-dist/book/installers/usage.html#path)、
  [cargo-dist 0.31 shell template](https://github.com/axodotdev/cargo-dist/blob/v0.31.0/cargo-dist/templates/installer/installer.sh.j2#L728-L747)、
  [PowerShell behavior](https://axodotdev.github.io/cargo-dist/book/installers/powershell.html#adding-things-to-path)
- static command/option completion は `completions SHELL`、current profile name completion は loaded
  `init SHELL` hook がinternal filtered feedから提供する。display/insertionを分離できないshellではzero-cell nameを
  candidateにせず、候補生成に失敗しても曖昧実行へfallbackしない。raw exact names interfaceは別に保つ。
- credential backend 固有 native library を追加して単一 binary 性を失わない。selector のために keychain dependency を持ち込まない。

## 優先 acceptance scenarios

1. modern SSO、legacy SSO、AWS Login、role / `source_profile`、`credential_process`、credentials-only、`default` を同じ一覧から選べる。
2. TUI query が typo でも候補 filter はできるが、非対話 typo は決して別 profile を実行しない。
3. TUI cancel、terminal resize、壊れた1 section、空ファイル、存在しない任意 path で panic せず、秘密値を出さない。
4. credential env が選択 profile を override する場合、成功表示せず変数名だけで原因を説明する。
5. `exec` は child にだけ profile を適用し、command の exit code / signal を保持する。`activate` は明示 shell hook 経由でのみ親 shell を変更する。
6. `AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE`、同名 profile merge、`[sso-session]` 非選択を AWS 仕様どおり扱う。
7. aws-vault の `credential_process` profile と AWS CLI v2 SSO profileを、awswit 自身が credential を取得せず利用できる。
