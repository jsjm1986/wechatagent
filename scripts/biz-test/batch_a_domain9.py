"""域⑨：长期记忆固化（真业务结构，非字符串包含浮测）。

记忆系统真实结构（src/agent/memory.rs + models.rs MemoryCardTyped/MemoryFact）：
- operating_memories.memory_card 是结构化 MemoryCardTyped:
  · coreFacts（长期核心事实，≤6，按 importance 倒序）——结构化 MemoryFact{id,text,confidence,importance,deprecatedAt,deprecationReason...}
  · recentFacts（近期事实，≤10，按 recency）
  · deprecatedFacts（弃用归档，≤20）——被推翻的旧事实带 deprecationReason/supersededBy
  · coreProfile{identity,businessContext,communicationStyle,operationGoal}
  · relationshipState/preferences/doNotDo/commitments/objections/openLoops/recentEpisodeSummary
- 候选→固化链路：发送后 Projection 抽取 memory_candidates(status=pending) → 自动 durable
  memory_consolidation task → status=consolidated → 进 memory_card
- **冲突裁决的真实设计（关键）**：deprecatedFacts/discarded 针对的是**已固化在上一版 coreFacts**
  被推翻（apply_consolidator_deprecations 在 previous.core_facts 按 id 查原 fact，memory.rs:548）。
  故必须**两次固化**才能测真弃用：第一次让旧事实(8岁)进 coreFacts 得 version N，第二次改口(10岁)
  + 再固化 → 旧事实应进 deprecatedFacts(带 deprecationReason) 或被 discarded 替换，落
  memory_conflict_resolved 事件。同一轮内改口不会产生 deprecation（无"上一版 fact"可弃用）。

prompt_key=user.memory_consolidator.task。测试跟随生产自动固化任务，不与自动任务争抢手动触发唯一键。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain9.py
"""
import json
import re
import sys
import time
from pathlib import Path
from typing import Optional

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑨记忆固化"
WXID = "biztest_c9"


def _require(condition: bool, description: str, evidence: object) -> None:
    if not _lib.expect(condition, DOMAIN, description, str(evidence), "critical"):
        raise SystemExit(f"{description}: {evidence}")


def _memory_card(account_id: str) -> dict:
    """取测试账号下唯一结构化 memory_card。"""
    mc = _lib.mongo_json(
        "db.operating_memories.findOne("
        f"{{contact_wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}},"
        "{memory_card:1,memory_card_version:1,memory_source_task_id:1,"
        "memory_source_task_claim_generation:1,_id:0})"
    )
    return mc if isinstance(mc, dict) else {}


def _fact_texts(card: dict, key: str) -> list[str]:
    """从 memory_card 某层（coreFacts/recentFacts/deprecatedFacts）取出所有 fact 文本。

    MemoryFactRepr 既可能是结构化对象 {text,...} 也可能是纯字符串（历史形态），都兼容。
    """
    facts = card.get(key, []) if isinstance(card, dict) else []
    out = []
    if isinstance(facts, list):
        for f in facts:
            if isinstance(f, dict):
                out.append(str(f.get("text", "")))
            else:
                out.append(str(f))
    return out


def _ages_in(texts: list[str]) -> set[str]:
    """从一批 fact 文本里抽出所有「年龄实体」(N岁/N 岁) 的数值集合。

    实体级抽取替代子串包含：旧断言 `"8" in t` / `"10" not in t` 会被无关数字
    （"预算5800"/"10课时"）误伤，且 `and "10" not in t` 把同时含"8岁"+"10"的混写
    fact 整条豁免 → 8岁实际仍生效却假绿 PASS（2026-06-27 域⑨实证假绿真因）。
    这里只认 `\\d+\\s*岁` 这一年龄实体，逐条比对数值，不碰其它数字。
    """
    ages: set[str] = set()
    for t in texts:
        for m in re.findall(r"(\d+)\s*岁", t):
            ages.add(m)
    return ages


def _wait_memory_card(account_id: str, *, after_version: int, required_age: str,
                      max_wait: int = 300) -> dict:
    """Wait until automatic consolidation advances and materializes one age fact."""
    deadline = time.time() + max_wait
    latest = {}
    while time.time() < deadline:
        latest = _memory_card(account_id)
        card = latest.get("memory_card", {}) if isinstance(latest, dict) else {}
        live = _fact_texts(card, "coreFacts") + _fact_texts(card, "recentFacts")
        if latest.get("memory_card_version", 0) > after_version and required_age in _ages_in(live):
            return latest
        time.sleep(5)
    return latest


