# awswit 製品要件

状態: `0.1` release baseline
更新日: 2026-08-24

## 1. 製品目的

awswit は、AWS 共有設定にある profile を発見し、利用者が意図した exact profile を短時間で選び、
現在の shell または一つの command process に安全に適用する Rust 製 CLI/TUI である。

「awsume / aws-vault 以上」は主観的な宣伝文句として扱わない。次の測定可能な結果を品質基準とする。

1. 非対話 typo が別 profile として成功する件数: **0**。
2. 失敗、cancel、不完全な shell response が親環境を部分変更する件数: **0**。
3. awswit の通常出力・diagnostic・history に credential 値が出る件数: **0**。
4. 公式 hook 経由の bare `awswit` で、stdout capture 中にも TUI が起動する。
5. normal exit、I/O failure、SIGINT/SIGTERM/SIGHUP で取得済み terminal state を復旧する。SIGKILL は除外する。
6. 並行 history update で協調する awswit process 間の無関係な更新を失わない。
7. Python、Node、AWS CLI、fzf を selector executable の runtime 前提にしない。

機能数より、identity context の食い違いを防ぐこと、fail-closed、日常操作の短さ、自己回復、責務面積の
小ささを優先する。競合と AWS 標準の根拠は
[competitive-research.md](competitive-research.md) を参照する。

## 2. 対象利用者と jobs

### P1: 複数の AWS 環境を日常利用する開発者

- `awswit` を起動し、数打鍵の fuzzy filter と明示 Enter で current shell を切り替える。
- current profile、favorite、最近の利用、provider metadata から候補を識別する。
- production / administrator profile を typo した名前から暗黙選択しない。

### P2: 一つの command だけ別 profile で実行する利用者

- `awswit exec PROFILE -- COMMAND...` で副作用を command scope へ閉じる。
- command の argument、標準入出力、終了状態を余計な shell 解釈なしで保つ。

### P3: 導入・障害対応を行う利用者

- offline `doctor` で source path、profile count、hook marker、環境 override、catalog issue を調べる。
- credential 値や token を収集せず、binary/hook の upgrade と rollback を再現可能に行う。

## 3. First-stable-release requirements

### Profile catalog

- **FR-CAT-001**: `~/.aws/config` の `[default]` / `[profile NAME]` と
  `~/.aws/credentials` の `[default]` / `[NAME]` を統合する。
- **FR-CAT-002**: path precedence は command option、対応する AWS environment、OS default の順とする。
- **FR-CAT-003**: config と credentials の同名 profile を一つにし、credentials-only profile も候補にする。
- **FR-CAT-004**: modern `[sso-session NAME]` と legacy inline SSO を metadata へ解決する。
  session/services section 自体は候補にしない。
- **FR-CAT-005**: role、SSO、AWS Loginの`login_session`、`credential_process`、`credential_source`、web identity、
  static keys の設定上の存在を分類する。access/secret/session-token values、login-session identity、process
  command text は model に保持しない。
- **FR-CAT-006**: profile 名、同点順位、issue の出力を同じ入力に対して決定的にする。
- **FR-CAT-007**: bounded streaming parser を使い、一つの不正 section から次の section で回復する。
  sourceは16 MiB、physical lineは64 KiB、issuesは1,024件、profile/SSO sessionは合計16,384件を上限とする。
  retained metadataは4 KiBに制限するが、値を保持しないstatic credential presenceは固定長を仮定せずphysical-line
  budgetを上限とする。
  path、line、固定 code 以外の秘密値を error に含めず、最終targetがregular fileでないsourceを読まない。Unixでは
  regular fileへのsymlinkを許容しつつnon-blocking open後にtarget typeを検証し、Windows reparse pointおよび
  direct device/custom namespaceはopen前に拒否する。通常のdrive/UNCとextended-length formは許可する。
- **FR-CAT-008**: missing default file は空、missing explicit environment/CLI file は hard error とする。
- **FR-CAT-009**: missing/cyclic source、invalid provider graph、missing/incomplete SSO、partial static tuple を
  safe issue として検出する。static access/secret tupleはconfigとcredentialsの各physical source内で独立に完結させ、
  一方のpartial tupleを他方のkeyで補完してportableなproviderだと推測しない。locally activatableでもsigning credentialを
  供給できないconfig-only、bearer-only SSO、
  top-level-only self-static roleを別profileの`source_profile`として参照した場合も
  `source_profile_not_credential_capable`として検出し、activation/execを成功扱いしない。
