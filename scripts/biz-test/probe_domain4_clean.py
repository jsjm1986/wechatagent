"""域④引荐链路取证探针:assist 开 + 明确签约意向,拿"干净样本"定性。

背景:域④路径2 FAIL 的真因不明——可能是 decision 没 emit namecardToSend、
reviewer 拦了、或端点畸形 JSON(autonomy_field_violation 缺16字段)砸中升档轮被
guard 正拦(红线工作,~25%)。本探针对每轮发 assist 开+签约意向,查完整链路:
  ptier 升档 → autonomy violation? → decision emit namecard? → reviewer 放行? → outbox 入卡?
畸形 JSON 轮(autonomy_field_violation)自动跳过重试,最多 N 轮拿 1 个干净样本。

干净样本判据:该轮无 autonomy_field_violation 事件(LLM 输出完整,走到了真实评审)。
拿到后逐层报告,数据说话定④是否真有项目级缺陷。不改业务,纯取证。

跑法:export DEPLOY_PASS=...; python -u scripts/biz-test/probe_domain4_clean.py
"""
import sys
import json
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "④引荐取证"
WXID = "biztest_c4probe"
MAX_ROUNDS = 6  # 端点畸形率本时段偏高(两形态叠加),6 轮穿噪声拿干净样本


def _events_for_run(wxid: str, run_id: str) -> list[dict]:
    # agent_events 顶层无 run_id(在 details.run_id,且非所有事件都有)。每轮跑前
    # reset_contact_conversation 已清空本 contact 的 agent_events,故取最近一批即本轮事件。
    rows = _lib.mongo_json(
        f'db.agent_events.find({{contact_wxid:"{wxid}"}},'
        '{kind:1,status:1,summary:1,details:1,created_at:1,_id:0})'
        '.sort({_id:-1}).limit(15).toArray()'
    )
    return rows if isinstance(rows, list) else []


