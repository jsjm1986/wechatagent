"""Drill into the two suspects."""
from pymongo import MongoClient
from datetime import datetime, timezone, timedelta

client = MongoClient("mongodb://localhost:27017")
db = client["wechatagent"]

since = datetime.now(timezone.utc) - timedelta(hours=72)

print("=== ALL inbound conversation_messages in last 72h, grouped by contact ===")
pipeline = [
    {"$match": {"direction": "inbound", "created_at": {"$gte": since}}},
    {"$group": {"_id": {"acct": "$account_id", "wxid": "$contact_wxid"}, "n": {"$sum": 1}, "last": {"$max": "$created_at"}}},
    {"$sort": {"last": -1}}
]
for row in db.conversation_messages.aggregate(pipeline):
    print(f"  acct={row['_id']['acct']} wxid={row['_id']['wxid']} count={row['n']} last={row['last']}")

print("\n=== contact wxid_d1swfa213aaq12 — full doc ===")
c = db.contacts.find_one({"wxid": "wxid_d1swfa213aaq12"})
import json
def safe(o):
    if hasattr(o, "isoformat"): return o.isoformat()
    if hasattr(o, "binary"): return str(o)
    return str(o)
# print only top-level keys + agent_profile inspection
if c:
    keys = list(c.keys())
    print(f"  keys={keys}")
    ap = c.get("agent_profile")
    if ap:
        if isinstance(ap, dict):
            print(f"  agent_profile keys: {list(ap.keys())}")
            # check if coreProfile appears twice via raw bson? unlikely - mongo stores as dict
            # Look at raw via bson
        else:
            print(f"  agent_profile type: {type(ap)}")

print("\n=== last 10 agent_events for wxid_d1swfa213aaq12 ===")
for ev in db.agent_events.find({"contact_wxid": "wxid_d1swfa213aaq12"}).sort("created_at", -1).limit(10):
    print(f"  {ev.get('created_at')} kind={ev.get('kind')} status={ev.get('status')} :: {ev.get('summary')}")

print("\n=== last 10 inbound for wxid_d1swfa213aaq12 ===")
for msg in db.conversation_messages.find({"contact_wxid": "wxid_d1swfa213aaq12", "direction": "inbound"}).sort("created_at", -1).limit(10):
    print(f"  {msg.get('created_at')} acct={msg.get('account_id')} content={msg.get('content','')[:80]}")

print("\n=== inbound for wxid_czpvyjvhzizj22 ANYWHERE ===")
for msg in db.conversation_messages.find({"contact_wxid": "wxid_czpvyjvhzizj22"}).sort("created_at", -1).limit(10):
    print(f"  {msg.get('created_at')} dir={msg.get('direction')} acct={msg.get('account_id')} content={msg.get('content','')[:80]}")

print("\n=== events for wxid_czpvyjvhzizj22 ===")
for ev in db.agent_events.find({"contact_wxid": "wxid_czpvyjvhzizj22"}).sort("created_at", -1).limit(10):
    print(f"  {ev.get('created_at')} kind={ev.get('kind')} status={ev.get('status')} acct={ev.get('account_id')} :: {ev.get('summary')}")

print("\n=== webhook_managed_contact_account_mismatch events (last 7 days) ===")
since7 = datetime.now(timezone.utc) - timedelta(days=7)
for ev in db.agent_events.find({"kind": "webhook_managed_contact_account_mismatch", "created_at": {"$gte": since7}}).sort("created_at", -1).limit(10):
    print(f"  {ev.get('created_at')} acct={ev.get('account_id')} wx={ev.get('contact_wxid')} :: {ev.get('summary')}")
