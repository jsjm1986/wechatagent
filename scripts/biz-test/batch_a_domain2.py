"""域②：对话改库 + 召回全链路含恢复（四阶段）。

种 1 条 verified chunk → 阶段1 客户问命中召回 → 阶段3 改库降级(needs_review)召回退出
(红线预期,非bug) → 阶段4 管理员 verify 回 verified 召回恢复。

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


def _chunk_id() -> str:
    """取种下的 chunk 的 _id 十六进制（ObjectId.$oid）。"""
    row = _lib.mongo_json(
        f'db.operation_knowledge_chunks.findOne({{source_name:"{SRC}"}},{{_id:1}})'
    )
    if not isinstance(row, dict):
        return ""
    oid = row.get("_id")
    if isinstance(oid, dict):
        return str(oid.get("$oid", ""))
    return str(oid or "")


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
    rows = _lib.mongo_json(
        f'db.agent_decision_reviews.find({{contact_wxid:"{WXID}",run_id:"{rid}"}},'
        '{used_knowledge_ids:1,status:1,_id:0}).sort({_id:-1}).limit(1).toArray()'
    )
    dr = rows[0] if isinstance(rows, list) and rows else {}
    used = dr.get("used_knowledge_ids", [])
    return (cid in str(used)), f"run_id={rid} status={dr.get('status')} used_knowledge_ids={used}"


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 退费咨询客户")
    _lib.reset_contact_conversation(account_id, WXID)
    # 清旧种子（幂等重跑）
    _lib.mongo(f'db.operation_knowledge_chunks.deleteMany({{source_name:"{SRC}"}})')

    # 种 1 条 verified chunk（测试 fixture，字段按 OperationKnowledgeChunk 真实 BSON snake_case）
    # 注意：workspace_id/domain/priority/updated_at 是 struct 非 optional 字段，漏写会让
    # 后端读 chunk 时 BSON 反序列化失败（同 contact updated_at 502 bug 同型）。
    # status 必须 "active"：召回加载点 knowledge_router.rs:70-71 写死 status=="active" AND
    # integrity_status=="verified" 双门，draft 永不进候选集（used_knowledge_ids 必空）。
    # 本 fixture 模拟"已过人工审核、可被运营召回的已审知识"——这正是阶段1"改前召回命中"的前置。
    seed = (
        'db.operation_knowledge_chunks.insertOne({'
        f'source_name:"{SRC}",'
        'workspace_id:"default",domain:"user_operations",priority:0,'
        'title:"biztest 退费政策",'
        'content:"7 天内无理由退费，需保留发票原件。",'
        'source_quote:"7 天内无理由退费，需保留发票原件。",'
        'integrity_status:"verified",status:"active",'
        f'account_id:"{account_id}",created_at:new Date(),updated_at:new Date()'
        '})'
    )
    _lib.mongo(seed)
    cid = _chunk_id()
    _lib.expect(bool(cid), DOMAIN, "种子 chunk 落库拿到 _id", f"cid={cid}", "critical")
    if not cid:
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
    _lib.assert_llm_success(600, "user.reply.task", DOMAIN)
    hit1, ev1 = _recall_hit(cid, run1)
    _lib.expect(hit1, DOMAIN, "阶段1 改前召回命中(used_knowledge_ids 含种子 chunk)",
                f"cid={cid} {ev1}", "high",
                "verified chunk 应被召回；未命中可能是检索阈值或 stage 不匹配")

    # ── 阶段2+3：改库降级 → 召回退出（红线预期） ──
    # 真实路径是经 chat 改库端点触发 AI 改库强制 needs_review；此处直接验降级机制本身
    # （把 chunk 标 needs_review，模拟 AI 改库后果），验其退出 verified 召回池。
    print(f"[{DOMAIN}] 阶段3 改库降级后召回...")
    _lib.mongo(
        f'db.operation_knowledge_chunks.updateOne({{source_name:"{SRC}"}},'
        '{$set:{integrity_status:"needs_review"}})'
    )
    run3 = _lib.send_and_wait(app_id, WXID, QUESTION_Q3, "m2b", max_wait=600)
    _lib.expect(run3 is not None, DOMAIN, "阶段3 webhook 一轮处理完成", f"run3={run3}", "high")
    miss3, ev3 = _recall_hit(cid, run3 or {})
    _lib.expect(not miss3, DOMAIN, "阶段3 改后召回退出(needs_review 切片不进 verified 召回池,红线预期非bug)",
                f"cid={cid} {ev3}", "high",
                "needs_review 切片仍被召回=verified 召回池过滤失效")

    # ── 阶段4：管理员 verify → 召回恢复 ──
    print(f"[{DOMAIN}] 阶段4 verify 后召回恢复...")
    _lib.mongo(
        f'db.operation_knowledge_chunks.updateOne({{source_name:"{SRC}"}},'
        '{$set:{integrity_status:"verified"}})'
    )
    run4 = _lib.send_and_wait(app_id, WXID, QUESTION_Q4, "m2c", max_wait=600)
    _lib.expect(run4 is not None, DOMAIN, "阶段4 webhook 一轮处理完成", f"run4={run4}", "high")
    hit4, ev4 = _recall_hit(cid, run4 or {})
    _lib.expect(hit4, DOMAIN, "阶段4 verify 后召回恢复",
                f"cid={cid} {ev4}", "critical",
                "verify 回 verified 后仍召不回=恢复链断(真bug)")

    print(f"[{DOMAIN}] 四阶段完成")


if __name__ == "__main__":
    main()