- **FR-CAT-010**: `list`、dynamic completion、TUIはtransitively activatableなprofilesだけを提示する。`doctor`は
  invalid recordsを含むtotal profile countと全safe issuesを提示し、invalid exact requestは`PROFILE_CONFIG_INVALID`とする。
- **FR-CAT-011**: `role_arn + credential_source = Environment`は、generic `AWS_PROFILE` selectionでdirect keyを残すと
  role bypass、clearするとsource喪失になるため、awswit固有のunsafe selection shapeとしてactivatable subsetから除外する。
  これはAWS config全般のinvalidityを主張せず、明示profile APIを制御できないtool boundaryとして診断する。
- **FR-CAT-012**: `role_arn`のないstandalone `credential_source`は、current SDKsがrejectまたはignoreして別providerへ
  fall throughし得るため、parsed diagnosisへ残しつつactivatable subsetから除外する。

### Selection TUI

- **FR-TUI-001**: hook が stdout を捕捉中でも、terminal stdin/stderr を使って TUI を起動する。
- **FR-TUI-002**: stdout は machine output 専用とし、TUI、status、diagnostic を混在させない。
- **FR-TUI-003**: fuzzy query は既存候補の filter/ranking だけに使う。確定値は visible row の完全名とする。
- **FR-TUI-004**: current、favorite、logical recency、provider、region、configured account/role/source を
  credential 値なしで識別できる。
- **FR-TUI-005**: current profile がない起動直後は row を armed にせず、0 match と併せて Enter を無効にする。
- **FR-TUI-006**: Esc は patch なしで cancel し、`Ctrl+C` と Unix termination signal は復旧後に signal semantics を
  保つ。
- **FR-TUI-007**: resize、1x1 を含む狭い terminal、Unicode query/profile、`NO_COLOR`、`TERM=dumb` で
  panic しない。terminal control、bidi control、zero-cell scalarはhuman viewで不可視のままにしない。
- **FR-TUI-008**: Nerd Font と特定 theme を要求せず、色以外の current/favorite/selection cue を持つ。
- **FR-TUI-009**: raw mode、alternate screen、cursor visibility の取得状態を normal return、error、unwind、
  およびUnixのSIGINT/SIGTERM/SIGHUPで可能な限り復旧する。SIGKILL/abortは保証外と明示する。

### Activation

- **FR-ACT-001**: current shell の変更は、生成済み hook を通した activation だけが行う。
- **FR-ACT-002**: Bash、Zsh、Fish、PowerShell hook は shell-neutral allow-list frame を全件検証後に適用し、
  profile/path を code として eval しない。
- **FR-ACT-003**: `AWS_PROFILE`、互換用 `AWS_DEFAULT_PROFILE`、選択 marker `AWSWIT_PROFILE` を同じ値で
  一 transaction に設定する。
- **FR-ACT-004**: region precedence は explicit `--region`、profile region の順とし、存在しなければ
  `AWS_REGION` と `AWS_DEFAULT_REGION` の両方を解除する。
- **FR-ACT-005**: command-line sourceは常に、relativeなenvironment sourceはinvocation時のlexical absolute pathとして、
  対応するAWS path variableへ伝播する。absolute environment sourceは既存値を継承し、default sourceは設定しない。
- **FR-ACT-006**: cancel、not found、invalid config、environment conflict、encoding/protocol failure は committed
  patch を出さず、hook は parent environment を変更しない。
- **FR-ACT-007**: `unset` はactivationと同じhook markerを要求し、awswit の profile/region 5変数だけを解除する。
  credential/path variables は暗黙に解除しない。
- **FR-ACT-008**: frame は version、closed variable allow-list、coherent profile/region pair、unique record、final
  commit marker を持つ。unknown/duplicate/truncated/post-commit record を拒否する。
- **FR-ACT-009**: Bash/Zshはtarget variableがplain/exported scalarであることを全件preflightし、readonly、integer、
  array、case transform等のspecial attributesでpartial mutationを起こさない。
- **FR-ACT-010**: PowerShellがfunction invocation時にunquoted `--`を消費する差を明示する。先頭が`-`のprofileは
  `--profile=NAME`を優先し、separator formを使う場合はliteral `'--'`をquoteしてhookへ渡す。

### Exact resolution and environment safety

- **FR-SAFE-001**: command/script/CI/`exec` の指定名は case-sensitive exact match だけを成功とする。
  prefix、LCS、編集距離による自動置換は禁止する。
