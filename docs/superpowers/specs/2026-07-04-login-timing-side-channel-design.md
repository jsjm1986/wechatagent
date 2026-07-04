# 修复：登录用户名枚举时序侧信道

> 分支 `fix/login-timing-side-channel`（从 origin/main 切）
> 来源：本会话定向审计（auth agent）候选 #1，已逐行亲验为真 bug（低危）

## 1. 缺陷（对最新代码 100% 亲验）

`authenticate`（session.rs:84-91）先按 username 查 `admin_users`：

```rust
let user = coll
    .find_one(doc! { "username": username }, None)
    .await?
    .ok_or(AuthError::InvalidCredentials)?;   // 用户不存在 → 立即返回，从不跑 verify
let ok = password::verify_password(password_plain, &user.password_hash)?;  // 用户存在 → 跑完整 Argon2
```

- 用户名**不存在** → `ok_or` 立即返回 `InvalidCredentials`，**从不**调 `verify_password`，秒回。
- 用户名**存在** → 跑完整 Argon2id verify（OWASP m=19MiB/t=2，耗时 ~30-50ms）后才因密码错返回。

后果：攻击者对 `POST /api/auth/login` 发 `{username:"admin", password:"x"}` vs `{username:"nope", password:"x"}`，响应时间差（秒回 vs ~30-50ms）可稳定区分「用户名是否存在」→ 枚举出真实 admin 用户名，缩小后续爆破面。默认配置即可达。低危（admin-only 端点、Argon2 本身拖慢后续爆破），但是真实的经典时序侧信道。

## 2. 方案（最小改动，标准修法）

用户名不存在时也跑一次 Argon2 verify（对进程级预计算的假 PHC 哈希），支付与「用户存在」等价的耗时，抹平时序差；恒判凭据无效。

- `password.rs`：新增进程级 `DUMMY_HASH`（`LazyLock`，一次性 `hash_password`）+ `verify_against_dummy(plaintext) -> bool`（恒 false）。
- `session.rs`：`find_one` 返 `None` 时先 `let _ = verify_against_dummy(password_plain);` 再返 `InvalidCredentials`。

**关键不变量**：假哈希必须是**合法 PHC**，否则 `verify_password` 走 `PasswordHash::new` 解析失败的**快路径**（不跑 Argon2），反而重新制造时序差。用测试 `dummy_hash_is_valid_phc` 锁住。

## 3. 测试

`password.rs` `#[cfg(test)]` 新增：
- `dummy_hash_is_valid_phc`：`PasswordHash::new(&DUMMY_HASH).is_ok()`（锁住 PHC 合法性不变量）。
- `verify_against_dummy_always_false`：`!verify_against_dummy("anything")` + 空串。

（真实的耗时等价性无法在单测里稳定断言 wall-clock；这里锁的是「假哈希合法 → 一定走 Argon2 慢路径」这一使耗时等价成立的结构不变量。）

## 4. 验证
- `cargo test --lib auth::` 全绿；`cargo test --lib` ≥ 350 / 0（实测 1802 / 0）。
- CI 双门（baseline + integration）。
