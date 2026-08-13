"""域②：对话改库 + 召回全链路含恢复（四阶段）。

种 1 条 citable draft → 人工 verify → 阶段1 客户问命中召回 → 正式 patch 降级
(needs_review) 后召回退出 → 阶段4 管理员再次 verify，召回恢复。

召回命中证据在 agent_decision_reviews.used_knowledge_ids(Vec<ObjectId>),不在 agent_run_logs。
等待用 send_and_wait 轮询 agent_run_logs(webhook 后台 runner,真模型一轮 300s+,固定 sleep 假阴)。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain2.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "②改库召回"
WXID = "biztest_c2"
SRC = "biztest_recall_chunk"
QUESTION = "你们退费政策是怎样的？"
# 阶段3/4 换措辞:同一句连发会触发真模型"已答过→先核对口径"的合理行为(不再走召回),
# 让"召回恢复"无从验证。语义同(都问退费规则)但措辞不同→每轮当新问题重新检索。
QUESTION_Q3 = "退款的话有什么具体要求吗？"
QUESTION_Q4 = "我想确认下退费的条件和流程，能再说下吗？"


def _recall_hit(cid: str, run: dict) -> tuple[bool, str]:
    """**本轮** decision_review(按 run_id 精确定位)的 used_knowledge_ids 是否含 cid。

    不用 latest_decision_review：真模型一轮慢(300s+)+三阶段连发,查询时最新 review 常是
    后续 no_reply 轮(used_knowledge_ids 必空)→假阴。且每轮重跑 deleteMany+insertOne 生成
    新 ObjectId,latest 取到的历史轮 review 持有的是旧 cid→跨轮错位。按本轮 run_id 取本轮
    review 根治这两个陷阱(systematic-debugging 2026-06-30 定性:召回链健全,测试取证方式错)。
    """
    rid = run.get("run_id", "") if isinstance(run, dict) else ""
    if not rid:
        return False, f"run 无 run_id, run={str(run)[:200]}"
    dr = _lib.decision_review_for_run(WXID, rid)
    used = dr.get("used_knowledge_ids", [])
    return (cid in str(used)), f"run_id={rid} status={dr.get('status')} used_knowledge_ids={used}"


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 退费咨询客户")
    _lib.reset_contact_conversation(account_id, WXID)
    cid = _lib.seed_citable_knowledge_chunk(
        SRC,
        account_id,
        "biztest 退费政策",
        "7 天内无理由退费，需保留发票原件。",
    )
    _lib.expect(bool(cid), DOMAIN, "citable draft 落库拿到 _id", f"cid={cid}", "critical")
    if not cid:
        return
    verified = _lib.verify_knowledge_chunk(cid)
    _lib.expect(verified.get("ok") is True, DOMAIN, "人工 verify 使知识进入生产 catalog",
                f"verify={verified}", "critical", "fixture 未走正式人工审定路径")
    if verified.get("ok") is not True:
        return

    # ── 阶段1：改前召回命中 ──
    print(f"[{DOMAIN}] 阶段1 改前召回（真模型，轮询等一轮完成）...")
    t0 = time.time()
    run1 = _lib.send_and_wait(app_id, WXID, QUESTION, "m2a", max_wait=600)
    print(f"  耗时 {time.time()-t0:.1f}s run1={run1}")
    _lib.expect(run1 is not None, DOMAIN, "阶段1 webhook 一轮处理完成(run log 落库)",
                f"run1={run1}", "critical", "超时未落 run log→端点挂或 runner 死,排查")
    if run1 is None:
        return
    _lib.assert_llm_success_for_run(
        str(run1.get("run_id", "")), "user.reply.fast.task", DOMAIN
    )
    hit1, ev1 = _recall_hit(cid, run1)
    _lib.expect(hit1, DOMAIN, "阶段1 改前召回命中(used_knowledge_ids 含种子 chunk)",
                f"cid={cid} {ev1}", "high",
                "verified chunk 应被召回；未命中可能是检索阈值或 stage 不匹配")

    # ── 阶段2+3：正式 patch 自动降级 → 召回退出（红线预期） ──
    print(f"[{DOMAIN}] 阶段3 改库降级后召回...")
    patched = _lib.patch_knowledge_chunk(
        cid, {"summary": "退费政策待重新审核：7 天内无理由退费，需保留发票原件。"}
    )
    item3 = _lib.get_knowledge_chunk(cid)
    degraded = (patched.get("ok") is True and item3.get("status") == "draft"
                and item3.get("integrityStatus") == "needs_review")
    _lib.expect(degraded, DOMAIN, "内容 patch 自动降级为 draft + needs_review",
                f"patch={patched} item={item3}", "critical",
                "内容编辑未使既有人工审定失效")
    run3 = _lib.send_and_wait(app_id, WXID, QUESTION_Q3, "m2b", max_wait=600)
    _lib.expect(run3 is not None, DOMAIN, "阶段3 webhook 一轮处理完成", f"run3={run3}", "high")
    miss3, ev3 = _recall_hit(cid, run3 or {})
    _lib.expect(not miss3, DOMAIN, "阶段3 改后召回退出(needs_review 切片不进 verified 召回池,红线预期非bug)",
                f"cid={cid} {ev3}", "high",
                "needs_review 切片仍被召回=verified 召回池过滤失效")

    # ── 阶段4：管理员 verify → 召回恢复 ──
    print(f"[{DOMAIN}] 阶段4 verify 后召回恢复...")
    reverified = _lib.verify_knowledge_chunk(cid)
    _lib.expect(reverified.get("ok") is True, DOMAIN, "管理员重新 verify 成功",
                f"verify={reverified}", "critical")
    run4 = _lib.send_and_wait(app_id, WXID, QUESTION_Q4, "m2c", max_wait=600)
    _lib.expect(run4 is not None, DOMAIN, "阶段4 webhook 一轮处理完成", f"run4={run4}", "high")
    hit4, ev4 = _recall_hit(cid, run4 or {})
    _lib.expect(hit4, DOMAIN, "阶段4 verify 后召回恢复",
                f"cid={cid} {ev4}", "critical",
                "verify 回 verified 后仍召不回=恢复链断(真bug)")

    print(f"[{DOMAIN}] 四阶段完成")


if __name__ == "__main__":
    main()
