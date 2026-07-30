# HC-001 LLM 凭证轮换运行手册

本手册用于处置 2026-07-30 已确认的公开凭证暴露。任何证据、命令输出、工单、提交信息或聊天中都不得记录凭证值、前后缀或可逆摘要。

## 完成标准

只有以下条件全部满足，HC-001 的凭证轮换部分才可关闭：

1. 上游生成的新凭证通过受影响协议的合成请求预检。
2. 正式 `.env` 与 MongoDB Provider 引用原子切换；服务健康、真实模型请求、运行中 ELF、重启计数和开放队列检查全部通过。
3. GitHub Actions repository secret `RSXERMU_KEY` 已同步，并由一次最小真实模型 Actions 任务证明新值可用。Secret 列表只能证明名称存在，不能证明保存的值正确。
4. 上游旧凭证已撤销；合成探测证明旧凭证被拒绝、新凭证仍被接受。
5. 经单独授权处理服务器明文副本，并记录 Actions artifact、fork、镜像层、其它克隆和公开 Git 历史的审计结论。

任一动态验证未执行或证据不完整时，只能记录为未完成，不得以静态检查、mock 或“Secret 名称存在”代替。

## 角色与授权边界

- Provider 所有者：在上游生成新凭证，最终撤销旧凭证。
- 生产变更批准人：批准短暂停止并重启 `wechatagent` 服务。
- GitHub 仓库管理员：批准更新 `jsjm1986/wechatagent` 的 Actions secret `RSXERMU_KEY`。
- 执行人：只通过 owner-only 文件和 stdin 传递新凭证；不得把值放进 argv、环境变量、shell history 或日志。

生产轮换、GitHub Secret 更新、旧凭证撤销、服务器副本删除、Git 历史改写及仓库安全设置变更是不同授权项，不得相互推定。

## 1. 生成并暂存新凭证

在 Provider 控制台生成新的、尚未在任何环境使用的凭证。不要把值发送到聊天或 issue。将它保存为执行人拥有的本地临时文件，并确保没有 group/other 权限。

通过 SFTP 将文件上传到服务器 `/run/wechatagent-hc001-new-key`。服务器文件必须是 root 拥有的普通文件、非 symlink、权限 `0600`。上传完成后只检查所有者、类型和权限，不输出内容：

```bash
chown root:root /run/wechatagent-hc001-new-key
chmod 0600 /run/wechatagent-hc001-new-key
test -f /run/wechatagent-hc001-new-key && test ! -L /run/wechatagent-hc001-new-key
stat -c '%U %G %a %F' /run/wechatagent-hc001-new-key
```

## 2. 只读生产预检

取得“允许只读生产预检”的确认后，在服务器项目目录执行：

```bash
python3 scripts/deploy/rotate_llm_credential.py preflight \
  --new-key-file /run/wechatagent-hc001-new-key \
  --env-file /opt/wechatagent/.env \
  --unit wechatagent \
  --evidence-dir /run/hc001-preflight-evidence
```

预检必须确认：正式健康为绿、所有受管开放队列为空、新旧值不同、MongoDB 中尚无新值引用，并且新凭证至少完成一个协议正确的真实模型探测；任何 active 受影响 Provider 都必须成功。证据仅包含类型化状态和计数，不包含 endpoint 或凭证。

预检失败时停止，不修改生产。不得为了通过而禁用检查、替换成 mock、复用其它 Provider 凭证或更改业务开关。

## 3. 原子切换正式环境

预检通过后，再取得“允许短暂停止并重启正式服务”的明确批准。记录变更窗口和当前健康状态，然后执行：

```bash
python3 scripts/deploy/rotate_llm_credential.py apply \
  --new-key-file /run/wechatagent-hc001-new-key \
  --env-file /opt/wechatagent/.env \
  --unit wechatagent \
  --confirm HC001-ROTATE-LEAKED-LLM-CREDENTIAL \
  --evidence-dir /run/hc001-apply-evidence
```

工具会在停止服务后再次检查队列，原子替换 `.env` 与 MongoDB Provider 引用，再验证健康、真实模型探测、Provider 引用、开放队列、运行中 ELF 和重启计数。失败时会恢复切换前 `.env` 和 Provider 引用并重启旧配置；若自动回滚也失败，必须按 stderr 给出的 `/run` 回滚文件人工恢复，不能继续后续步骤。

## 4. 同步 GitHub Actions Secret

正式环境验证通过后，取得 GitHub Secret 更新批准。在持有本地 owner-only 新凭证文件的可信主机上先运行只读预检：

```bash
python3 scripts/deploy/sync_github_secret.py preflight \
  --repo jsjm1986/wechatagent \
  --secret-name RSXERMU_KEY
```

确认当前 `gh` 身份和目标仓库正确后，通过 stdin 同步；重定向路径只在本机 shell 打开，不会进入子进程 argv：

```bash
python3 scripts/deploy/sync_github_secret.py apply \
  --repo jsjm1986/wechatagent \
  --secret-name RSXERMU_KEY \
  --confirm HC001-SYNC-GITHUB-RSXERMU-KEY \
  < /secure/local/path/new-key
```

工具丢弃 `gh` 的 stdout/stderr，且错误消息不回显输入值。成功输出只证明 GitHub 报告该 Secret 名称存在；GitHub API 不允许回读 Secret 值，因此下一步真实模型任务是必需验证。