- **FR-SAFE-002**: not-found diagnostic は候補を提示してもよいが、提示先を自動実行しない。
- **FR-SAFE-003**: direct static、session/security token、web identity、role environment、container provider、
  IMDS credential endpoint、AWS Login cache、legacy credential-file override、service-specific Bedrock bearer
  credentialに関係する既知`AWS_*` variablesの存在を検出し、値を保持・表示しない。
- **FR-SAFE-004**: standard/legacy/`AMAZON_*`のdirect access/secret/session credential variablesは、
  `credential_source = Environment`を含むすべてのprofileで常にconflictとする。environment-provider precedenceが
  selected role assumptionを迂回し得るため、tuple completenessやalias precedenceを安全性の根拠にしない。
  `credential_source = EcsContainer`を明示するcomplete chainだけcontainer URI/authorization variablesを例外とする。
  IMDS endpoint overrideは`credential_source = Ec2InstanceMetadata`を明示するcomplete chainだけ、
  alternate login cacheは`login_session`を明示するcomplete chainだけで例外とする。Bedrock bearer token、
  legacy credential-file override、mismatched provider、invalid chain は fail-closed とする。
- **FR-SAFE-005**: conflict は既定 reject とし、`--clear-credential-overrides` だけが検出済み conflict names の削除を
  許可する。partial/duplicateを含むdirect-key namesも同じnamed conflictsとして扱い、activation と exec の scope 差を
  明示する。
- **FR-SAFE-006**: profile、region、path、history、environment、external argv を untrusted input とし、shell
  injection、argument reinterpretation、terminal/log forging を防ぐ。
- **FR-SAFE-007**: PowerShell hookはvisibleかつnon-nullな`$StoredAWSCredentials`をcredential objectのfield/valueへ
  触れずに検出し、current-shell activationをfail-closedとする。caller-private scope、external `exec`、.NET SDK Store、
  application-level/custom providerは保証外として具体的なconsumer verification手順を文書化する。
- **FR-SAFE-008**: consumer behavior flagを暗黙所有しない。特にEOL Go SDK v1のshared-config loadingにはapplication側
  `SharedConfigEnable`または起動scopeの`AWS_SDK_LOAD_CONFIG=1`が必要であること、移行とidentity verificationを文書化する。

### Child execution

- **FR-EXEC-001**: `exec PROFILE -- COMMAND...` は validated profile/region/source/clear patch を command process
  だけへ適用する。先頭が`-`のprofileは`exec --profile=NAME -- COMMAND...`で曖昧なく指定できる。
- **FR-EXEC-002**: command を executable + argument vector として起動し、shell で再解釈しない。
- **FR-EXEC-003**: stdin/stdout/stderr を継承する。Unix は process replacement で native exit/signal semantics を
  保つ。Windows はchild status 0..255をそのまま返し、範囲外またはnumeric codeなしを1とする。
- **FR-EXEC-004**: executable not found は 127、permission/not executable は 126 とする。
- **FR-EXEC-005**: Unixのbare executableはinherited `PATH`だけを検索する。PATH未設定は127、明示empty PATHはPOSIXの
  current-directory componentとして扱う。Windowsの`.bat`/`.cmd`暗黙shell起動は126で拒否し、意図する場合だけ
  `cmd.exe`を明示させる。
- **FR-EXEC-006**: PowerShell hookは、unquoted `--`が消費された`exec` invocationについてclosed awswit option grammarから
  command境界が一意な場合だけseparatorを復元する。option-shaped executable等の曖昧な境界はexit 2と固定diagnosticで拒否し、
  callerにquoted literal `'--'`を要求する。推測したcommandを実行しない。

### History and favorites

- **FR-HIST-001**: favorite/history は credential を含まない bounded auxiliary state とする。
- **FR-HIST-002**: one invocation の favorite delta と selection を一つの owner が統合し、最新 on-disk state へ
  merge する。stale snapshot overwrite を禁止する。
- **FR-HIST-003**: cooperating process 間の exclusive lock、unique same-directory temporary file、file sync、atomic
  replace、Unix `0600`、corrupt backup を持つ。
- **FR-HIST-004**: unsupported newer schema は上書きせず、legacy は deterministic best-effort migration を行う。
- **FR-HIST-005**: read/write/lock/location failure は sanitized warning とし、safe selection/activation/exec を
  失敗させない。history/lockのspecial fileはblocking readや永続的変更をせず拒否する。
