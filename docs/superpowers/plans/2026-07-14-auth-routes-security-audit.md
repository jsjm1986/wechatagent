# auth + routes 安全隔离面深度审查（第二批）Implementation Plan

> **For agentic workers:** 审查工程，非常规代码实现。产出是 findings 台账（docs），不是代码。不适用 SDD 的 implementer→reviewer 双裁决；改由**主控编排**：TaskS 根因层先派并等回 → 结论作基准喂 Task1-5 并行派 → 主控逐条亲验 file:line → 填台账。步骤用 checkbox 跟踪。

**Goal:** 对 auth + routes 安全隔离面（authz/IDOR 落实）做纯代码/设计审查，产出经主控逐条亲验的 findings 台账，合并 docs PR。

**Architecture:** 簇S（授权根因层）先审并等回，其结论（workspace 是否唯一来源、pub helper 暴露面、find_contact_by_id 锁 workspace 的正确姿势）作基准喂给簇1-5；簇1-5 并行审 handler 调用点是否正确锁 workspace → 主控 Read/Grep 逐条复核越权链 + 驳回夸大 → 汇总进单一台账 → docs PR。只审不修。

**Tech Stack:** 无代码产出。纯 Markdown 台账 + git。审查对象 Rust（src/auth/*.rs + src/routes/**/*.rs）。

## Global Constraints

- 分支 `docs/auth-routes-security-audit`，基于含 #206/#207 的最新 origin/main（c3739d4）。
- **只审不修**：本批绝不改任何 .rs。产出纯 docs（台账）。
- **subagent 只读**：不改任何文件；每 finding 附亲验 file:line 贴代码行；先 100% 读懂再下结论；凭猜测打回。
- **subagent 全部继承主会话 Opus**：省略 model 参数（`model:"opus"` 报 400，省略即继承）。
- **主控逐条亲验**：每 finding Read/Grep 复核 file:line + 越权链成立性，驳回夸大。
- **两态**：PLAUSIBLE（读码）/ CONFIRMED（可构造跨 workspace/account 越权访问）。
- **严重度校准防夸大**：High=推荐配置下确定性可达的跨 workspace/account 越权读写或认证绕过；Medium=需多条件叠加/多租户启用才触发/仅信息泄漏；Low=观测/边缘/输入校验无越权后果/就绪债。**⚠️ 单租户默认部署下不可达的隔离缺陷 = 多租户就绪债，不夸大成 High**（memory project_multitenant_isolation_debt 口径）。每条带主控裁定理由。
- **元家族聚焦**：middleware 保证认证但不保证授权，授权靠每个 handler 自觉锁 workspace——找"自觉"漏掉处（新增 handler 忘了 #153 收口模式）。
- 不碰主仓在途工作（主仓被并行会话占 feat/principal-auth-exemption）。

---

### Task 0: 建台账骨架

**Files:** Create `docs/superpowers/specs/2026-07-14-auth-routes-security-audit-findings.md`

- [ ] **Step 1: 写台账头部 + 字段模板**

头含审查范围（6 簇 + 文件清单）、方法论、严重度校准口径（含单租户不可达=就绪债原则）、元家族说明。字段模板逐字：

```
### [X-NN] 一句话标题
- 入口频道: —
- 所属簇: S|1|2|3|4|5
- 类型: IDOR|认证|授权|输入校验|敏感泄漏|就绪债
- 严重度: High|Medium|Low（主控裁定理由）
- 现象/风险:
- 越权链: （谁能越权访问谁的什么资源；非越权类填 —）
- 根因（亲验 file:line）:
- 复现设想:
- 验证状态: PLAUSIBLE|CONFIRMED
- 修复建议:
- 状态: Open
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-14-auth-routes-security-audit-findings.md
git commit -m "docs(audit): auth+routes 安全审查台账骨架(第二批)"
```

---

### Task S: 根因层（授权共享层）审查 —— 先派并等回

**审查对象:** `src/auth/`（全部 5 文件 681 行）+ `src/routes/shared.rs`（2427，授权/落库 helper）

