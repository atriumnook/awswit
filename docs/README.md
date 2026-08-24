# awswit documentation

このディレクトリは、awswit の「なぜ」「何を満たすか」「どう動くか」「どう運用するか」を
分離して管理する。AWS profile や credential のように誤解が事故へ直結する用語は、文書間で
同じ意味を使う。

## 文書マップ

### Requirements — 何を満たすか

- [製品要件](requirements/product-requirements.md) — 対象利用者、機能・非機能要件、非目標、
  リリース判定条件
- [競合・標準仕様調査](requirements/competitive-research.md) — awsume、aws-vault、AWS 公式仕様を
  一次資料から比較した判断根拠

### Design — 現行実装はどう保証するか

- [アーキテクチャ](design/architecture.md) — Module の責務、信頼境界、データフロー、設計判断
- [詳細仕様](design/specification.md) — CLI、catalog、TUI、安全判定、shell protocol、履歴、終了状態の
  実装契約

### Operations — どう導入し、復旧するか

- [運用ランブック](operations/runbook.md) — install、upgrade、doctor、障害対応、rollback、
  security incident 対応

## 契約の読み方

文書の役割が衝突した場合は、次の順で判断する。

1. ユーザーから見える現行挙動は、対象 version の executable と
   [CLI contract tests](../tests/cli_contract.rs) が事実源である。
2. [詳細仕様](design/specification.md) は、その挙動と保証境界を説明する。
3. [製品要件](requirements/product-requirements.md) は、リリースまでに満たすべき結果を定義する。
   「未検証」「保留」と明記した項目は実装済みの主張ではない。
4. [アーキテクチャ](design/architecture.md) は内部設計を説明し、CLI 互換性を追加で約束しない。

`0.0.x` は pre-1.0 である。`list --format json` と `doctor --format json` の machine schema、
activation protocol、history schema は version を持つが、文書に安定と明記されるまでは将来の
minor release で変更され得る。shell hook は executable と同時に更新する。

## 共通用語

- **profile** — AWS 共有 `config` / `credentials` の名前付き設定。AWS principal や有効な
  credential そのものではない。
- **catalog** — 2つの共有ファイルから得た、決定的順序の named profile records、activatable subset、安全な issue の
  snapshot。`list` / completion / TUIはactivatable subsetだけを提示し、`doctor`は全issueを提示する。
- **selection** — activatable subset 内に存在する完全な profile 名。非対話経路では exact match だけが作れる。
  catalogに存在してもsemantic invalidなexact nameはnot-foundではなくconfiguration errorになる。
- **activation** — shell hook が、検証済み patch を現在の shell process に適用すること。
- **execution** — `exec` が、検証済み patch を command の process scope だけに適用すること。
- **credential override** — 選択 profile より優先され得る、既存の `AWS_*` credential provider
  環境変数。awswit は値でなく存在と変数名だけを判断材料にする。
- **configured metadata** — account ID、role ARN、SSO session など設定ファイル由来の表示情報。
  AWS が確認した current identity ではない。
- **best-effort** — 失敗しても安全な profile 選択を妨げない補助機能。現在は利用履歴と favorite が
  該当する。

## 保証境界

awswit が保証するのは、安全な profile 選択と環境への適用までである。access key の保管、STS、
MFA、SSO / AWS Login、token refresh、AWS API による identity 検証は行わない。これらは AWS CLI / SDK、
IAM Identity Center、または aws-vault など、選択した profile の provider に委譲する。

文書内の `MUST` / `MUST NOT` は必須契約、`SHOULD` は通常守るが明示した理由で例外を許す契約、
`MAY` は任意挙動を表す。