def _await_projection(run: dict, label: str, account_id: str) -> str:
    """Wait for one exact projection, returning empty on a model JSON-contract miss."""
    run_id = str(run.get("run_id", ""))
    review = _lib.wait_projection_terminal(WXID, run_id, max_wait=300)
    logs = _lib.projection_llm_logs(run_id)
    if _lib.projection_model_contract_failure(review):
        _require(
            any(row.get("status") in {"success", "cache_hit"} for row in logs),
            f"{label} invalid_projection 仍有精确真模型审计",
            logs,
        )
        print(
            f"[{DOMAIN}] {label} 模型投影 JSON 形态不合约，"
            f"run_id={run_id}；将以新消息做有界重试..."
        )
        return ""
    _require(
        review.get("post_decision_status") == "completed"
        and review.get("post_decision_memory_done") is True,
        f"{label} post-decision Projection 完成记忆投影",
        review,
    )
    _require(
        any(row.get("status") in {"success", "cache_hit"} for row in logs),
        f"{label} 精确 user.projection.task 真调成功",
        logs,
    )
    active = _lib.active_memory_consolidation_task(WXID, account_id)
    task_id = _lib.bson_object_id(active.get("_id")) if active else ""
    if task_id:
        task = _lib.wait_memory_task_terminal(task_id, max_wait=300)
        _require(
            task.get("status") == "sent"
            and task.get("gateway_status") in {"consolidated", "no_candidates"},
            f"{label} 自动记忆固化任务到达终态",
            task,
        )
    return run_id