**Interfaces:**
- Produces: 簇S findings（S-NN）+ **审查基准**（喂给 Task1-5）：workspace 是否唯一来源、pub helper 暴露面、find_contact_by_id 锁 workspace 的正确姿势、AuthenticatedAdmin.current_workspace 的可信度。

- [ ] **Step 1: 派审查 subagent（只读，继承 Opus），等它回**

dispatch 指令要点：
- 审 src/auth/ 全部（middleware/session/jwt/password/mod）+ routes/shared.rs 的授权/落库 helper（validate_account:138 / find_contact_by_id:167 / upsert_contact_from_value:184(pub) / apply_contact_changes:632(pub) / apply_memory·playbook·domain_changes / ensure_operating_memory:255）。
- **重心**：①current_workspace 是否唯一 workspace 来源、有无从请求体/query 取 workspace 的旁路 ②pub（非 pub(super)）helper 跨模块暴露面是否被误用绕过 workspace ③find_contact_by_id 等按 id 取单条是否强制锁 workspace ④validate_account 是否校验 account 归属当前 workspace ⑤认证链本身有无绕过（白名单、JWT 验签、session TTL）。
- **产出基准**：明确回答"handler 该怎么锁 workspace 才算安全"，供 Task1-5 当审查标尺。
- 硬约束（先读懂+file:line+只读+两态+严重度带理由）。报告写 `.superpowers/audit2/cluster-S-report.md`。

- [ ] **Step 2: 主控逐条亲验 + 提炼基准**

对 subagent 每个 finding Read/Grep 复核；提炼"安全锁 workspace 的正确姿势"作为 Task1-5 dispatch 要点。

- [ ] **Step 3: 填台账（簇S）+ Commit**

```bash
git add docs/superpowers/specs/2026-07-14-auth-routes-security-audit-findings.md
git commit -m "docs(audit): 簇S 授权根因层 findings(主控亲验)"
```

---

### Task 1-5: 资源域簇审查（TaskS 完成后并行派）

> 五簇结构相同，仅审查对象与重心不同。每簇：派只读 subagent（继承 Opus，带 TaskS 基准 + IDOR 检查清单）→ 主控逐条亲验 → 填台账 → commit。报告写 `.superpowers/audit2/cluster-{1,2,3,4,5}-report.md`。

**IDOR 检查清单（喂给每簇 subagent，逐 handler 核）：**
1. workspace 是否来自 AuthenticatedAdmin.current_workspace（非请求体/query 可伪造来源）？
2. DB 查询 filter 是否含 workspace_id？
3. 按 id 取单条时是否像 find_contact_by_id 那样锁 workspace（防越 workspace 取他人对象）？
4. list 端点是否漏 workspace 过滤（返回全租户数据）？
5. account_id 是否校验归属当前 workspace？

**每簇 subagent 硬约束**：先 100% 读懂再下结论；每 finding 附亲验 file:line 贴代码行；只读不改；两态；严重度带理由（**单租户不可达=就绪债不夸大成 High**）；元家族=找 handler 忘锁 workspace 处。

