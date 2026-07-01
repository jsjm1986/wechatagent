# CI 已知缺陷与拓扑孤儿项

本文件记录 CI 工作流（`.github/workflows/ci.yml`）中已知、但**尚未修复**的结构性缺陷。每项标注是否为"待用户决策的拓扑孤儿项"——这类改动会影响合并门行为或 CI 拓扑，不在常规测试补齐波次里擅自改，须经用户拍板。

## G1：纯前端改动绕过 `check-no-human-takeover` 红线 lint（待用户决策）

**现象（已亲验，2026-07-01）**：`changes` job 的 `dorny/paths-filter`（`ci.yml:75-85`）把 `backend` 过滤器定义为 `src/**` / `tests/**` / `Cargo.*` / `scripts/**` / `.github/workflows/ci.yml` / `frontend/src/contracts/**`——**不含一般的 `frontend/src/**`**（仅 `frontend/src/contracts/**` 计入 backend）。而 `baseline` job（`ci.yml:91`）的门是 `if: needs.changes.outputs.backend != 'false'`，`check-no-human-takeover.sh`（`ci.yml:114`）挂在 baseline job 内。

**影响**：一个**只改 `frontend/src/`（非 contracts 子目录）**的 PR，`backend` filter 输出 `false` → baseline job 被跳过 → `check-no-human-takeover` 不运行 → 该 PR 里前端新增的"人工接管 / takeover / hand-off"等违反"全 AI 自治、无人工接管"定位的字面量**不被拦截**。而该 lint 的扫描目录本就包含 `frontend/src/`（`check-no-human-takeover.sh:30`），说明设计意图是要覆盖前端的——被 job 级 paths-filter 架空了。

**注意**：非 PR 事件（push / schedule）因无 diff 基线，filter 按"变更"处理 + 下游 `!= 'false'` 兜底会全量跑，故 push 到 main 时不失守；**仅 PR 阶段纯前端改动失守**。

**相关行**：`ci.yml:75-85`（filter 定义）、`ci.yml:91`（baseline if）、`ci.yml:114-124`（lint step）、`check-no-human-takeover.sh:26-31`（扫描目录含 frontend/src）。

**为何不在本波修**：修法有多个方向且各有取舍——(a) 给 `check-no-human-takeover` 单独建一个不受 backend filter 门控的 job（拓扑改动）；(b) 把 `frontend/src/**` 纳入 backend filter（会让纯前端 PR 也跑整个 backend baseline，拖慢前端迭代）；(c) 在 frontend-contract job 里也挂一份 lint（词表/脚本要双挂）。选哪个影响 CI 拓扑与迭代速度，属需用户拍板的决策，故本波只记录不改。

**状态**：待用户决策的 CI 拓扑孤儿项。
