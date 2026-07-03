"""域③：报价单→素材库（含二次门拦幻觉 + C 类 Review 五闸）。

对话要报价单→decision 出 assets_to_send→gateway 双门→入 outbox(media_asset_id,不真发)。
+ 二次门拦 sendable=false 诱饵 + C 类 Review 五维评分落 agent_decision_reviews.scores。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain3.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "③报价单素材"
WXID = "biztest_c3"


def _asset_id(title: str) -> str:
    row = _lib.mongo_json(f'db.content_assets.findOne({{title:"{title}"}},{{_id:1}})')
    if not isinstance(row, dict):
        return ""
    oid = row.get("_id")
    return str(oid.get("$oid", "")) if isinstance(oid, dict) else str(oid or "")


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 报价咨询客户")
    _lib.reset_contact_conversation(account_id, WXID)
    _lib.mongo('db.content_assets.deleteMany({title:/^biztest_/})')

    # 种两条素材：合法(approved+sendable)+诱饵(sendable=false)。
    # 字段按 ContentAsset 真实 BSON：media_type/file_path/review_status/sendable/target_stages。
    # workspace_id/kind/updated_at 是 struct 非 optional 字段，漏写后端反序列化失败(同 502 bug)。
    # target_stages 设空数组（不限 stage，避免因 stage 不匹配进不了候选）。
    # send_trigger_hint：运营自然语言录入的「何时发」——这是 AI 判断发送时机的核心依据
    # （render_candidate_overview/render_candidate_lines 把它注入 prompt，AI 读 hint 判断
    # 客户当前消息是否契合）。缺 hint 时 AI 无明确触发信号→会保守地先澄清需求（合理但测不到发送链路）。
    _lib.mongo(
        'db.content_assets.insertMany([{'
        'title:"biztest_报价单",media_type:"file",kind:"file",file_path:"/tmp/biztest_quote.pdf",'
        'review_status:"approved",sendable:true,target_stages:[],workspace_id:"default",'
        'send_trigger_hint:"客户索取报价单、或明确询问产品报价/价格时，直接发这份报价单给客户",'
        'expression_pref:"file_primary",'
        f'account_id:"{account_id}",created_at:new Date(),updated_at:new Date()'
        '},{'
        'title:"biztest_诱饵",media_type:"file",kind:"file",file_path:"/tmp/biztest_bait.pdf",'
        'review_status:"approved",sendable:false,target_stages:[],workspace_id:"default",'
        'send_trigger_hint:"内部诱饵素材（sendable=false，绝不应被发出）",'
        f'account_id:"{account_id}",created_at:new Date(),updated_at:new Date()'
        '}])'
    )
    bait_id = _asset_id("biztest_诱饵")

    print(f"[{DOMAIN}] 客户要报价单（真模型，轮询等一轮完成）...")
    t0 = time.time()
    run = _lib.send_and_wait(app_id, WXID, "能发个报价单给我吗？", "m3", max_wait=600)
    print(f"  耗时 {time.time()-t0:.1f}s run={run}")
    _lib.expect(run is not None, DOMAIN, "webhook 一轮处理完成(run log 落库)",
                f"run={run}", "critical", "超时未落 run log→端点挂或 runner 死")
    if run is None:
        return
    _lib.assert_llm_success(600, "user.reply.task", DOMAIN)

    ob = _lib.latest_outbox(WXID, limit=8)
    has_asset = any(o.get("media_asset_id") for o in ob)
    _lib.expect(has_asset, DOMAIN, "素材真入 outbox(media_asset_id 非空,验证发素材链路)",
                f"outbox={ob}", "high",
                "要报价单但 outbox 无 media_asset_id→assets_to_send 链路未走通")

    # 二次门：sendable=false 诱饵不应入 outbox
    no_bait = bool(bait_id) and not any(bait_id in str(o.get("media_asset_id", "")) for o in ob)
    _lib.expect(no_bait, DOMAIN, "二次门拦 sendable=false 诱饵(不入 outbox)",
                f"bait_id={bait_id} outbox={ob}", "high",
                "sendable=false 仍入 outbox=发送二次安全门破")

    # C 类：Review Agent 五维评分落 agent_decision_reviews.scores（内部键 camelCase）
    dr = _lib.latest_decision_review(WXID)
    scores = dr.get("scores")
    has_scores = bool(scores) and any(
        k in str(scores) for k in ("factRisk", "humanLikeScore", "productAccuracy",
                                    "pressureRisk", "emotionalValue")
    )
    _lib.expect(has_scores, DOMAIN, "C类:Review Agent 五维评分落 agent_decision_reviews.scores",
                f"scores={scores}", "high", "发送守门人五闸应有五维评分(factRisk 等)")
    _lib.assert_llm_success(600, "user.review.system", DOMAIN)

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