## 5. 最小真实模型验证

手动触发只依赖 `RSXERMU_KEY` 的最小 Actions 真实模型任务。检查完整对话内容和执行细节，确认请求确实到达预期 Provider、模型产生业务有效响应、没有 fallback、skip、零调用或假绿，并保存不含凭证的 run URL、commit、job、模型、调用计数和断言结果。

同时再运行一次正式业务路径的最小真实模型请求，确认服务仍健康、开放队列无意外积压、日志中无认证错误。不得只依据 HTTP 200 或 workflow 绿色状态作结论。

若 CI 验证失败但生产新凭证可用，先诊断 Secret 同步或 Workflow 配置；不得撤销旧凭证。若生产验证失败，按生产轮换工具的回滚结果处理，并停止后续步骤。

## 6. 撤销旧凭证并双向证明

只有正式环境和 GitHub Actions 都已证明新凭证可用后，才由 Provider 所有者在上游撤销旧凭证。撤销后使用不含项目代码、业务数据或用户数据的合成请求分别证明：

- 旧凭证返回认证拒绝；
- 新凭证仍可完成协议正确请求；
- 正式服务和最小 Actions 真实模型任务仍通过。

探测证据只记录 `rejected_auth`、`accepted` 等类型化结果，不记录凭证、endpoint、响应头或可逆摘要。旧凭证撤销后，不得把恢复旧值作为回滚方案。

## 7. 后续清理与独立决策

Actions 审计已冻结为 807/807 个当前可枚举 run 日志全部成功扫描，其中 69 个 CI run 合计原值命中 1,795 次；313/313 个当前可下载 artifact 零命中。历史输出根因已修复并由 `workflow-secret-must-be-direct` CI 硬门防回归。日志删除不可逆，仍须取得明确授权后使用冻结检查点先执行只读预检；不得手工扩大目标集合：

```bash
python3 scripts/deploy/delete_hc001_actions_logs.py preflight \
  --checkpoint .tmp/hc001-runs-scan.json
```

只有预检输出 `targetRuns=69 / verifiedRuns=69 / deletedLogs=0` 且批准人确认后，才可使用工具要求的精确认短语执行 `apply`。工具只调用 69 个 `/actions/runs/{id}/logs` 删除端点，不删除 workflow run、artifact、commit 或 branch。删除后必须逐一证明 69 个日志下载端点已不可用，并再次扫描剩余可枚举日志；未执行这些复验时不得记录为已清理。

服务器载体审计早期覆盖 23,722 个普通文件；仓库化只读预检器已按当前部署状态重新覆盖 102,259 个普通文件、正式克隆全部 5,117 个 Git blob，以及部署/回滚目录中的 25 个压缩载体。当前确认范围不是单一的“8 个文件”，必须分三类处理：

1. 10 个普通文件：正式 `.env` 由轮换工具原子替换；其它备份/审计副本在轮换和撤销验证后逐路径替换或删除。10 个路径必须以清理前最新只读预检结果为准，不能复用早期 8 文件清单。
2. 17 个命中压缩载体：16 个数据库归档类 gzip 各命中 2 次；1 个 `source-config.tar.gz` 含 6 个命中环境文件成员。先决定哪些回滚点必须保留；保留项须从已验证的新状态重建，或以独立密钥加密封存，不能只修改压缩包表面元数据。
3. 正式克隆中的 29 个命中 Git blob：普通文件删除和 `git gc` 不能替代历史改写。只有在单独批准公开 Git 历史改写、所有克隆重新同步及部署引用迁移后才能处理；在此之前以旧凭证撤销消除访问能力。

任何服务器清理都必须先运行 `python3 scripts/deploy/audit_hc001_server_carriers.py`，重新确认路径、普通文件类型、所有者、权限和旧值命中数；该工具没有 mutation 模式，现存副本应明确返回 `ordinary_file_copies_present`、`git_object_copies_present`、`archive_copies_present` blocker。不得用递归通配删除，不得删除正式二进制、当前数据库或唯一可用回滚点。处理后必须重复普通文件、Git blob 和压缩载体三类扫描，只记录范围、计数和零命中/保留原因。

以下事项分别评估、分别授权：

- 审计并按保留策略处理 GitHub Actions run/artifact、Packages 元数据和其它克隆。当前仓库没有容器镜像构建/发布链，服务器没有容器运行时；GitHub Packages 仍因当前令牌缺少 `read:packages` 无法枚举，不能把权限失败记为“确认为空”；
- 启用 GitHub Secret Scanning、push protection 和 `main` 分支保护；
- 是否改写公开 Git 历史。历史改写会改变提交 ID，并要求所有克隆和部署引用重新同步，不能作为轮换前置条件。

完成后删除本地和服务器 `/run` 中的新凭证临时文件及只含非敏感状态的临时证据目录。删除前必须精确确认路径；不得删除正式 `.env`、正式发布目录或回滚二进制。

## 停止条件

出现以下任一情况立即停止并请求人类确认：新凭证来源或目标 Provider 不明确；active Provider 协议探测失败；生产队列非空；正式 `.env` 权限或所有者异常；服务健康不稳定；MongoDB 引用计数变化；运行中 ELF 改变；GitHub 目标仓库或登录身份不符；真实模型任务发生 fallback、skip 或零调用；自动回滚失败；待删除副本路径与事件记录不一致。