def _send_projected_turn(app_id: str, content: str, tag: str, label: str,
                         account_id: str, *, required_age: Optional[str] = None,
                         max_attempts: int = 3) -> str:
    """Inject one semantic turn until its real-model projection satisfies the contract."""
    latest: dict = {}
    latest_candidates: list[dict] = []
    for attempt in range(max_attempts):
        latest = _lib.send_and_wait(
            app_id,
            WXID,
            content,
            f"{tag}_projection_{attempt + 1}",
            max_wait=600,
        ) or {}
        _require(
            bool(latest.get("run_id")),
            f"{label} webhook 完成（投影尝试 {attempt + 1}/{max_attempts}）",
            latest,
        )
        run_id = _await_projection(latest, label, account_id)
        if not run_id:
            continue
        if required_age is not None:
            latest_candidates = _lib.memory_candidates_for_runs(WXID, [run_id])
            eligible = [
                row for row in latest_candidates
                if row.get("status") in {"pending", "consolidated"}
            ]
            candidate_ages = _ages_in(_lib.memory_candidate_texts(eligible))
            if required_age not in candidate_ages:
                print(
                    f"[{DOMAIN}] {label} 合法投影未生成 {required_age} 岁可固化候选；"
                    f"run_id={run_id}，做有界新 run 重试..."
                )
                continue
        return run_id
    _require(
        False,
        f"{label} 在有界重试内产生合约有效且满足事实要求的 Projection",
        {
            "run": latest,
            "required_age": required_age,
            "candidates": latest_candidates,
        },
    )
    return ""


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 记忆固化客户")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo(
        f'db.operating_memories.deleteMany({{contact_wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}});'
        f'db.memory_candidates.deleteMany({{contact_wxid:{json.dumps(WXID)},account_id:{json.dumps(account_id)}}})'
    )

    # ── 第一阶段：建立基线事实（孩子8岁）并首次固化 → 8岁进 coreFacts ──
    turns_a = [
        "以后跟我沟通请尽量用简短要点，我不喜欢大段文字。",
        "请记住：我孩子今年8岁，目前零基础。",
    ]
    run_ids_a = []
    for i, t in enumerate(turns_a):
        print(f"[{DOMAIN}] A阶段对话轮 {i+1}/{len(turns_a)}（真模型轮询）...")
        run_ids_a.append(
            _send_projected_turn(
                app_id,
                t,
                f"m9a_{i}",
                f"A阶段对话轮{i+1}",
                account_id,
                required_age="8" if i == 1 else None,
            )
        )

    # 候选记忆由发送后的独立 Projection 产生；按精确 parent run 取证，不能在主决策
    # 送达后立即查 contact 级最新行（Projection 尚未完成时会产生竞态假阴性）。
    cands = _lib.memory_candidates_for_runs(WXID, run_ids_a)
    has_cand = isinstance(cands, list) and len(cands) > 0
    _require(has_cand, "Projection 阶段抽取出精确 run 的 memory_candidates", cands)

    print(f"[{DOMAIN}] 等待自动固化让8岁进入事实层...")
    card_a = _wait_memory_card(account_id, after_version=0, required_age="8")
    _require(
        card_a.get("memory_card_version", 0) > 0,
        "A阶段事实触发首个 memory_card 版本",
        card_a,
    )
    task_a_id = _lib.bson_object_id(card_a.get("memory_source_task_id"))
    task_a = _lib.memory_task_evidence(task_a_id)
    _require(task_a.get("status") == "sent" and task_a.get("gateway_status") == "consolidated",
             "A阶段自动固化 task 到达 consolidated 终态", task_a)
    generation_a = task_a.get("claim_generation")
    commit_events_a = _lib.memory_commit_events(task_a_id, generation_a)
    completed_a = [event for event in commit_events_a if event.get("kind") == "memory_consolidated"]
    _require(len(completed_a) == 1, "A阶段自动固化恰有一条完成审计", commit_events_a)
    consolidator_run_a = (completed_a[0].get("details") or {}).get("runId")
    _lib.assert_llm_success_for_run(
        str(consolidator_run_a or ""), "user.memory_consolidator.task", DOMAIN,
    )

    ver_a = card_a.get("memory_card_version", 0)
    core_a = _fact_texts(card_a.get("memory_card", {}), "coreFacts")
    recent_a = _fact_texts(card_a.get("memory_card", {}), "recentFacts")
    all_a = core_a + recent_a
    # 8岁应已固化进事实层（core 或 recent）——实体级断言（避免 "预算5800" 等无关数字误判）
    age8 = "8" in _ages_in(all_a)
    _lib.expect(age8, DOMAIN, "A阶段:8岁事实固化进 coreFacts/recentFacts(结构化事实层)",
                f"core={core_a} recent={recent_a}", "high",
                "8岁未进结构化事实层→事实抽取/固化链路未产出")
    # 候选应被标 consolidated（已消化）
    cands_after = _lib.mongo_json(
        f'db.memory_candidates.find({{contact_wxid:"{WXID}",status:"consolidated"}},{{_id:1}}).toArray()'
    )
    consolidated_ok = isinstance(cands_after, list) and len(cands_after) > 0
    _lib.expect(consolidated_ok, DOMAIN, "固化后候选记忆状态 consolidated(候选→固化闭环)",
                f"consolidated={cands_after}", "medium",
                "固化后候选仍 pending→候选消化链路断")

    # ── 第二阶段：改口10岁（推翻已固化的8岁）+ 再固化 → 测真冲突裁决/弃用 ──
    print(f"[{DOMAIN}] B阶段改口10岁（推翻已固化8岁）...")
    _send_projected_turn(
        app_id,
        "我刚核对过信息，认真更正：孩子今年10岁，之前说8岁是我记错了。"
        "这不是玩笑，请按10岁更新长期记录。",
        "m9b",
        "B阶段改口轮",
        account_id,
        required_age="10",
    )

    print(f"[{DOMAIN}] 等待自动固化触发对8岁的弃用/冲突裁决...")
    card_b = _wait_memory_card(account_id, after_version=ver_a, required_age="10")
    _require(
        card_b.get("memory_card_version", 0) > ver_a,
        "B阶段更正触发新 memory_card 版本",
        card_b,
    )
    task_b_id = _lib.bson_object_id(card_b.get("memory_source_task_id"))
    task_b = _lib.memory_task_evidence(task_b_id)
    generation = task_b.get("claim_generation")
    _require(task_b.get("status") == "sent" and task_b.get("gateway_status") == "consolidated"
             and isinstance(generation, int), "B阶段自动固化 task 到达精确 consolidated 终态", task_b)

    ver_b = card_b.get("memory_card_version", 0)
    mcb = card_b.get("memory_card", {})
    core_b = _fact_texts(mcb, "coreFacts")
    recent_b = _fact_texts(mcb, "recentFacts")
    deprecated_b = _fact_texts(mcb, "deprecatedFacts")
    live_b = core_b + recent_b

    # 版本应推进（两次固化）
    _require(ver_b > ver_a, "memory_card_version 随第二次固化推进", {"before": ver_a, "after": ver_b})
    _require(_lib.bson_object_id(card_b.get("memory_source_task_id")) == task_b_id,
             "memory row 绑定第二次固化 task", card_b)

    # 实体级抽取：把生效层/弃用层各自的「年龄实体」数值集合算出来，逐值比对（不靠子串）
    live_ages = _ages_in(live_b)
    deprecated_ages = _ages_in(deprecated_b)

    # 核心：10岁成为生效事实
    age10_live = "10" in live_ages
    _require(age10_live, "B阶段改口后10岁成为生效事实", {"live": live_b, "ages": sorted(live_ages)})

    # 冲突裁决（真业务逻辑）：8岁应不再是生效事实——要么进 deprecatedFacts，要么被替换移除。
    # 弃用归档优先（带 deprecationReason 的结构化归档是设计意图）。
    # 实体级判定：生效层的年龄实体集合不应再含 "8"（旧断言 `"8岁" in t and "10" not in t`
    # 被混写 fact 绕过 → 假绿，2026-06-27 实证）。
    age8_live = "8" in live_ages
    age8_deprecated = "8" in deprecated_ages
    # 生效事实层不应再有"8岁"（10岁已是 winner）。
    _require(not age8_live, "冲突裁决后8岁不再生效", {"live": live_b, "ages": sorted(live_ages)})

    commit_events = _lib.memory_commit_events(task_b_id, generation)
    completed = [event for event in commit_events if event.get("kind") == "memory_consolidated"]
    conflicts = [event for event in commit_events if event.get("kind") == "memory_conflict_resolved"]
    _require(len(completed) == 1, "第二次 task 恰有一条完成审计", commit_events)
    complete_details = completed[0].get("details") or {}
    run_id = complete_details.get("runId")
    _require(bool(run_id) and complete_details.get("memoryCardVersion") == ver_b,
             "完成审计绑定固化 run/version", complete_details)
    discarded_ages = _ages_in(_lib.memory_discarded_texts(completed))
    age8_audited_discard = "8" in discarded_ages
    _require(
        age8_deprecated or age8_audited_discard,
        "旧8岁事实进入 deprecatedFacts，或由同 task 完成审计明确 discarded",
        {
            "deprecated": deprecated_b,
            "completion": complete_details,
        },
    )
    matching_conflicts = [event for event in conflicts if
        (event.get("details") or {}).get("runId") == run_id
        and (event.get("details") or {}).get("previousVersion") == ver_a
        and (event.get("details") or {}).get("memoryCardVersion") == ver_b
        and (event.get("details") or {}).get("auditSource") in {"model_conflict", "memory_card_diff"}]
    _require(
        len(matching_conflicts) == len(conflicts) and len(matching_conflicts) <= 1,
        "冲突事件若存在则绑定第二次 task 的 run、前后版本且不重复",
        commit_events,
    )
    if age8_deprecated:
        _require(
            len(matching_conflicts) == 1,
            "deprecatedFacts 分支恰有一条冲突裁决审计",
            commit_events,
        )

    # coreProfile 应有实质画像（identity/businessContext 非空）——记忆不只是 fact 列表
    profile = mcb.get("coreProfile", {}) if isinstance(mcb, dict) else {}
    profile_filled = isinstance(profile, dict) and bool(
        str(profile.get("identity", "")).strip() or str(profile.get("businessContext", "")).strip()
    )
    _lib.expect(profile_filled, DOMAIN, "coreProfile 实质画像填充(identity/businessContext)",
                f"coreProfile={profile}", "medium",
                "固化后 coreProfile 空→画像层未产出(记忆退化成纯 fact 列表)")

    print(f"[{DOMAIN}] 完成（真业务结构验证：三层事实/冲突裁决/候选闭环/画像层）")


if __name__ == "__main__":
    main()
