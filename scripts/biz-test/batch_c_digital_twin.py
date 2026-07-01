"""阶段2 digital-twin 域：关系类型建议保守闭环（识别→pending 建议→approve 写回）。

数字分身支持 customer/peer/friend 三类关系。LLM 在对话中识别关系信号 → decision.
agent_generated_signals → gateway 提取 → 落 relationship_type_suggestions(status=pending,
**不直接生效**) → 运营 approve → 写 contact.domain_attributes.relationship_type(canonical)。

本脚本铁证：
- 发强"同行"信号对话 → 真调 LLM agent 决策链；若 LLM 产出 pending 建议：
  suggested_value ∈ {customer,peer,friend}（canonical,非臆造）
- **确定性验 approve→写回闭环**：用独立 biztest contact 直接 seed 一条 pending 建议
  (suggested_value=peer)，approve 后断言 contact.domain_attributes.relationship_type
  写回该 canonical。这条不依赖 LLM 是否自产建议(自产是自主行为、单轮常 0),保证
  approve→写回红线每轮必被执行(否则写入链坏掉/approve 不写 contact 会被漏掉)。
注：LLM 关系识别是自主行为,未必每轮产出；未产生时记为观察(low)不算 failed;
   隔离(跨 workspace approve→NotFound)由 lib/集成测试覆盖,本脚本不重复(不冒充 IDOR)。

跑法：export DEPLOY_PASS=... ADMIN_USER=admin ADMIN_PASS=admin; python scripts/biz-test/batch_c_digital_twin.py
依赖：先跑 step0_preflight.py。
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "digital-twin关系建议"
WXID = "biztest_peer"
CANONICAL = {"customer", "peer", "friend"}


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "同行交流")
    _lib.reset_contact_conversation(account_id, WXID)
    # 清掉旧 pending 建议，保证干净起点。
    _lib.mongo(f'db.relationship_type_suggestions.deleteMany({{contact_id:null}})')
    rows = _lib.mongo_json(
        f'db.contacts.find({{wxid:"{WXID}",account_id:"{account_id}"}},{{_id:1}}).toArray()'
    )
    contact_id = rows[0]["_id"]["$oid"] if rows and isinstance(rows[0]["_id"], dict) else (str(rows[0]["_id"]) if rows else None)
    if not contact_id:
        _lib.record(DOMAIN, "contact 未建", f"wxid={WXID}", "critical")
        raise SystemExit("contact 未建")
    _lib.mongo(f'db.relationship_type_suggestions.deleteMany({{contact_id:"{contact_id}"}})')

    # 发强"同行"信号：明确表明自己也是同业，想交流获客经验（非买家口吻）。
    msgs = [
        "你好，我也是做私域运营这块的，咱们算同行，想跟你交流下获客打法。",
        "我自己也带团队做客户运营，没打算买啥，就是同行之间交流经验。",
    ]
    for i, m in enumerate(msgs, 1):
        print(f"[{DOMAIN}] 发同行信号 {i}/{len(msgs)}...")
        ok = _lib.send_and_wait(app_id, WXID, m, f"{DOMAIN}-{i}")
        _lib.expect(ok, DOMAIN, f"第{i}轮触发 agent 决策链(真调LLM)",
                    f"send_and_wait={ok}", "high")
        time.sleep(2)

    # 查是否产生 pending 建议。
    sugg = _lib.mongo_json(
        f'db.relationship_type_suggestions.find({{contact_id:"{contact_id}"}},'
        '{suggested_value:1,status:1,confidence:1,_id:1}).toArray()'
    )
    sugg = sugg if isinstance(sugg, list) else []
    print(f"[{DOMAIN}] 产生建议 {len(sugg)} 条: {sugg}")

    if not sugg:
        # 诊断:LLM 这轮把关系判断写进了 relationship_read 理性字段，还是连理性层都没判定？
        # 注意(2026-06-30 复跑确证):关系建议闭环本身健全(另一轮真产出 peer 建议 confidence=90
        # →approve 写回 contact)，单轮 0 产出是 LLM 发挥不稳/run 被 superseded 的动态测试常态，
        # **不是**结构化字段缺失类 bug。故 0 产出一律记 low 观察(可复跑)，只把诊断证据留痕，
        # 不据单轮下"prompt 引导位置缺陷"结论(那需多轮稳定复现才成立)。
        runs = _lib.mongo_json(
            f'db.agent_run_logs.find({{contact_wxid:"{WXID}"}}).sort({{created_at:-1}}).limit(1)'
            '.toArray().map(r=>({sig:(r.decision||{}).agent_generated_signals||[],'
            'rel:(r.decision||{}).relationship_read||""}))'
        )
        r0 = runs[0] if isinstance(runs, list) and runs else {}
        rel_read = r0.get("rel", "")
        sigs = r0.get("sig", [])
        judged_in_text = any(k in rel_read for k in ("同行", "对等", "peer", "朋友", "客户"))
        note = ("relationship_read 已判关系但本轮 agent_generated_signals 空"
                if judged_in_text and not sigs
                else "relationship_read 本轮也未明确判定关系")
        _lib.record(DOMAIN, f"本轮未产生关系建议({note})",
                    f"relationship_read={rel_read!r} signals={sigs}", "low",
                    "关系识别是 LLM 自主行为,单轮未产出可复跑;闭环本身下方用确定性 seed 验证,"
                    "非结构化字段缺失结论;若多轮稳定 0 产出且 relationship_read 总判对才升级调查")
        print(f"[{DOMAIN}] 本轮 LLM 未自产建议({note}),记 low 观察;下方用确定性 seed 验闭环。")
    else:
        # LLM 真产出了建议:验 canonical 值(非臆造)——这条守 gateway MachineWrite 闸红线。
        s0 = sugg[0]
        val = s0.get("suggested_value")
        _lib.expect(val in CANONICAL, DOMAIN, "LLM 自产 suggested_value 是 canonical 三类之一",
                    f"suggested_value={val} canonical={CANONICAL}", "critical",
                    "臆造非字典值=污染队列(MachineWrite 闸失效)")

    # ── 确定性验 approve→写回闭环(不依赖 LLM 是否自产建议)──
    # 用独立 biztest contact 直接 seed 一条 pending 建议,调真 approve API,断言后端真写回
    # contact.domain_attributes.relationship_type。这样 approve→写回红线每轮必被执行,
    # 不被"LLM 单轮 0 产出"跳过。seed 文档字段与 gateway.rs:4153 upsert 结构一致
    # (workspace_id/account_id/contact_id/status=pending/suggested_value/confidence/时间戳)。
    seed_wxid = "biztest_twin_seed"
    _lib.ensure_managed_contact(account_id, seed_wxid, "闭环seed客户")
    seed_rows = _lib.mongo_json(
        f'db.contacts.find({{wxid:"{seed_wxid}",account_id:"{account_id}"}},{{_id:1,workspace_id:1}}).toArray()'
    )
    if not (isinstance(seed_rows, list) and seed_rows):
        _lib.record(DOMAIN, "seed contact 未建", f"wxid={seed_wxid}", "critical")
        raise SystemExit("seed contact 未建")
    seed_cid = seed_rows[0]["_id"]["$oid"] if isinstance(seed_rows[0]["_id"], dict) else str(seed_rows[0]["_id"])
    seed_ws = seed_rows[0].get("workspace_id", "default")
    # 清旧 + seed 一条 pending 建议(suggested_value=peer 是 canonical 确定值)。
    _lib.mongo(f'db.relationship_type_suggestions.deleteMany({{contact_id:"{seed_cid}"}})')
    _lib.mongo(
        f'db.relationship_type_suggestions.insertOne({{workspace_id:"{seed_ws}",'
        f'account_id:"{account_id}",contact_id:"{seed_cid}",status:"pending",'
        f'suggested_value:"peer",confidence:88,occurrences:1,'
        f'first_seen_at:new Date(),last_seen_at:new Date()}})'
    )
    seed_sugg = _lib.mongo_json(
        f'db.relationship_type_suggestions.find({{contact_id:"{seed_cid}",status:"pending"}},'
        '{_id:1}).toArray()'
    )
    if not (isinstance(seed_sugg, list) and seed_sugg):
        _lib.record(DOMAIN, "seed 建议未落库", f"contact_id={seed_cid}", "critical",
                    "确定性 seed 失败,无法验 approve 闭环")
        raise SystemExit("seed 建议未落库")
    seed_sid = seed_sugg[0]["_id"]["$oid"] if isinstance(seed_sugg[0]["_id"], dict) else str(seed_sugg[0]["_id"])

    print(f"[{DOMAIN}] approve seed 建议 {seed_sid}(peer)...")
    seed_approved = _lib.api("POST", f"/api/admin/relationship-type-suggestions/{seed_sid}/approve",
                             {}, admin=True, timeout=60)
    seed_aerr = _lib.is_api_error(seed_approved)
    if seed_aerr:
        _lib.record(DOMAIN, "approve seed 建议端点失败(BLOCKED)",
                    f"resp={str(seed_approved)[:150]} err={seed_aerr}", "high",
                    "approve 端点故障标 BLOCKED 等恢复复跑,非业务 bug")
        raise SystemExit(f"approve 端点失败: {seed_aerr}")

    time.sleep(1)
    seed_after = _lib.mongo_json(
        f'db.contacts.find({{_id:ObjectId("{seed_cid}")}},'
        '{"domain_attributes.relationship_type":1,_id:0}).toArray()'
    )
    seed_rt = None
    if seed_after and isinstance(seed_after, list):
        da = seed_after[0].get("domain_attributes") or {}
        seed_rt = da.get("relationship_type") if isinstance(da, dict) else None
    _lib.expect(seed_rt == "peer", DOMAIN,
                "approve 后 contact.relationship_type 写回 seed 的 canonical 值(peer)",
                f"relationship_type={seed_rt} approved={str(seed_approved)[:150]}", "critical",
                "approve 是业务生效点,须真写 contact.domain_attributes.relationship_type=被批准的值")
    # 清理 seed 痕迹。
    _lib.mongo(f'db.relationship_type_suggestions.deleteMany({{contact_id:"{seed_cid}"}})')
    _lib.mongo(f'db.contacts.deleteMany({{wxid:"{seed_wxid}"}})')

    print(f"[{DOMAIN}] 完成。LLM自产建议={len(sugg)}条 + 确定性seed→approve→写回 relationship_type={seed_rt}✓")


if __name__ == "__main__":
    main()
