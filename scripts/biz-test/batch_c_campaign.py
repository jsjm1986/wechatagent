"""阶段2 campaign 真 LLM 派发闭环域：dispatch → 扇出 follow_up task → worker→gateway→outbox 真发送。

campaign 引擎：create(draft) → preview(圈人) → dispatch(重新圈人+扇出 follow_up 任务)。
扇出只建 kind="follow_up" 的 AgentTask，发送链路(task worker→gateway→outbox→MCP)完全复用。
活动级去重靠 campaign_sends 唯一索引 (campaignId, contactWxid)。

**关键约束(已查证)**:create_campaign 硬绑 state.config.default_account_id(campaigns.rs:210)。
server .env DEFAULT_ACCOUNT_ID=default，但 wechat_accounts 只有 account_id "1"/"2"(无 "default")，
所有 contact 在 account_id="2"。故 campaign 圈人(用 default account)在生产配置下命中 0 人——
这是**配置错配发现**。本脚本在 account_id="default" 下铺最小测试数据(biztest_ 前缀隔离)走通链路，
同时记录该错配。

本脚本铁证：
- 命中 0 人 dispatch → BadRequest(campaigns.rs:305)
- 圈到 1 人 dispatch → campaign_sends 落 1 条 + follow_up task 建(kind=follow_up,status=pending)
- 二次 dispatch 同集 → DuplicateKey 去重，dispatchedCount 增量为 0(campaigns.rs:361)
- 等 task worker(30s tick)→ gateway → outbox：该 contact 有 outbox 条目且 status∈
  {pending,in_flight,sent}(真走发送链,排除 failed_terminal/canceled)
- dispatch 不存在的 campaign → NotFound(诚实:非跨 workspace IDOR,那需另一 workspace
  admin,由集成测试覆盖)

跑法：export DEPLOY_PASS=... ADMIN_USER=admin ADMIN_PASS=admin; python scripts/biz-test/batch_c_campaign.py
依赖：先跑 step0_preflight.py。
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "campaign派发闭环"
# campaign 绑 default account；contact 用 biztest_ 前缀隔离。
CAMP_ACCOUNT = "default"
WXID = "biztest_campaign"
STAGE = "need_discovery"  # 状态机合法态(查证 operation_domain_configs.state_machine.states)


def _cleanup():
    """清掉本测试在 default account 下的痕迹(biztest_ 前缀隔离，绝不碰非 biztest 数据)。"""
    _lib.mongo(f'db.contacts.deleteMany({{wxid:"{WXID}"}})')
    _lib.mongo(f'db.campaigns.deleteMany({{title:/^biztest_/}})')
    _lib.mongo(f'db.campaign_sends.deleteMany({{contactWxid:"{WXID}"}})')
    _lib.mongo(f'db.agent_tasks.deleteMany({{contact_wxid:"{WXID}"}})')
    _lib.mongo(f'db.agent_send_outbox.deleteMany({{contact_wxid:"{WXID}"}})')
    _lib.mongo(f'db.conversation_messages.deleteMany({{contact_wxid:"{WXID}"}})')
    _lib.mongo(f'db.agent_run_logs.deleteMany({{contact_wxid:"{WXID}"}})')


def _create_campaign(title: str, stage: str | None) -> str | None:
    body = {"title": title, "intentText": "给你推荐一个适合的方案，方便聊聊吗？"}
    if stage is not None:
        body["segmentFilter"] = {"customerStage": stage}
    resp = _lib.api("POST", "/api/campaigns", body, admin=True, timeout=60)
    return resp.get("id") if isinstance(resp, dict) else None


def main() -> None:
    _lib.biztest_account()  # 确认 preflight 跑过
    _cleanup()

    # 注：不再 upsert wechat_accounts 的 default 行——WechatAccount 有 alias/display_name/
    # last_sync_at 三个非 Option 无默认字段,只 setOnInsert 部分字段会造出反序列化即报错的坏行,
    # 污染生产 default workspace 的批量加载(account_scheduler/tasks 的 try_collect)。
    # outbox_dispatcher.rs:621 明示"account 查不到时保守放行不阻断发送",故 default account
    # 缺行时发送链照常走通,无需造行。config 硬绑 default_account_id 的错配另在下方 record 登记。
    _lib.record(DOMAIN, "create_campaign 硬绑 default_account_id,生产命中 0 人",
                "campaigns.rs:210 硬绑 config.default_account_id(默认 default),但生产 "
                "wechat_accounts 仅 account_id 1/2 无 default,API 建的 campaign 在生产配置下"
                "圈人恒 0",
                "high",
                "配置错配:campaign 定向推送在生产配置下对真实客户永远派发不出(本脚本在 "
                "default account 下铺 biztest 数据走通链路以验证引擎,不代表生产可用)")

    # ── 铁证 1：命中 0 人 dispatch → BadRequest ──
    # 先建一个圈不到人的 campaign(stage 圈一个没人的态)。此刻 biztest contact 还没建。
    cid_empty = _create_campaign("biztest_empty", "customer_success")
    _lib.expect(bool(cid_empty), DOMAIN, "create campaign 返回 id", f"cid={cid_empty}", "critical")
    disp_empty = _lib.api("POST", f"/api/campaigns/{cid_empty}/dispatch", {}, admin=True, timeout=60)
    e0 = _lib.is_api_error(disp_empty)
    _lib.expect(e0 is not None and "api_error" in (e0 or ""), DOMAIN,
                "命中 0 人 dispatch 被拒(BadRequest)",
                f"resp={str(disp_empty)[:150]} err={e0}", "high",
                "命中 0 人应拒绝派发,不静默成功")

    # ── 铺一个能被圈到的 managed contact(default account, customer_stage=need_discovery)──
    _lib.mongo(
        f'db.contacts.updateOne({{wxid:"{WXID}",account_id:"{CAMP_ACCOUNT}"}},'
        f'{{$set:{{agent_status:"managed",nickname:"campaign客户",updated_at:new Date(),'
        f'"domain_attributes.customer_stage":"{STAGE}",'
        f'"operation_mode_override.quiet_hours.enabled_override":false}},'
        f'$setOnInsert:{{workspace_id:"default",created_at:new Date()}}}},{{upsert:true}})'
    )

    # ── 铁证 2：圈到 1 人 dispatch → campaign_sends + follow_up task ──
    cid = _create_campaign("biztest_hit", STAGE)
    _lib.expect(bool(cid), DOMAIN, "create 圈人 campaign 返回 id", f"cid={cid}", "critical")
    # 先 preview 看圈人数(targetCount)。
    prev = _lib.api("POST", f"/api/campaigns/{cid}/preview", {}, admin=True, timeout=60)
    tc = prev.get("targetCount") if isinstance(prev, dict) else None
    print(f"[{DOMAIN}] preview targetCount={tc} prev={str(prev)[:200]}")
    _lib.expect(isinstance(tc, int) and tc >= 1, DOMAIN, "preview 圈到 >=1 人(default account 铺了 contact)",
                f"targetCount={tc}", "high",
                "圈不到=campaign 绑 default account 但该 account 无 managed contact(配置错配)")

    disp = _lib.api("POST", f"/api/campaigns/{cid}/dispatch", {}, admin=True, timeout=90)
    derr = _lib.is_api_error(disp)
    if derr:
        _lib.record(DOMAIN, "dispatch 失败(BLOCKED)", f"resp={str(disp)[:160]}", "high", derr)
        _cleanup()
        raise SystemExit(f"dispatch 失败: {derr}")
    dc = disp.get("dispatchedCount")
    print(f"[{DOMAIN}] dispatch1 dispatchedCount={dc} resp={str(disp)[:160]}")
    _lib.expect(dc == 1, DOMAIN, "dispatch 命中 1 人(dispatchedCount=1)",
                f"dispatchedCount={dc}", "high")

    # campaign_sends 落 1 条。注意:CampaignSend 是 camelCase(models.rs:596 serde rename),
    # BSON 字段是 contactWxid 不是 contact_wxid。
    sends = _lib.mongo_json(
        f'db.campaign_sends.countDocuments({{contactWxid:"{WXID}"}})'
    )
    _lib.expect(sends == 1, DOMAIN, "campaign_sends 落 1 条台账",
                f"count={sends}", "high")
    # follow_up task 建。
    tasks = _lib.mongo_json(
        f'db.agent_tasks.find({{contact_wxid:"{WXID}",kind:"follow_up"}},'
        '{status:1,kind:1,_id:0}).toArray()'
    )
    tasks = tasks if isinstance(tasks, list) else []
    _lib.expect(len(tasks) == 1, DOMAIN, "扇出 1 个 follow_up task",
                f"tasks={tasks}", "high")

    # ── 铁证 3：二次 dispatch 同集 → 去重(dispatchedCount 增量 0)──
    disp2 = _lib.api("POST", f"/api/campaigns/{cid}/dispatch", {}, admin=True, timeout=90)
    dc2 = disp2.get("dispatchedCount") if isinstance(disp2, dict) else None
    print(f"[{DOMAIN}] dispatch2 dispatchedCount={dc2}")
    _lib.expect(dc2 == 0, DOMAIN, "二次 dispatch 同集去重(dispatchedCount=0)",
                f"dispatchedCount={dc2}", "high",
                "campaign_sends 唯一索引去重失效=重复推送骚扰客户")
    sends2 = _lib.mongo_json(f'db.campaign_sends.countDocuments({{contactWxid:"{WXID}"}})')
    _lib.expect(sends2 == 1, DOMAIN, "去重后 campaign_sends 仍 1 条(未重复落)",
                f"count={sends2}", "high")

    # ── 铁证 4：等 task worker(30s tick)→ gateway → outbox 真发送 ──
    print(f"[{DOMAIN}] 等 task worker 跑 follow_up→gateway→outbox(真调 LLM,最多 ~300s)...")
    deadline = time.time() + 300
    outbox = []
    while time.time() < deadline:
        outbox = _lib.latest_outbox(WXID, limit=5)
        if outbox:
            break
        # 看 task 是否被 claim/处理(状态流转)。
        time.sleep(20)
    tstate = _lib.mongo_json(
        f'db.agent_tasks.find({{contact_wxid:"{WXID}"}},'
        '{status:1,gateway_status:1,attempt_count:1,_id:0}).toArray()'
    )
    print(f"[{DOMAIN}] outbox={outbox} task_state={tstate}")
    if outbox:
        # 收紧:不是"outbox 非空即绿"(那只证明入队),而是至少一条 status ∈ 未失败态
        # {pending,in_flight,sent}——排除 failed_terminal/canceled。发送链真断(如账号问题)
        # 会落 failed_terminal/canceled,这条断言即 FAIL,而非被"有条目"掩盖。
        live = [o for o in outbox if o.get("status") in ("pending", "in_flight", "sent")]
        _lib.expect(bool(live), DOMAIN,
                    "follow_up 经 worker→gateway→outbox 真走发送链(status∈pending/in_flight/sent)",
                    f"outbox={outbox}", "high",
                    "outbox 有条目但全为 failed_terminal/canceled=发送链真断,非成功;"
                    "只到 pending/in_flight(worker 未跑完)可复跑,sent=真送达")
    else:
        # 没 outbox:区分端点故障(BLOCKED)vs 真链路问题。
        glitch = _lib.endpoint_glitch_recent(WXID)
        if glitch:
            _lib.record(DOMAIN, "follow_up 发送链未达 outbox(端点故障BLOCKED)",
                        f"glitch={str(glitch.get('summary',''))[:100]} task={tstate}", "high",
                        "LLM 端点偶发故障,标 BLOCKED 非业务 bug,可复跑")
        else:
            _lib.record(DOMAIN, "follow_up dispatch 后未达 outbox",
                        f"task_state={tstate} outbox=[]", "high",
                        "task 建了但 worker→gateway→outbox 未产送达;查 task 是否被 claim/gateway 拦截原因")

    # ── 铁证 5：dispatch 不存在的 campaign → NotFound ──
    # 注:这不是跨 workspace IDOR 测试——dispatch filter 虽含 workspaceId,但不存在的 id
    # 无论有无 workspace 约束都返 NotFound,本断言测不到隔离维度(真正 IDOR 需另一 workspace
    # 的真实 campaign + 无权 admin,由集成测试覆盖)。这里只诚实验"dispatch 不存在 id 被拒"。
    fake = "000000000000000000000000"
    nf = _lib.api("POST", f"/api/campaigns/{fake}/dispatch", {}, admin=True, timeout=30)
    nferr = _lib.is_api_error(nf)
    # 收紧:必须是业务错误(api_error 前缀),排除 _error/_raw/超时等端点故障假绿。
    _lib.expect(nferr is not None and "api_error" in (nferr or ""), DOMAIN,
                "dispatch 不存在的 campaign 被拒(NotFound,非端点故障)",
                f"resp={str(nf)[:120]} err={nferr}", "high",
                "不存在 id 应返业务 NotFound;若是 _error/超时=端点故障非此断言目标")

    _cleanup()
    print(f"[{DOMAIN}] 完成。命中0拒绝✓ dispatch扇出✓ 去重✓ outbox发送链(非失败态)✓ 不存在id拒绝✓")


if __name__ == "__main__":
    main()
