# Webhook 签名校验恢复设计（方案 B：对齐 gewe-agent 每账号签名）

- 日期：2026-07-09
- 分支：`fix/webhook-signature-verify`（worktree `.claude/worktrees/roster-debug`，基线 = origin/main `a5fe937` #157 合并点）
- 状态：设计已获用户逐节批准，待 spec 复审 → writing-plans

## 1. 背景与问题

2026-07-09 联调期间，为打通「GeWe 真实消息 → gewe-agent 转发 → wechatagent agent 自动回复」整条生产消息流，在 117 的 `/opt/wechatagent/.env` 追加了 `WEBHOOK_VERIFY_SIGNATURE=false` 并重启。这使 `:3003` 上的 `POST /webhooks/wechat` 成为**公网无鉴权入口**——任何人可 POST 伪造入站消息，触发 AI 对真实客户回复。这是必须回退的生产安全降级。

**核心结论：直接把 `WEBHOOK_VERIFY_SIGNATURE` 改回 `true` 不够，会拒掉每一条转发消息。** 原因是两端的签名方案在联调后已经不匹配——wechatagent 现在校验的是旧方案（早期直连 GeWe 回调时代设计的），而 gewe-agent 侧配上回调密钥后实际发送的是一套全新的签名方案。

### 1.1 两端当前签名形态（均亲验源码，非猜测）

| 维度 | wechatagent 现在校验的 | gewe-agent 现在发送的（配密钥后） |
|---|---|---|
| 头名 | `x-mcp-signature` | `x-webhook-signature` |
| 值格式 | 裸 hex | `sha256=<hex>` |
| HMAC 密钥 | 全局 `MCP_API_KEY` | 每 slot 独立的 `messageWebhookSecret` |
| 签名内容 | 仅 raw body | `"<timestamp>." + raw_body` |
| 时间戳头 | 无 | `x-webhook-timestamp`（毫秒） |
| 签名覆盖范围 | body | 整个转发后 body（含 `_mcp` 信封） |

**wechatagent 侧证据：**
- `src/webhooks.rs:295` — `if state.config.webhook_verify_signature { ... }`，且此校验位于**解析 appId（`:369`）之前**。
- `src/webhooks.rs:297` — 读 `x-mcp-signature` 头。
- `src/webhooks.rs:300` — `verify_hmac_sha256(state.config.mcp_api_key.as_bytes(), &body, provided)`。
- `src/webhooks.rs:1165` — `verify_hmac_sha256(key, body, provided_hex)`：hex 解码 + `HMAC-SHA256` + `verify_slice` 常时间比对；空/解码失败/长度不符 → false。
- `src/config.rs:328` — `pub webhook_verify_signature: bool`。
- `src/config.rs:694` — `parse_bool(&env_or("WEBHOOK_VERIFY_SIGNATURE", "true"))`，默认 true。

**gewe-agent 侧证据（`/opt/gewe-agent/src/`，2026-07-09 从 117 直读）：**
- `app.ts:1112` — `void postMessageWebhook(webhookUrl, forwardedPayload, slot.messageWebhookSecret ?? null)`，即配了密钥就带签名转发。
- `app.ts:1129-1170` — `postMessageWebhook`：`bodyText = JSON.stringify(payload)` 一次序列化，同一字符串既喂 HMAC 又当 fetch body（字节对齐）；密钥存在时 `Object.assign(headers, buildWebhookSignatureHeaders(decryptSecret(secretEncrypted), rawBody))`。
- `webhook-signing.ts`：
  - `computeWebhookSignatureHex(secret, timestampHeader, rawBody)` = `createHmac("sha256", secret).update(Buffer.concat([Buffer.from(`${timestampHeader.trim()}.`, "utf8"), rawBody])).digest("hex")`。
  - `buildWebhookSignatureHeaders` 返回 `{ "x-webhook-timestamp": String(timestampMs), "x-webhook-signature": `sha256=${hex}` }`。
- `security.ts:53` — `decryptSecret`（AES-256-GCM）：gewe-agent 侧密钥**加密落库**，转发时解密成明文再签。用户提供的明文回调密钥即对应 slot 的明文 `messageWebhookSecret`。
- `app.ts:1039` — `forwardAccountMessageWebhook`：`isLikelyMessageCallback(body)` 为 false 时直接 `{ status: "skipped" }`。**只有消息回调才被签名转发；控制事件（testMsg/Offline/Online）不经此转发路径到达 wechatagent。**
- `app.ts:984` — gewe-agent 自身对**入站** GeWe 回调也做 `skewed_timestamp` 时间戳偏差校验，两端方案对称。

**结论：方案 B 的本质 = 让 wechatagent 改成校验 gewe-agent 的新签名方案（每账号密钥 + 时间戳）。**

## 2. 已否决的替代方案

深度重估过三条替代根方案，均不如"对齐签名"：

