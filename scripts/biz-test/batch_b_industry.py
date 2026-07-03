"""批 B：行业闭环（心理/教育/医美 × 域⑦兼容 + 域⑫画像playbook）。

每行业：AI 生成 profile(guide.domain_profile.draft)→断红线(落 draft+is_active=false+seeded_by=generated_by_ai
+生成状态机)→人审 activate(热生效)→该行业下对话→验 customer_stage/operation_state 落该行业
canonical 值(非销售域典型值)。

**本任务切换全局 active profile**，必须最后跑(批 A 全部完成后)，跑前存档原 active、finally 恢复。

实测确认：
- domain_profiles 字段 is_active/seeded_by(AI=generated_by_ai)/generated_state_machine/profile_id。
- generate POST /api/admin/domain-profiles/generate body camelCase {businessDescription,profileId,displayName}
  →响应 {id,profileId};prompt_key=guide.domain_profile.draft(走 generate_agent_json,写 log)。
- activate POST /api/admin/domain-profiles/:id/activate,热生效(invalidate cache,无需重启)。
- customer_stage 落 contacts.domain_attributes.customer_stage(过 C2 状态机校验);
  operation_state 落 agent_decision_reviews.operation_state。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_b_industry.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

D7 = "⑦行业兼容"
D12 = "⑫画像playbook"

# 销售域典型 stage 值（落这些 = 行业 profile 没生效/假通用）
SALES_STAGES = {"new_contact", "closing", "negotiation", "objection_handling",
                "需求探索", "成交推进", "异议处理", "陌生接触"}

INDUSTRIES = [
    ("biztest_psych", "心理陪伴",
     "为情绪困扰用户提供陪伴式倾听，不做诊断不卖课，引导用户表达和梳理情绪",
     "我最近压力很大，总是睡不着"),
    ("biztest_edu", "教育培训",
     "少儿编程培训机构，按试听-评估-报名-续费推进，关注孩子学习兴趣和家长预算",
     "想给孩子了解一下编程课"),
    ("biztest_med", "医美咨询",
     "轻医美项目咨询，严格合规话术，关注客户需求与到院面诊预约",
     "想咨询一下你们的项目"),
]


def _profile_oid(pid: str) -> str:
    row = _lib.mongo_json(f'db.domain_profiles.findOne({{profile_id:"{pid}"}},{{_id:1}})')
    if not isinstance(row, dict):
        return ""
    oid = row.get("_id")
    return str(oid.get("$oid", "")) if isinstance(oid, dict) else str(oid or "")


def _contact_stage(account_id: str, wxid: str) -> str:
    row = _lib.mongo_json(
        f'db.contacts.findOne({{wxid:"{wxid}",account_id:"{account_id}"}},'
        '{domain_attributes:1,_id:0})'
    )
    if isinstance(row, dict):
        da = row.get("domain_attributes") or {}
        if isinstance(da, dict):
            return str(da.get("customer_stage", ""))
    return ""


def run_industry(pid: str, name: str, desc: str, opener: str, app_id: str, account_id: str) -> None:
    # 1. AI 生成行业 profile（长 LLM，后台轮询）
    print(f"[{D7}] {name} 生成 profile...")
    _lib.mongo(f'db.domain_profiles.deleteMany({{profile_id:"{pid}"}})')
    gen = _lib.api_bg(
        "POST", "/api/admin/domain-profiles/generate",
        {"businessDescription": desc, "profileId": pid, "displayName": name},
        admin=True, max_wait=600, tag="genprof",
    )
    print(f"  gen={str(gen)[:200]}")
    _lib.assert_llm_success(600, "guide.domain_profile.draft", D7)

    # 2. 红线：落 draft + 未生效 + seeded_by=generated_by_ai + 生成了状态机
    row = _lib.mongo_json(
        f'db.domain_profiles.findOne({{profile_id:"{pid}"}},'
        '{is_active:1,seeded_by:1,generated_state_machine:1,_id:0})'
    )
    row = row if isinstance(row, dict) else {}
    _lib.expect(row.get("is_active") in (False, None), D7,
                f"{name} AI 生成的 profile 未自动生效(红线:AI 不自动 activate)",
                f"is_active={row.get('is_active')}", "critical",
                "AI 生成直接 is_active=true=AI 自作主张上线行业配置红线破")
    _lib.expect("generated" in str(row.get("seeded_by", "")), D7,
                f"{name} seeded_by=generated_by_ai(可溯源 AI 生成)",
                f"seeded_by={row.get('seeded_by')}", "high")
    _lib.expect(bool(row.get("generated_state_machine")), D7,
                f"{name} AI 为新行业生成了状态机(通用化核心能力)",
                f"has_sm={bool(row.get('generated_state_machine'))}", "high",
                "未生成状态机=AI 无法为新行业建阶段流转,通用化不成立")

    # 域⑫：playbook 生成（去销售偏见）——可选子断言
    # （/operation-playbooks/generate 需 profile context，本版聚焦 profile 生成红线）

    # 3. 人审 activate（热生效）
    did = _profile_oid(pid)
    if not did:
        _lib.record(D7, f"{name} profile 落库失败拿不到 _id，跳过 activate", f"pid={pid}", "high",
                    "generate 未落库")
        return
    print(f"[{D7}] {name} activate...")
    _lib.api("POST", f"/api/admin/domain-profiles/{did}/activate", {}, admin=True)
    time.sleep(4)  # 热生效缓存失效

    # 4. 该行业下对话 → 验 stage 落该行业 canonical 值（非销售域）
    wxid = f"{pid}_c"
    _lib.ensure_managed_contact(account_id, wxid, f"biztest {name}客户")
    _lib.reset_contact_conversation(account_id, wxid)
    print(f"[{D7}] {name} 行业下对话...")
    run = _lib.send_and_wait(app_id, wxid, opener, f"{pid}_m", max_wait=600)
    _lib.expect(run is not None, D7, f"{name} 行业对话 webhook 完成", f"run={run}", "high")
    _lib.assert_llm_success(600, "user.reply.task", f"{D7}/{name}")

    stage = _contact_stage(account_id, wxid)
    dr = _lib.latest_decision_review(wxid)
    op_state = str(dr.get("operation_state", ""))
    not_sales = (stage not in SALES_STAGES) and (op_state not in SALES_STAGES)
    _lib.expect(not_sales, D7, f"{name} customer_stage/operation_state 落该行业值(非销售域典型值)",
                f"stage={stage} operation_state={op_state}", "high",
                "落 new_contact/closing 等销售值=行业 profile 未生效或假通用(状态机没换)")

    # 心理域额外：纯情感回复不被 grounding 误拦（funnel=false 域）
    if "psych" in pid:
        ob = len(_lib.latest_outbox(wxid, limit=10))
        _lib.expect(ob > 0, D7, "心理域纯情感回复不被 grounding 误拦(funnel=false)",
                    f"outbox_count={ob}", "high",
                    "纯情感倾听无产品声明却被 grounding 拦=情感域被销售域 grounding 误伤")


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    # 存档原 active profile
    orig = _lib.mongo_json('db.domain_profiles.findOne({is_active:true},{profile_id:1,_id:0})')
    orig_pid = orig.get("profile_id") if isinstance(orig, dict) else None
    print(f"原 active profile = {orig_pid}（跑完恢复）")

    try:
        for pid, name, desc, opener in INDUSTRIES:
            print(f"\n===== 行业: {name} =====")
            run_industry(pid, name, desc, opener, app_id, account_id)
    finally:
        # 恢复原 active（即便中途断言失败也恢复，避免污染后续）
        if orig_pid:
            od = _profile_oid(orig_pid)
            if od:
                _lib.api("POST", f"/api/admin/domain-profiles/{od}/activate", {}, admin=True)
        print(f"已恢复 active profile = {orig_pid}")

    print("批 B 完成")


if __name__ == "__main__":
    main()