- **FR-HIST-006**: favorite/useのないempty entryを除き、最大512 entries、最大input 1 MiBに制限する。
  explicit source catalogsを切り替えても他catalogの利用履歴を一回のcommitで削除せず、chooserはcurrent catalogに
  存在するnamesだけへhistoryを適用する。

### Discovery, diagnosis, and shell support

- **FR-CLI-001**: `list` はactivatable subsetについてhuman、exact names、JSONを提供する。namesはdeterministicかつ
  one-per-lineとする。
- **FR-CLI-002**: `doctor` は source/origin、profile count、current profile、hook marker、override names、catalog
  issues を offline で検査し、remote identity verification を主張しない。
- **FR-CLI-003**: `init SHELL` と `completions SHELL` は stdout へ artifact だけを出し、hook/completion lineをrc fileへ
  挿入しない。配布installerによるexecutable directoryのPATH管理は別責務として明示・opt-out可能にする。
- **FR-CLI-004**: help は parent shell を child executable から変更できない理由、hook、activation と exec の差を
  示す。
- **FR-CLI-005**: profile name completion は shell ごとに安全に列挙できることを first-stable release の要件とする。
  static command completion だけの状態を完了とは判定しない。display labelとexact insertionを分離できないshellでは
  zero-cell nameを候補から除き、raw names/exact/TUI経路を維持する。
- **FR-CLI-006**: dynamic completion は観測専用とし、PowerShellでは内部のprofile列挙が呼出元の
  `$LASTEXITCODE`を変更しない。

## 4. Non-functional requirements

- **NFR-001 Security**: awswit 自身は access key、secret、session/bearer token、SSO/AWS Login cacheを
  保存・表示・telemetry送信せず、AWS/network APIを呼ばない。明示した`exec` commandのnetwork behaviorは
  この保証外とする。
- **NFR-002 Reliability**: input-dependent panic を release path に残さず、broken pipe を成功として扱う。terminal と
  shell transaction を PTY/integration test で検証する。
- **NFR-003 Portability**: Linux x86-64/ARM64、macOS Intel/Apple Silicon、Windows x86-64 を build target とし、
  Bash/Zsh/Fish/PowerShell parser を CI で検証する。
- **NFR-004 Distribution**: release は one executable per target、SHA-256、CycloneDX SBOM、GitHub artifact provenance、
  shell/PowerShell installer を生成する。publishing は `main` から `v`-prefixed SemVer を手動dispatchし、artifact
  gate後にtransactional draft/tag/releaseを作る。release commitと同じSHAでRust/dependency/shell gateを再実行し、build/final
  asset setおよびcanonical source tree全fileのbyte identityをexactに検証してからattestする。publishingにはImmutable Releasesとactive/no-bypassなrelease-tag
  update/deletion rulesetを必須とする。installerのPATH mutation、disable control、unmanaged modeを文書化し、公開releaseが
  実際にgateを通過したことを別途確認する。
- **NFR-005 Performance**: 1,000 profiles の catalog load/filter が interactive operation を阻害しないことを benchmark
  fixture で回帰管理する。Ubuntu 24.04 / Rust 1.94 / release build で catalog load+filter p95 <= 200 ms、fuzzy
  filter p95 <= 16 msをCI budgetとする。
- **NFR-006 Accessibility**: state を色だけで表現せず、text marker/label と keyboard-only operation を持つ。
- **NFR-007 Diagnostics**: application runtime error はstable short code、sanitized message、actionable hintをstderrに
  出す。CLI syntax failureはraw argumentをechoしないfixed `CLI_INVALID`、help/version renderingはclapとする。shell
  hookのinvalid-frame rejectionはfixed messageとしframe内容を出さない。raw secret値、raw argv、自動debug telemetryを
  出さない。
- **NFR-008 Supply chain**: lockfile を管理し、format、lint、test、advisory/license/source policy、shell parsing、release
  artifact smoke/checksum/SBOM を automated gate にする。
- **NFR-009 Compatibility**: unknown AWS key/section を前方互換に扱い、awswit 独自 provider/plugin schema を導入しない。

## 5. Explicit non-goals

