"""阶段2 评测域：验 PR#73 judge_user_operation_scenario 尺度错配修复在真实环境生效。

打 /api/user-operations/evaluations/run（内部对 4 内置场景跑真 simulate → judge）。
修复前 judge 三处拦截全失效：grounding<60 死规则让"质量正常"场景恒误判 failed、
finalReviewStatus 读不存在字段是死门。修复后改读 turn.status 同源。

本脚本铁证：
- summary.passed > 0（修复前因 grounding<60 死规则几乎全 failed；修复后质量场景应 passed）
- 任何 failed 场景的 issues 里**不再出现** 旧死规则字符串 "knowledge_grounding 评分不足（<60）"
  / "hallucination 评分过高（≥50）"（这两条已被删除，出现=部署的还是旧代码）
- evaluation.issues 若有，只能是新同源信号（"Review 闸拦截"/"发送网关拦截"）

跑法：export DEPLOY_PASS=... ADMIN_USER=admin ADMIN_PASS=admin; python scripts/biz-test/batch_c_evaluation.py
依赖：先跑 step0_preflight.py（/tmp/biztest_account + cookie）。
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "评测体系(judge修复)"
WXID = "biztest_eval"

# 修复中被删除的旧死规则字符串——部署正确则评测 issues 里绝不应再出现。
# 注:故意用**前缀子串**(截在全角括号「（<60）」之前),规避"全/半角括号写错一字→
# 永远命中不到→否定式断言恒绿"的假绿陷阱。源码历史串(git show 09dbb3a)含全角括号,
# 前缀 "knowledge_grounding 评分不足"/"hallucination 评分过高" 与之逐字一致,旧规则复活必命中。
OLD_DEAD_RULES = [
    "knowledge_grounding 评分不足",
    "hallucination 评分过高",
]


def main() -> None:
    account_id, _app_id = _lib.biztest_account()

    # 确认 server 跑的是含 judge 修复的基线（commit 09dbb3a 或其后）。
    _, head = _lib.remote_run("cd /opt/wechatagent && git rev-parse --short HEAD")
    print(f"[{DOMAIN}] server HEAD={head.strip()}")

    _lib.ensure_managed_contact(account_id, WXID, "评测客户")
    _lib.reset_contact_conversation(account_id, WXID)

    # 评测端点要 contact_id（_id）。查 wxid 对应 _id。
    rows = _lib.mongo_json(
        f'db.contacts.find({{wxid:"{WXID}",account_id:"{account_id}"}},{{_id:1}}).toArray()'
    )
    if not rows:
        _lib.record(DOMAIN, "contact 未建", f"wxid={WXID}", "critical", "ensure_managed_contact 失败")
        raise SystemExit("contact 未建")
    contact_id = rows[0]["_id"]["$oid"] if isinstance(rows[0]["_id"], dict) else str(rows[0]["_id"])
    print(f"[{DOMAIN}] contact_id={contact_id}")

    # 真跑评测（4 场景 × 多轮 simulate，慢，用 api_bg）。
    print(f"[{DOMAIN}] /evaluations/run 真跑(4 场景真 LLM)...")
    t0 = time.time()
    result = _lib.api_bg(
        "POST", "/api/user-operations/evaluations/run",
        {"accountId": account_id, "contactId": contact_id},
        admin=True, max_wait=900, tag="eval",
    )
    print(f"  耗时 {time.time()-t0:.1f}s")

    if not isinstance(result, dict):
        _lib.record(DOMAIN, "评测端点返回非预期", str(result)[:300], "critical",
                    _lib.is_api_error(result) or "")
        raise SystemExit("评测端点失败")

    summary = result.get("summary", {})
    items = result.get("items", [])
    total = summary.get("total", 0)
    passed = summary.get("passed", 0)
    print(f"[{DOMAIN}] summary: total={total} passed={passed} failed={summary.get('failed')}")
    for it in items:
        ev = it.get("evaluation", {})
        print(f"  - {it.get('scenario')}: passed={it.get('passed')} "
              f"finalReviewStatus={ev.get('finalReviewStatus')} issues={ev.get('issues')}")

    # 铁证 1：真调 LLM（评测内部跑 simulate 的 decide/review）。
    _lib.assert_llm_success(900, "user.reply.task", DOMAIN)

    # 铁证 2：质量正常场景不再被 grounding<60 死规则全误判 → passed > 0。
    _lib.expect(passed > 0, DOMAIN, "修复后至少 1 个场景 passed(旧 grounding<60 死规则会全误判 failed)",
                f"summary={summary}", "critical",
                "若 passed=0 且 issues 含'grounding 评分不足'=部署仍是旧代码或修复回退")

    # 铁证 3：旧死规则字符串绝不再出现（已删除；出现=旧代码）。
    all_issues = []
    for it in items:
        all_issues += (it.get("evaluation", {}).get("issues") or [])
    dead_hit = [s for s in all_issues if any(old in s for old in OLD_DEAD_RULES)]
    _lib.expect(not dead_hit, DOMAIN, "评测 issues 不含已删除的旧尺度错配死规则",
                f"dead_hit={dead_hit} all_issues={all_issues}", "critical",
                "出现'grounding 评分不足/hallucination 评分过高'=server 跑的还是修复前代码")

    # 铁证 4：finalReviewStatus 现在是同源 turn.status 闭集值（修复前恒为空字符串=死门）。
    statuses = [it.get("evaluation", {}).get("finalReviewStatus") for it in items]
    valid = {"would_send", "review_blocked", "gateway_blocked", "no_reply"}
    nonempty_valid = all((s in valid) for s in statuses if s)
    has_nonempty = any(s for s in statuses)
    _lib.expect(has_nonempty and nonempty_valid, DOMAIN,
                "finalReviewStatus 为同源 turn.status 闭集值(修复前恒空=死门)",
                f"statuses={statuses}", "high",
                "全空=仍读不存在的 finalReviewStatus 字段（旧死门未修）")

    print(f"[{DOMAIN}] 完成。passed={passed}/{total}, 旧死规则命中={len(dead_hit)}")


if __name__ == "__main__":
    main()