- **网络层 ACL（iptables 只放行 117 本机源）**：能关死 `:3003` 对外，但属运维态、易漂（gewe-agent 迁机即断），且不利用 gewe-agent 已实现的签名。→ 仅作**可选纵深防御叠加**（§7），不作根方案。
- **gewe-agent 转发到 127.0.0.1 + `:3003` 只绑 loopback**：安全性最高，但要改 gewe-agent 的 SSRF 门（`media-store.ts:332` `assertHostnameResolvesPublic` 现拒回环/私网），把两服务永久绑死同机；且用户已在 gewe-agent 侧配好回调密钥，明确要走签名路。→ 否决。
- **复用 `MCP_API_KEY` 当签名密钥**：省一个字段，但要让 gewe-agent 每 slot 密钥 = wechatagent 的 `MCP_API_KEY`，把两个本不相关的密钥耦死，破坏 gewe-agent 每 slot 独立轮换模型。→ 否决。

## 3. 设计决策（用户已逐项批准）

1. **密钥作用域 = 每账号密钥。** 在 `WechatAccount` 模型加 `webhook_secret` 字段，每账号存自己 slot 的明文密钥，对齐 gewe-agent 每 slot 独立密钥 + 可单独轮换的模型。
2. **防重放 = 校验签名 + 时间戳时效。** 验 HMAC 的同时校验 `x-webhook-timestamp` 与当前时间偏差在窗口内，超窗拒绝。与 gewe-agent 入站侧 skew 校验对称。
3. **验签位置 = 方案 B（fail-closed 全路径验签）。** 建立不变式：**签名开关打开时，任何产生副作用的路径都不得绕过签名门。** 验签点从"解析 appId 之前"下沉到"解析 appId、查到账号密钥之后、任何副作用（含控制事件写 online、写库、喂 Agent）之前"。

## 4. 错误处理与降级

统一以 `400 Bad Request` 拒绝不合法请求，绝不 5xx（避免调用方误判为服务故障而重试放大），也绝不静默放行。开关打开（`webhook_verify_signature == true`）时的判定顺序与结果：

| 情形 | 结果 |
|---|---|
| 缺 `x-webhook-signature` 头 | 400，不产生任何副作用 |
| 缺 `x-webhook-timestamp` 头 | 400 |
| 时间戳非法（非数字/解析失败） | 400 |
| 时间戳与当前时间偏差超窗（默认 ±5 分钟，见下） | 400 |
| 签名值格式非法（非 `sha256=<hex>` 或 hex 解码失败） | 400 |
| 解析不出 appId / 查不到对应账号 | 400（无账号即无从取密钥，等价于无法验签） |
| 账号存在但 `webhook_secret` 未配置 | **400，fail-closed**——开关开着却没配密钥，视为拒绝而非放行 |
| HMAC 比对不通过 | 400 |
| 全部通过 | 进入原副作用流程（控制事件写 online / 写入站消息 / 喂 Agent） |

- **时间戳偏差窗口**：新增 `WEBHOOK_TIMESTAMP_SKEW_SECONDS`，默认 300（±5 分钟），沿用 `config.rs` 现有 `env_or` + `parse` 模式。与 gewe-agent 入站侧 `skewed_timestamp` 校验对称。
- **紧急逃生阀保留**：`WEBHOOK_VERIFY_SIGNATURE=false` 仍是总开关。开关关闭时完全跳过上表所有校验，行为回到联调期形态——**仅作为线上应急回退手段，不是常态**。
- **fail-closed 的理由**：开关打开代表运维已声明"这个入口必须验签"。此时若某账号漏配密钥，放行等于开一个静默后门，与恢复签名校验的初衷相悖；拒绝则会在联调/部署时立刻暴露"密钥没配"，符合"错误应尽早、显式暴露"。

## 5. 测试策略

遵循项目红线：只加测试不降基线（`cargo test --lib ≥ 350/0`；四个 PBT ≥ 33/0），绝不为过测试改业务逻辑。

**纯函数抽取 + 单元测试。** 把验签逻辑抽成一个可单测的纯函数（拟名 `verify_webhook_signature`），入参为密钥、时间戳头、签名头、raw body、当前时间、窗口秒数，返回 `Ok(())` / 具体拒绝原因。围绕它加确定性单测：

1. 正确签名 + 时效内 → 通过。
2. body 被篡改（多一个字节）→ 拒绝。
3. 密钥错误 → 拒绝。
4. 时间戳超窗（正偏/负偏各一）→ 拒绝。
5. 缺时间戳 → 拒绝。
6. `sha256=` 前缀剥离正确；带前缀与不带前缀均能识别（按 gewe-agent 实际发送形态 `sha256=<hex>` 为准）。
7. hex 大小写混合 → 与 gewe-agent 输出的小写 hex 对齐（确认解码大小写不敏感或统一小写）。

**字节对齐样本**：单测里构造与 gewe-agent 完全一致的签名内容 `Buffer.concat(["<ts>.".utf8, rawBody])`，用同一密钥算出期望 hex，硬编码进测试作为金标，确保两端算法逐字节一致（这是方案 B 成败的关键，必须有一条测试锁死）。