- access key / secret / token の追加、編集、保管、出力、rotation;
- STS `AssumeRole` / `GetSessionToken`、MFA prompt、SSO / AWS Login、OIDC registration、token refresh/cache;
- credential の有効性、AWS account/principal、permission の remote verification;
- daemon、credential server、plugin runtime、Python module;
- external `fzf`、`AWSWIT_FZF_OPTS`、multiple chooser mode;
- noninteractive fuzzy substitution とその設定 file;
- awswit hook/completion line の rc file への automatic insertion、shell autodetection;
- 一実装しかない filesystem/clock/environment への speculative trait seam;
- long-lived SDK process が既に cache した credential の強制変更。

## 6. Exit contract

| 状態 | result | stdout | stderr |
|---|---:|---|---|
| activation frame生成成功 | 0 | complete allow-list frame | hook status only after apply |
| list/doctor/init/completion成功 | 0 | requested data/artifact | warning only |
| broken pipe | 0 | partial/empty | no panic |
| Esc cancel | 130 | empty | optional status |
| Unix TUI signal | native signal result | empty | no forged text |
| validation/config/safety error | 1 | empty for activation | coded diagnostic + optional hint |
| CLI syntax error | 2 | empty | fixed `CLI_INVALID` + help hint |
| command not found/not executable | 127/126 | process dependent | coded diagnostic |
| successful `exec` | child-native on Unix; numeric portable code on Windows | child-native | child-native |

## 7. Verification status

この表は 2026-08-24 時点の repository evidence を分類する。要件本文はリリース目標であり、この表だけが
「実装済み」の主張を行う。

| Area | Status | Evidence / gap |
|---|---|---|
| exact selection、override policy、argv safety | covered | module + `tests/cli_contract.rs` |
| bounded credential-blind catalog、file-local static tuple、SSO、AWS Login、provider conflicts、activatable filtering | covered | `src/catalog` unit tests |
| TUI state/layout、Esc/SIGTERM restoration | covered on tested paths | unit tests + Linux PTY tests; SIGKILL excluded |
| shell transaction | covered on four CI shell runtimes | shared adversarial corpus + Bash/Zsh/Fish/PowerShell transactional runtime gates; ordinary CI/release-gate body parity check |
| concurrent/multi-catalog/corrupt/versioned history | covered | thread/process/different-catalog/storage tests; non-Unix directory fsync excluded |
| static + dynamic profile completion | covered on four hook runtimes | combined artifacts plus Bash/Zsh/Fish/PowerShell option, collision, literal-space, leading-hyphen, Unicode capture, and zero-cell omission tests |
| 1,000-profile latency budget | covered on fixed CI runner | release-mode 31/101-sample p95 tests; Ubuntu 24.04, Rust 1.94 |
| five-target release artifact publication | **workflow configured, current baseline unpublished** | public `v0.0.2` predates this baseline; publish and inspect a later five-target release |
| remote AWS identity/authentication | out of scope | offline by design |

## 8. Release acceptance scenarios

1. 公式 hook の bare `awswit` が stdout capture 中も TUI を開き、selection 後だけ parent shell を変更する。
2. absent `production-admn` は exact failure となり、existing `production-admin` を実行しない。
3. favorite toggle + selection/cancel/restart と parallel commit で unrelated update を失わない。
4. valid modern/legacy SSO、AWS Login、role/source、process、credentials-only、default を同じcatalogから識別し、
   provider conflictをlist/completion/TUIから除外してdoctor issueに残す。`login_session`の値は保持・表示しない。
5. direct/web/container/IMDS/AWS Login/Bedrock environment override は values なしで説明される。selected chain が
   対応する`Environment` / `EcsContainer` / `Ec2InstanceMetadata` / AWS Login providerを明示した場合だけmatching
   variablesを許可し、service-specific Bedrock bearerを含むそれ以外は既定停止する。
6. invalid/truncated/duplicate frame、Bash/Zsh special-variable preflight failure で parent environment を部分変更しない。
7. TUI Esc/SIGTERM 後の raw mode、cursor、alternate screen が tested PTY で復旧する。
8. `exec` は child scope、argv、output、Unix exit/signal semantics を維持する。PowerShellはunquoted separatorを
   一意な場合だけ復元し、option-shaped child commandの曖昧さを実行せず拒否する。
9. empty/missing/malformed config、corrupt/newer history、0 profile、small terminal、broken pipe が panic しない。
10. profile completion の four-hook contract と、固定runner上の1,000-profile p95 budgetを維持する。
11. config/credentialsに分割されたpartial static tupleをcomplete providerへ合成せず、sourceごとのissueとして停止する。
12. five-target artifacts の checksum、smoke test、SBOM、provenance gate 成功後だけ release を公開する。
