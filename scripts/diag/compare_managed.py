"""Diagnostic: compare all managed contacts and their gateway-relevant state."""
from pymongo import MongoClient
from datetime import datetime, timezone, timedelta

client = MongoClient("mongodb://localhost:27017")
db = client["wechatagent"]

print("=== wechat_accounts ===")
for a in db.wechat_accounts.find():
    print(f"  workspace={a.get('workspace_id')} account_id={a.get('account_id')} app_id={a.get('app_id')} name={a.get('display_name') or a.get('name')}")

print("\n=== ALL contacts where agent_status=managed ===")
managed = list(db.contacts.find({"agent_status": "managed"}))
print(f"  count={len(managed)}")
for c in managed:
    print(f"\n--- {c.get('nickname') or c.get('wxid')} ---")
    print(f"  _id={c.get('_id')}")
    print(f"  workspace={c.get('workspace_id')} account_id={c.get('account_id')}")
    print(f"  wxid={c.get('wxid')}")
    print(f"  agent_status={c.get('agent_status')}")
    print(f"  customer_stage={c.get('customer_stage')}")
    print(f"  cooldown_until={c.get('cooldown_until')}")
    print(f"  last_inbound_at={c.get('last_inbound_at')}")
    print(f"  last_outbound_at={c.get('last_outbound_at')}")
    print(f"  last_agent_run_at={c.get('last_agent_run_at')}")
    print(f"  last_message_at={c.get('last_message_at')}")
    op = c.get('operation_policy') or {}
    if op:
        print(f"  operation_policy={op}")
    print(f"  custom_agent_instructions={c.get('custom_agent_instructions')}")
    print(f"  playbook_id={c.get('playbook_id')}")

print("\n=== inbound messages count per managed contact (last 24h) ===")
since = datetime.now(timezone.utc) - timedelta(hours=24)
for c in managed:
    n_in = db.conversation_messages.count_documents({
        "workspace_id": c.get("workspace_id"),
        "account_id": c.get("account_id"),
        "contact_wxid": c.get("wxid"),
        "direction": "inbound",
        "created_at": {"$gte": since},
    })
    n_out = db.conversation_messages.count_documents({
        "workspace_id": c.get("workspace_id"),
        "account_id": c.get("account_id"),
        "contact_wxid": c.get("wxid"),
        "direction": "outbound",
        "created_at": {"$gte": since},
    })
    print(f"  {c.get('nickname') or c.get('wxid'):30s} (account={c.get('account_id')}) inbound24h={n_in} outbound24h={n_out}")

print("\n=== last 5 agent_run_logs per managed contact (24h) ===")
for c in managed:
    print(f"\n  --- {c.get('nickname') or c.get('wxid')} ---")
    runs = list(db.agent_run_logs.find({
        "workspace_id": c.get("workspace_id"),
        "account_id": c.get("account_id"),
        "contact_wxid": c.get("wxid"),
        "created_at": {"$gte": since},
    }, {"created_at":1, "trigger":1, "gateway_status":1, "final_review_status":1, "decision_should_reply":1}).sort("created_at", -1).limit(5))
    if not runs:
        print(f"    (no run logs in last 24h)")
    for r in runs:
        print(f"    {r.get('created_at')} trigger={r.get('trigger')} gw={r.get('gateway_status')} review={r.get('final_review_status')} should_reply={r.get('decision_should_reply')}")

print("\n=== recent agent_events related to webhook delivery (last 24h) ===")
for ev in db.agent_events.find({
    "kind": {"$in": [
        "webhook_unknown_app_id",
        "webhook_managed_contact_account_mismatch",
        "webhook_rate_limited",
        "send_gateway_blocked",
        "agent_error",
    ]},
    "created_at": {"$gte": since},
}).sort("created_at", -1).limit(40):
    print(f"  {ev.get('created_at')} kind={ev.get('kind')} status={ev.get('status')} acct={ev.get('account_id')} wx={ev.get('contact_wxid')} :: {ev.get('summary')}")