- [ ] **Task 1 客户数据域**：contacts.rs(1980) + conversations.rs + tasks.rs + reviews.rs + send_ledger.rs + operation_view.rs + contract_snapshot.rs。重心=客户 PII/对话/任务的越权读写。派→亲验→填台账（1-NN）→commit `docs(audit): 簇1 客户数据域 findings`。
- [ ] **Task 2 配置/凭证域**：management.rs(3004) + llm_providers.rs(699) + accounts.rs + souls.rs + playbooks.rs + prompt_templates.rs + domain_profiles.rs(1236) + domain_schemas.rs + domains.rs。重心=LLM 凭证/账号密钥/prompt 的越权读写+敏感泄漏。派→亲验→填台账（2-NN）→commit `docs(audit): 簇2 配置凭证域 findings`。
- [ ] **Task 3 媒体/运营动作域**：campaigns.rs(1168) + media_assets.rs(604) + referral_cards.rs + ask_human_inbox.rs(789) + principal_escalations.rs + simulations.rs + products.rs + chunk_locks.rs + assets.rs。重心=发送/引荐/圈人动作面的越权触发。派→亲验→填台账（3-NN）→commit `docs(audit): 簇3 媒体运营动作域 findings`。
- [ ] **Task 4 admin/指标/观测域**：admin_*.rs(7) + observability.rs(859) + evolution.rs(1255) + evaluations.rs + outcomes_autonomy.rs + outcome_metrics.rs + lessons_learned.rs + guides.rs + guide_profile.rs + behavior_signal_metrics.rs + events.rs。重心=admin 操作 + 指标读端点越权（复核 #153 收口后新增）。派→亲验→填台账（4-NN）→commit `docs(audit): 簇4 admin指标观测域 findings`。
- [ ] **Task 5 knowledge 端点层**：src/routes/knowledge/（10 文件 11544 行：sources_meta 1122/wiki_edit 1092/repair 862/verify 663/catalog/chat/crud/digest_inbox/import/mod）。重心=导入/验证/修复/wiki 编辑/来源管理写操作的越权 + chunk 归属校验。派→亲验→填台账（5-NN）→commit `docs(audit): 簇5 knowledge端点层 findings`。

---

### Task E: 台账收尾 + push + PR

- [ ] **Step 1: 汇总头** —— 总 findings 数、严重度分布（H/M/L）、越权类元家族归纳、后续 P0-P3 修复路线（若有 High 优先级高于第一批 5 个 Medium）。
- [ ] **Step 2: 交叉去重** —— 扫全台账去重跨簇重复（如共享 helper 缺陷被多簇各报一次，归并到簇S 留痕）。
- [ ] **Step 3: Commit + push（显式 refspec）+ PR**

```bash
git add docs/superpowers/specs/2026-07-14-auth-routes-security-audit-findings.md
git commit -m "docs(audit): auth+routes 安全审查台账收尾(严重度分布+修复路线)"
LOCAL=$(git rev-parse HEAD)
git push origin HEAD:refs/heads/docs/auth-routes-security-audit -u
git ls-remote origin refs/heads/docs/auth-routes-security-audit   # 亲验 tip==LOCAL
gh pr create --head docs/auth-routes-security-audit --base main --title "..." --body "..."
gh pr view docs/auth-routes-security-audit --json number,headRefName,baseRefName,headRefOid  # 核身份
```

- [ ] **Step 4:** docs-only PR 走 paths-ignore，后端 job 大概率 skip（同 PR#178/#207）。核 CI 无意外 FAILURE 后 squash merge（不带 --delete-branch，worktree 铁律）。

---

## Self-Review

**1. Spec coverage:** 6 簇 → TaskS + Task1-5 一一对应；只审不修+台账格式 → Task0 + 各 Step 填台账；主控亲验 → 各 Step 2；IDOR 清单+元家族+严重度校准 → Global Constraints + 各簇要点；越权链字段 → Task0 模板；后续修复路径 → TaskE Step1。✓ 无遗漏。

**2. Placeholder scan:** 无 TBD/TODO。台账字段模板、IDOR 清单、各簇审查对象+重心、commit message 均具体。TaskE 的 PR title/body `"..."` 执行时按实际 findings 填（届时才知内容），非计划占位。✓

**3. Type consistency:** finding 编号 S-NN/1-NN…5-NN 全计划一致；台账路径与报告文件 `.superpowers/audit2/cluster-{S,1..5}-report.md` 命名一致。✓

## 备注

- TaskS **必须先派并等回**（其基准喂后续），Task1-5 在 TaskS 完成后可一次性并行派 5 个 subagent。主控亲验各簇串行做保质量。
- 审查 subagent 用 general-purpose（只读指令约束，非 Explore——Explore 读摘录漏 read window 外内容不适合审计），继承 Opus。
- 报告文件 `.superpowers/audit2/*.md` 是 git-ignored scratch，不进 commit。
- 本批零代码改动、零 CI 风险（纯 docs）。
