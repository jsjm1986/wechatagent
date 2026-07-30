"""清所有 biztest_ 前缀测试数据。幂等。绝不碰非 biztest_ 数据。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/cleanup.py
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib


def build_cleanup_script() -> str:
    parts = []
    # 按 biztest_ 前缀删（各集合的 wxid/contact_wxid/from_wxid 字段）。
    # 注意：mongo 直接查询用 snake_case BSON 字段名（API 响应才是 camelCase serde rename）。
    for c in [
        "contacts",
        "conversation_messages",
        "agent_run_logs",
        "agent_decision_reviews",
        "agent_send_outbox",
        "agent_events",
        "agent_principal_escalations",
        "operating_memories",
    ]:
        parts.append(
            f"r.{c}=db.{c}.deleteMany({{$or:["
            f"{{contact_wxid:/^biztest_/}},{{from_wxid:/^biztest_/}},{{wxid:/^biztest_/}}]}}).deletedCount"
        )
    # 新导入链把 source_name 存在 document，chunk 仅保留 ObjectId document_id；
    # 旧业务脚本仍会直接写带 source_name 的 legacy chunk，两种形态都必须清理。
    parts.append(
        "var _docids=db.operation_knowledge_documents.find({source_name:/^biztest_/},{_id:1})"
        ".toArray().map(d=>d._id); "
        "r.kchunks_by_document=db.operation_knowledge_chunks"
        ".deleteMany({document_id:{$in:_docids}}).deletedCount; "
        "r.kchunks_legacy=db.operation_knowledge_chunks"
        ".deleteMany({source_name:/^biztest_/}).deletedCount; "
        "r.kdocs=db.operation_knowledge_documents"
        ".deleteMany({source_name:/^biztest_/}).deletedCount"
    )
    parts.append("r.assets=db.content_assets.deleteMany({title:/^biztest_/}).deletedCount")
    parts.append("r.cards=db.referral_cards.deleteMany({display_name:/^biztest_/}).deletedCount")
    parts.append("r.profiles=db.domain_profiles.deleteMany({profile_id:/^biztest_/}).deletedCount")
    return "var r={}; " + "; ".join(parts) + "; printjson(r)"


def main() -> None:
    print(_lib.mongo(build_cleanup_script()))


if __name__ == "__main__":
    main()
