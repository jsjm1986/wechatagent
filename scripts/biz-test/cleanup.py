"""清所有 biztest_ 前缀测试数据。幂等。绝不碰非 biztest_ 数据。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/cleanup.py
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib


def main() -> None:
    parts = []
    # 按 biztest_ 前缀删（各集合的 wxid/contactWxid/fromWxid 字段）。
    for c in [
        "contacts",
        "conversation_messages",
        "agent_run_logs",
        "agent_send_outbox",
        "agent_events",
        "agent_principal_escalations",
        "operating_memories",
    ]:
        parts.append(
            f"r.{c}=db.{c}.deleteMany({{$or:["
            f"{{contactWxid:/^biztest_/}},{{fromWxid:/^biztest_/}},{{wxid:/^biztest_/}}]}}).deletedCount"
        )
    # 知识/素材/卡片/profile 按 biztest_ 命名删。
    parts.append("r.chunks=db.operation_knowledge_chunks.deleteMany({sourceName:/biztest/}).deletedCount")
    parts.append("r.assets=db.content_assets.deleteMany({title:/^biztest_/}).deletedCount")
    parts.append("r.cards=db.referral_cards.deleteMany({displayName:/^biztest_/}).deletedCount")
    parts.append("r.profiles=db.domain_profiles.deleteMany({profileId:/^biztest_/}).deletedCount")
    js = "var r={}; " + "; ".join(parts) + "; printjson(r)"
    print(_lib.mongo(js))


if __name__ == "__main__":
    main()
