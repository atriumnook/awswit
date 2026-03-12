# コードレビュー指摘修正計画

## Context

9つの専門レビュアー（Correctness, Security, Performance, Concurrency, Maintainability, API/Compatibility, Testing, Principles Guardian, AI Anti-Pattern）による包括的レビューで、Critical 3件、Major 14件、Minor 10件以上の問題が検出された。本計画はマージ条件となるCritical/Major issueと、優先度の高いテスト追加を6つのコミットグループに分けて修正する。

## Group 1: セキュリティ修正

**コミット**: `fix(security): add O_NOFOLLOW to atomic writes, enforce credential file permissions, check fcntl`

### 1-1. atomic_write_restricted に O_NOFOLLOW 追加
- **ファイル**: `src/utils/fs.rs:34-39`
- **現状**: `.create(true).truncate(true).mode(0o600)` — symlink追従する
- **修正**: `.create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW)` に変更
  - `create_new(true)` で既存ファイルへの上書きも防止（truncate不要になる）
  - lock_file_with_permissions (line 90) と同じパターン

### 1-2. クレデンシャルファイルのパーミッション違反をエラーに
- **ファイル**: `src/config/aws_files.rs:93-104`
- **現状**: world-writable検出時に `tracing::warn!` のみで続行
- **修正**: world-writable (`mode & 0o002 != 0`) の場合は `Err(AwswitError::ConfigFileError{...})` を返す
  - group/others-readable (line 105-112) は警告のまま維持（セキュリティリスクは低い）

### 1-3. fcntl F_SETFD の戻り値チェック
- **ファイル**: `src/autorefresh/daemon.rs:288-296`
- **現状**: `F_GETFD`は戻り値チェックあるが、`F_SETFD`の戻り値未チェック
- **修正**:
```rust
unsafe {
    cmd.pre_exec(move || {
        let flags = libc::fcntl(write_fd, libc::F_GETFD);
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::fcntl(write_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    });
}
```

---

## Group 2: シェルスクリプト修正

**コミット**: `fix: propagate PowerShell exit code, remove unused version header, relax profile name validation`

### 2-1. PowerShell終了コード伝播 (Critical)
- **ファイル**: `src/init/powershell.ps1:11-14`
- **現状**: `Write-Error $output; return` — `$LASTEXITCODE`がリセットされる
- **修正**:
```powershell
if ($exitCode -ne 0) {
    Write-Error $output
    $global:LASTEXITCODE = $exitCode
    return
}
```

### 2-2. AWSWIT_VERSION ヘッダ削除
- **ファイル**: `src/shell/export.rs`
- **現状**: line 182 で `AWSWIT_VERSION=...` を出力するが、全シェルスクリプトで未消費。`default` ケースに落ちて `Write-Output` される
- **修正**:
  - line 182: `output.push_str(&format!("AWSWIT_VERSION={}\n", ...))` を削除
  - line 217-218: `generate_unset_output` から `AWSWIT_VERSION=...` 部分を削除し `"AWSWIT_UNSET=1\n".to_string()` のみに
  - テスト更新: version header を期待するテストを修正

### 2-3. プロファイル名バリデーション緩和
- **ファイル**: `src/autorefresh/credentials_file.rs:14-34`
- **現状**: `[a-zA-Z0-9_.-]` のみ許可。AWS CLIの `/` や `:` を含むプロファイル名が使えない
- **修正**: INIフォーマットを壊す文字のみ拒否に変更
```rust
pub fn validate_profile_name(name: &str) -> Result<(), AwswitError> {
    if name.is_empty() {
        return Err(AwswitError::ValidationError {
            message: "Profile name cannot be empty".to_string(),
        });
    }
    if name.chars().any(|c| c == '[' || c == ']' || c == '=' || c == '\n' || c == '\r' || c.is_control()) {
        return Err(AwswitError::ValidationError {
            message: format!(
                "Profile name '{}' contains invalid characters (not allowed: [, ], =, control chars)",
                name
            ),
        });
    }
    Ok(())
}
```
- テスト更新: `/` や `:` が許可されることを確認するテスト追加

---

## Group 3: 並行性・信頼性修正

**コミット**: `fix: improve PID verification on macOS, use DateTime for expiry comparison, re-verify before cleanup`

### 3-1. macOSでのPIDリサイクル対策
- **ファイル**: `src/autorefresh/daemon.rs:399-415`
- **現状**: `#[cfg(not(target_os = "linux"))]` で `None` を返す（検証不能）
- **修正**: macOSブランチを追加し、`ps -p $pid -o comm=` で検証
```rust
#[cfg(target_os = "linux")]
{
    // 既存の /proc/pid/comm ロジック（変更なし）
}
#[cfg(target_os = "macos")]
{
    match std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
    {
        Ok(output) if output.status.success() => {
            let comm = String::from_utf8_lossy(&output.stdout);
            let name = comm.trim().rsplit('/').next().unwrap_or("");
            Some(name == "autoawswit")
        }
        Ok(_) => Some(false),
        Err(_) => None,
    }
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
{
    let _ = pid;
    None
}
```