**回归保护**：`verify_hmac_sha256`（旧 `x-mcp-signature` 路径）若被新函数取代则连同其测试一并迁移，不留下无人调用的死函数；若保留则注明用途。

## 6. 变更面清单（预估，实现时以实际为准）

- `src/models.rs` — `WechatAccount` 加 `webhook_secret: Option<String>`（紧邻 `mcp_api_key`），并入手写 `Debug` 的掩码列表（`:96` 一带），避免明文密钥进日志。
- `src/webhooks.rs` — 新增 `verify_webhook_signature` 纯函数；把 `:295` 的验签块改为方案 B 位置（appId 解析、查账号之后，任何副作用之前）；改读 `x-webhook-signature` + `x-webhook-timestamp`；控制事件短路（`:335`）下沉到验签之后。
- `src/config.rs` — 新增 `webhook_timestamp_skew_seconds`（默认 300）。`webhook_verify_signature` 已存在（`:328`/`:694`），不改默认值。
- `src/routes/accounts.rs` — 账号创建/更新表单接收并落库 `webhook_secret`（对齐 `:187` 现有 `mcp_api_key` 明文落库约定）；读取路径（`:116`）按需返回掩码。
- `frontend/src/`（账号配置表单）— 加 `webhook_secret` 输入项。**注意 no-human-takeover lint 扫 `frontend/src/` 新增行禁用词**，字段标签用中性词（如"回调签名密钥"），勿触雷。
- `.env.example` — 记 `WEBHOOK_TIMESTAMP_SKEW_SECONDS`，并把 `WEBHOOK_VERIFY_SIGNATURE` 注释更新为"生产应为 true，联调临时可 false"。
- 测试文件 — `src/webhooks.rs` 内 `#[cfg(test)]` 单测（纯函数就地测，无需 Docker）。

## 7. 部署与回退顺序（关键：错序会中断生产消息流）

按此顺序执行，任一步失败即停：

1. **合并到 main**：本分支过 CI（baseline 双门 + no-human-takeover lint + integration job）后合并。
2. **部署 117，`.env` 仍保持 `WEBHOOK_VERIFY_SIGNATURE=false`**：用 git bundle 增量包经 paramiko 上传（GitHub 从 117 不可达），`setsid` 后台构建，构建完 `systemctl restart wechatagent`。此时新代码已上线但验签仍关，消息流不中断。
3. **给账号 102 配置 `webhook_secret`**：值 = gewe-agent slot 102 的明文回调密钥（用户提供）。通过管理端表单或直接 mongosh 写入 `wechat_accounts`。
4. **翻开关**：`.env` 改 `WEBHOOK_VERIFY_SIGNATURE=true` → `systemctl restart wechatagent`。
5. **验证矩阵**（缺一不可）：
   - 吴界（账号 102 managed 真实好友）发一条真实微信消息 → gewe-agent 带签名转发 → wechatagent 验签通过 → AI 正常回复（对照联调期成功链路）。
   - 伪造一条**无签名/错签名**的 `POST /webhooks/wechat` → 返回 400，无副作用（`:3003` 公网无鉴权入口已封死）。
   - 时间戳超窗的请求 → 400。

**为何必须先部署再翻开关**：若顺序反了（先翻 `.env` 再部署旧代码），旧代码仍按 `x-mcp-signature` + `MCP_API_KEY` 验签，会拒掉 gewe-agent 的新签名转发，生产消息流立即中断。第 2、4 步分离即为规避此窗口。

## 8. 可选纵深防御（默认不做，记录备用）

在 117 上加 iptables，仅放行 gewe-agent 所在主机源 IP 访问 `:3003`，把公网面进一步收窄。此为运维态叠加项，与签名校验正交；因易随 gewe-agent 迁机漂移且不利用已实现的签名，**不作根方案，仅在需要额外收口时启用**。

## 9. 代码亲验记录（红线要求）

以下断言均于本设计期当场 Read/Grep/SSH 亲验，非记忆：

- wechatagent 侧：`webhooks.rs:295/297/300/335/369/1165`、`config.rs:328/694`、`models.rs:58/71/96`、`routes/accounts.rs:116/187`、`secret.rs:17` 均已逐行确认（§1.1 与 §6 引用的行号来源）。
- gewe-agent 侧（117 `/opt/gewe-agent/src/`，2026-07-09 SSH 直读）：`app.ts:984/1039/1112/1129`、`webhook-signing.ts`（`computeWebhookSignatureHex` / `buildWebhookSignatureHeaders`）、`security.ts:53` 均已确认。
- 分支 `fix/webhook-signature-verify` = origin/main `a5fe937`（0/0），工作树干净，无需新建分支。

> 实现阶段（writing-plans → 编码）落地任一改动前，须对上述行号再亲验一次（代码可能已变动），不得凭本文件旧引用直接改。
