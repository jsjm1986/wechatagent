"""域⑨：长期记忆固化（真业务结构，非字符串包含浮测）。

记忆系统真实结构（src/agent/memory.rs + models.rs MemoryCardTyped/MemoryFact）：
- operating_memories.memory_card 是结构化 MemoryCardTyped:
  · coreFacts（长期核心事实，≤6，按 importance 倒序）——结构化 MemoryFact{id,text,confidence,importance,deprecatedAt,deprecationReason...}
  · recentFacts（近期事实，≤10，按 recency）
  · deprecatedFacts（弃用归档，≤20）——被推翻的旧事实带 deprecationReason/supersededBy
  · coreProfile{identity,businessContext,communicationStyle,operationGoal}
  · relationshipState/preferences/doNotDo/commitments/objections/openLoops/recentEpisodeSummary
- 候选→固化链路：decision 抽取 memory_candidates(status=pending) → consolidate 后 status=consolidated → 进 memory_card
- **冲突裁决的真实设计（关键）**：deprecatedFacts/discarded 针对的是**已固化在上一版 coreFacts**
  被推翻（apply_consolidator_deprecations 在 previous.core_facts 按 id 查原 fact，memory.rs:548）。
  故必须**两次固化**才能测真弃用：第一次让旧事实(8岁)进 coreFacts 得 version N，第二次改口(10岁)
  + 再固化 → 旧事实应进 deprecatedFacts(带 deprecationReason) 或被 discarded 替换，落
  memory_conflict_resolved 事件。同一轮内改口不会产生 deprecation（无"上一版 fact"可弃用）。

prompt_key=user.memory_consolidator.task。触发端点 /contacts/:id/memory-consolidation/run(:id=ObjectId)。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain9.py
"""
import json
import re
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑨记忆固化"
WXID = "biztest_c9"


def _require(condition: bool, description: str, evidence: object) -> None:
    if not _lib.expect(condition, DOMAIN, description, str(evidence), "critical"):
        raise SystemExit(f"{description}: {evidence}")


def _contact_oid(account_id: str) -> str:
    row = _lib.mongo_json(
        f'db.contacts.findOne({{wxid:"{WXID}",account_id:"{account_id}"}},{{_id:1}})'
    )
    if not isinstance(row, dict):
        return ""
    oid = row.get("_id")
    return str(oid.get("$oid", "")) if isinstance(oid, dict) else str(oid or "")


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


def _consolidate(account_id: str, tag: str) -> dict:
    contact_id = _contact_oid(account_id)
    if not contact_id:
        return {"_error": "no contact id"}
    return _lib.api_bg(
        "POST", f"/api/contacts/{contact_id}/memory-consolidation/run",
        {}, admin=True, max_wait=300, tag=tag,
    )


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
        "你好，我想给孩子报编程课",
        "我孩子今年8岁，零基础",
        "预算大概5000左右",
    ]
    for i, t in enumerate(turns_a):
        print(f"[{DOMAIN}] A阶段对话轮 {i+1}/{len(turns_a)}（真模型轮询）...")
        r = _lib.send_and_wait(app_id, WXID, t, f"m9a_{i}", max_wait=600)
        _require(isinstance(r, dict) and bool(r.get("run_id")), f"A阶段对话轮{i+1} webhook 完成", r)

    # 候选记忆链路：decision 阶段应抽取出候选（status pending/consolidated）
    cands = _lib.mongo_json(
        f'db.memory_candidates.find({{contact_wxid:"{WXID}"}},{{source:1,status:1,_id:0}}).toArray()'
    )
    has_cand = isinstance(cands, list) and len(cands) > 0
    _require(has_cand, "decision 阶段抽取出 memory_candidates", cands)

    print(f"[{DOMAIN}] 第一次固化（让8岁进 coreFacts）...")
    res_a = _consolidate(account_id, "memcon_a")
    print(f"  res_a={str(res_a)[:200]}")
    _require(_lib.is_api_error(res_a) is None and bool(res_a.get("taskId")),
             "第一次固化返回 durable taskId", res_a)
    task_a_id = res_a["taskId"]
    task_a = _lib.memory_task_evidence(task_a_id)
    _require(task_a.get("status") == "sent" and task_a.get("gateway_status") == "consolidated",
             "第一次固化 task 到达 consolidated 终态", task_a)
    _lib.assert_llm_success(400, "user.memory_consolidator.task", DOMAIN)

    card_a = _memory_card(account_id)
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
    rb = _lib.send_and_wait(app_id, WXID, "哦我说错了，孩子其实10岁了，不是8岁", "m9b", max_wait=600)
    _require(isinstance(rb, dict) and bool(rb.get("run_id")), "B阶段改口轮 webhook 完成", rb)

    print(f"[{DOMAIN}] 第二次固化（应触发对8岁的弃用/冲突裁决）...")
    res_b = _consolidate(account_id, "memcon_b")
    print(f"  res_b={str(res_b)[:200]}")
    _require(_lib.is_api_error(res_b) is None and bool(res_b.get("taskId")),
             "第二次固化返回 durable taskId", res_b)
    task_b_id = res_b["taskId"]
    task_b = _lib.memory_task_evidence(task_b_id)
    generation = task_b.get("claim_generation")
    _require(task_b.get("status") == "sent" and task_b.get("gateway_status") == "consolidated"
             and isinstance(generation, int), "第二次固化 task 到达精确 consolidated 终态", task_b)

    card_b = _memory_card(account_id)
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
    # 生效事实层不应再有"8岁"（10岁已是 winner）。理想：旧值进 deprecatedFacts。
    _require(not age8_live, "冲突裁决后8岁不再生效", {"live": live_b, "ages": sorted(live_ages)})
    _require(age8_deprecated, "旧8岁事实进入 deprecatedFacts 权威归档", deprecated_b)

    commit_events = _lib.memory_commit_events(task_b_id, generation)
    completed = [event for event in commit_events if event.get("kind") == "memory_consolidated"]
    conflicts = [event for event in commit_events if event.get("kind") == "memory_conflict_resolved"]
    _require(len(completed) == 1, "第二次 task 恰有一条完成审计", commit_events)
    complete_details = completed[0].get("details") or {}
    run_id = complete_details.get("runId")
    _require(bool(run_id) and complete_details.get("memoryCardVersion") == ver_b,
             "完成审计绑定固化 run/version", complete_details)
    matching_conflicts = [event for event in conflicts if
        (event.get("details") or {}).get("runId") == run_id
        and (event.get("details") or {}).get("previousVersion") == ver_a
        and (event.get("details") or {}).get("memoryCardVersion") == ver_b
        and (event.get("details") or {}).get("auditSource") in {"model_conflict", "memory_card_diff"}]
    _require(bool(matching_conflicts),
             "冲突事件绑定第二次 task 的 run、前后版本和审计来源", commit_events)

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