### 3-2. RFC3339文字列比較をDateTime比較に修正
- **ファイル**: `src/autorefresh/runner.rs:559-573`
- **現状**: `if existing_exp >= new_exp` — 文字列の辞書順比較
- **修正**: `DateTime::parse_from_rfc3339` でパースして比較。パース失敗時は文字列比較にフォールバック
```rust
if let (Some(new_exp), Some(ref profile)) = (expiration, &existing_profile) {
    if let Some(ref existing_exp) = profile.awswit_role_expiration {
        let should_skip = match (
            DateTime::parse_from_rfc3339(existing_exp),
            DateTime::parse_from_rfc3339(new_exp),
        ) {
            (Ok(existing_dt), Ok(new_dt)) => existing_dt >= new_dt,
            _ => existing_exp >= new_exp, // fallback
        };
        if should_skip {
            tracing::debug!("Skipping redundant refresh for {} ...", profile_name);
            return Ok(());
        }
    }
}
```

### 3-3. 期限切れプロファイル削除前の再検証
- **ファイル**: `src/autorefresh/runner.rs:234-251`
- **現状**: 期限切れ判定後に削除。判定と削除の間に更新されると有効なプロファイルを消す可能性
- **修正**: 各プロファイル削除直前にJSONを再読み込みし、まだ期限切れかどうか再確認。更新されていたらスキップ

---

## Group 4: コード品質改善

**コミット**: `refactor: use tokio::process for credential_process, improve region warning, remove unused params`

### 4-1. credential_process を非同期化
- **ファイル**: `src/profile/resolver.rs:609-654`
- **現状**: `std::process::Command` + スレッドベースタイムアウトでtokioランタイムをブロック
- **修正**: `tokio::process::Command` + `tokio::time::timeout` に置き換え
  - `fn get_credentials_from_process` → `async fn get_credentials_from_process`
  - `std::process::Command::new("sh")` → `tokio::process::Command::new("sh")`
  - `mpsc::channel` + `thread::spawn` のタイムアウト → `tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())`
  - タイムアウト時: `child.kill().await`
  - 呼び出し元にも `.await` 追加（既にasyncコンテキスト内なので影響は限定的）

### 4-2. AWSリージョン警告の改善
- **ファイル**: `src/aws/sts.rs:22-29`
- **現状**: 警告ログは出しているが内容を確認。探索結果では既に "us-east-1" を含む警告が出ている
- **確認**: 現在の警告メッセージが十分明確か確認し、必要なら改善

### 4-3. 未使用パラメータ削除
- **ファイル**: `src/profile/resolver.rs:331-337`
- **現状**: `_args`, `_sts_client`, `_cache_manager` が未使用
- **修正**: シグネチャから削除し、呼び出し元（line 180付近）も更新

---

## Group 5: テスト追加

**コミット**: `test: add shell_quote edge cases, control char validation, and profile name tests`

### 5-1. shell_quote() ユニットテスト
- **ファイル**: `src/shell/export.rs` のtestsモジュール
- 追加テスト:
  - 空文字列 → `''`
  - シングルクォート含む → 正しくエスケープ
  - 連続シングルクォート
  - `$HOME` などのシェル特殊文字

### 5-2. validate_credential_value 制御文字テスト拡充
- **ファイル**: `src/autorefresh/credentials_file.rs` のテスト or `tests/unit_tests.rs`
- 追加: `\x01`〜`\x1F` 全制御文字のリジェクト確認ループ

### 5-3. validate_profile_name の新バリデーションテスト
- Group 2-3 の変更に対応するテスト
- `/`, `:` が許可されること
- `[`, `]`, `=`, 制御文字がリジェクトされること

---

## Group 6: 軽微なクリーンアップ

**コミット**: `chore: add cache key version prefix, remove unused config extra field`

### 6-1. キャッシュキーにバージョンプレフィックス追加
- **ファイル**: `src/profile/resolver.rs:21`
- `format!("session-{}-{}", ...)` → `format!("v1-session-{}-{}", ...)`
- 既存キャッシュは無効化される（MFA再入力1回のみ）

### 6-2. AwswitConfig の未使用 extra フィールド削除
- **ファイル**: `src/config/awswit_config.rs:34-35`
- `#[serde(flatten)] pub extra: HashMap<String, serde_yml::Value>` を削除
- `set_value` / `get_value` / `reset_value` のextraフォールスルーも削除

---

## 実行順序

```
Group 1 (セキュリティ) → Group 2 (シェル) → Group 3 (並行性) → Group 4 (コード品質) → Group 5 (テスト) → Group 6 (クリーンアップ)
```

Group 1-3 は独立しており並行作業可能。Group 5 は Group 2-3 の変更後に実施。

## 検証

各グループのコミット後に以下を実行:
1. `cargo build` — コンパイル成功
2. `cargo test` — 全テスト通過
3. `cargo clippy` — 警告なし
4. Group 1 後: `atomic_write_restricted` が `O_NOFOLLOW` を使用していることをコードで確認
5. Group 2 後: PowerShellの `$LASTEXITCODE` 設定を確認、`AWSWIT_VERSION` が出力に含まれないことをテストで確認
6. Group 3 後: RFC3339比較テストが `DateTime` ベースになっていることを確認
7. Group 4 後: `get_credentials_from_process` が `tokio::process::Command` を使用していることを確認

## スコープ外（後続PRで対応）

- resolve_role_chain の分割（Maintainability M11）
- daemon.rs の責務分離（Maintainability M12）
- credential_source のテスト追加（Testing M13 — AWS SDK モックが必要）
- ShellExporter のトレイトベース化（Principles Guardian）
- TUIのインクリメンタルフィルタリング（Performance）