def _review_for_run(wxid: str, run_id: str) -> dict:
    rows = _lib.mongo_json(
        f'db.agent_decision_reviews.find({{contact_wxid:"{wxid}",run_id:"{run_id}"}},'
        '{status:1,scores:1,reply_text:1,review_summary:1,risks:1,next_best_action:1,_id:0})'
        '.sort({_id:-1}).limit(1).toArray()'
    )
    return rows[0] if isinstance(rows, list) and rows else {}


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "biztest 引荐取证客户")
    _lib.mongo('db.referral_cards.deleteMany({display_name:/^biztest_probe/})')
    _lib.mongo(
        'db.referral_cards.insertOne({'
        'display_name:"biztest_probe_顾问王老师",target_wxid:"biztest_advisor",'
        'review_status:"approved",enabled:true,target_stages:[],workspace_id:"default",'
        'send_trigger_hint:"客户明确要签约或到店参观时引荐",'
        f'account_id:"{account_id}",created_at:new Date(),updated_at:new Date()'
        '})'
    )

    clean_found = False
    for rnd in range(1, MAX_ROUNDS + 1):
        _lib.reset_contact_conversation(account_id, WXID)
        # assist 开
        _lib.mongo(
            f'db.contacts.updateOne({{wxid:"{WXID}",account_id:"{account_id}"}},'
            '{$set:{"domain_attributes.assist_mode_override":"force_on"}})'
        )
        print(f"\n[{DOMAIN}] === 第 {rnd}/{MAX_ROUNDS} 轮:assist 开 + 明确签约意向 ===", flush=True)
        t0 = time.time()
        run = _lib.send_and_wait(app_id, WXID, "我想尽快签约报名，能给我安排专属顾问对接吗？",
                                 f"p4r{rnd}", max_wait=600)
        print(f"  耗时 {time.time()-t0:.1f}s run={run}", flush=True)
        if run is None:
            print(f"[{DOMAIN}] 第{rnd}轮 run=None(端点故障已自动重试仍失败),跳过", flush=True)
            continue
        run_id = run.get("run_id", "")
        status = run.get("status", "")
        frs = run.get("final_review_status", "")

        evs = _events_for_run(WXID, run_id)
        kinds = [e.get("kind") for e in evs]
        # 两种畸形 JSON 形态都要跳过(都是端点 tool_use 劫持/截断,~25%,非项目 bug):
        #   ① autonomy_field_violation:decision 缺必填字段被自治 guard 拦
        #   ② ptier_self_assessment_malformed:sufficiency 解析为空,自评畸形→没升档
        MALFORMED_KINDS = ("autonomy_field_violation", "ptier_self_assessment_malformed")
        malformed = any(k in MALFORMED_KINDS for k in kinds)
        ptier = next((e for e in evs if e.get("kind") == "ptier_run_tier"), {})
        ptd = ptier.get("details", {}) if isinstance(ptier, dict) else {}

        print(f"  status={status} final={frs}", flush=True)
        print(f"  ptier: tier={ptd.get('tier_used')} suff={ptd.get('sufficiency')} "
              f"escalated={ptd.get('escalated')}", flush=True)
        print(f"  events={kinds}", flush=True)

        if malformed:
            bad = [k for k in kinds if k in MALFORMED_KINDS]
            print(f"[{DOMAIN}] 第{rnd}轮=畸形JSON轮({bad}),端点噪声非bug,跳过取样", flush=True)
            continue

        # no_reply 轮:AI 没产出回复(可能 fast_chat 判无需回复/上下文变化),没走到引荐决策,跳过
        if status == "no_reply" or run.get("lifecycle") == "context_changed":
            print(f"[{DOMAIN}] 第{rnd}轮=no_reply/context_changed,未走到引荐决策点,跳过取样", flush=True)
            continue

        # 干净样本:走到了真实评审
        clean_found = True
        review = _review_for_run(WXID, run_id)
        ob = _lib.latest_outbox(WXID, limit=8)
        has_card = any(o.get("referral_card_id") for o in ob)
        print(f"\n[{DOMAIN}] >>> 干净样本(第{rnd}轮,无畸形JSON) <<<", flush=True)
        print(f"  final_status={frs}", flush=True)
        print(f"  reply_text={str(review.get('reply_text'))[:200]}", flush=True)
        print(f"  scores={json.dumps(review.get('scores',{}),ensure_ascii=False)}", flush=True)
        print(f"  review_summary={str(review.get('review_summary'))[:300]}", flush=True)
        print(f"  outbox 有名片(referral_card_id)? {has_card}", flush=True)
        print(f"  outbox={json.dumps(ob,ensure_ascii=False,default=str)[:400]}", flush=True)

        # 逐层定性
        if has_card:
            print(f"\n[{DOMAIN}] 结论:✅引荐链路通——干净轮 decision emit 名片→reviewer 放行→入 outbox。"
                  f"④无项目级缺陷,前述 FAIL 系畸形JSON端点噪声。", flush=True)
        elif frs in ("held_by_ai_policy", "blocked_unverified_product_claim"):
            print(f"\n[{DOMAIN}] 结论:⚠️干净轮仍被 reviewer/gateway 拦(final={frs})——"
                  f"查 review_summary 看拦的是引荐动作还是夹带的产品声明。这才是真缺口。", flush=True)
        else:
            print(f"\n[{DOMAIN}] 结论:⚠️干净轮 final={frs} 但无名片入 outbox——"
                  f"decision 可能没 emit namecardToSend(升档拿到清单却没选材)。查 next_best_action。", flush=True)
            print(f"  next_best_action={json.dumps(review.get('next_best_action',{}),ensure_ascii=False)[:300]}", flush=True)
        break

    if not clean_found:
        print(f"\n[{DOMAIN}] {MAX_ROUNDS}轮全是畸形JSON/run=None,端点本时段不稳,标 BLOCKED,"
              f"端点恢复后复跑(非项目bug)。", flush=True)


if __name__ == "__main__":
    main()
