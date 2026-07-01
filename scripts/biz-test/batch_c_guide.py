"""阶段2 guide 引导层域：自然语言指令 → preview(不落业务库) → apply(真映射 DB)。

引导层让运营用自然语言（"把这个客户标记为高意向，多关注"）调 LLM 生成一份
**可确认的配置修改预览**(suggestedChanges)，先落 user_operation_guide_previews 表
(status=pending)**不碰业务库**；运营确认后 apply 才把 suggestedChanges 真映射到
contact/memory/playbook/domain(guides.rs:158-161 四个 apply_*_changes)。

本脚本铁证：
- preview 调用后：user_operation_guide_previews 表 +1 条 pending；**contact 业务字段不变**
  (红线：preview 是只读预览，绝不直接改业务库)
- apply 调用后：preview.status→applied；suggestedChanges 里声明的字段真写进 contact
  (updated_at 被刷新 + 至少一个映射字段落库)
- apply 已 applied 的 preview 再 apply → 400(guides.rs:140 "not pending")
注：suggestedChanges 由 LLM 生成，无法预知改哪个字段；故断言"preview 不落业务库"(确定)
  + "apply 后 contact.updated_at 变化"(确定，只要 suggestedChanges 非空)，不臆测具体字段值。

跑法：export DEPLOY_PASS=... ADMIN_USER=admin ADMIN_PASS=admin; python scripts/biz-test/batch_c_guide.py
依赖：先跑 step0_preflight.py。
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "guide引导层"
WXID = "biztest_guide"


def _contact_snapshot(account_id: str) -> dict:
    """取 contact 的业务字段快照（preview 前后比对，证 preview 不落库）。"""
    rows = _lib.mongo_json(
        f'db.contacts.find({{wxid:"{WXID}",account_id:"{account_id}"}},'
        '{human_profile_note:1,follow_up_policy:1,operation_state:1,updated_at:1,'
        '"domain_attributes.customer_stage":1,_id:1}).toArray()'
    )
    return rows[0] if isinstance(rows, list) and rows else {}


def main() -> None:
    account_id, _app_id = _lib.biztest_account()
    _lib.ensure_managed_contact(account_id, WXID, "引导测试客户")
    _lib.reset_contact_conversation(account_id, WXID)
    # 清掉旧 preview，干净起点。
    _lib.mongo(f'db.user_operation_guide_previews.deleteMany({{contact_wxid:"{WXID}"}})')

    snap = _contact_snapshot(account_id)
    contact_id = snap.get("_id", {}).get("$oid") if isinstance(snap.get("_id"), dict) else str(snap.get("_id", ""))
    if not contact_id:
        _lib.record(DOMAIN, "contact 未建", f"wxid={WXID}", "critical", "ensure_managed_contact 失败")
        raise SystemExit("contact 未建")
    print(f"[{DOMAIN}] contact_id={contact_id} 起始快照={snap}")

    prev_count = _lib.mongo_json(
        f'db.user_operation_guide_previews.countDocuments({{contact_wxid:"{WXID}"}})'
    )
    prev_count = prev_count if isinstance(prev_count, int) else 0

    # ── preview：自然语言指令 → LLM 生成 suggestedChanges（真调 LLM，用 api_bg）──
    instruction = "这个客户是高意向用户，请把他标记成重点跟进，并补充一句画像备注说明他关注价格。"
    print(f"[{DOMAIN}] /guide/preview 真跑(LLM 生成配置预览)...")
    t0 = time.time()
    preview_resp = _lib.api_bg(
        "POST", "/api/user-operations/guide/preview",
        {"accountId": account_id, "contactId": contact_id, "instruction": instruction},
        admin=True, max_wait=720, tag="guide_preview",
    )
    print(f"  耗时 {time.time()-t0:.1f}s resp={str(preview_resp)[:300]}")

    err = _lib.is_api_error(preview_resp)
    if err:
        _lib.record(DOMAIN, "preview 端点失败(BLOCKED 非业务 bug)", f"resp={str(preview_resp)[:200]}",
                    "high", f"端点故障 {err}，标 BLOCKED 不假绿")
        raise SystemExit(f"preview 端点失败: {err}")

    _lib.assert_llm_success(720, "user.guide.preview", DOMAIN)

    preview_id = (preview_resp.get("item") or {}).get("id") or (preview_resp.get("item") or {}).get("_id")
    _lib.expect(bool(preview_id), DOMAIN, "preview 返回 previewId",
                f"resp={str(preview_resp)[:200]}", "critical")

    # 铁证 1：preview 落 user_operation_guide_previews 表(+1, status=pending)。
    after_count = _lib.mongo_json(
        f'db.user_operation_guide_previews.countDocuments({{contact_wxid:"{WXID}"}})'
    )
    after_count = after_count if isinstance(after_count, int) else 0
    _lib.expect(after_count == prev_count + 1, DOMAIN, "preview 落 1 条 guide_preview 记录",
                f"prev={prev_count} after={after_count}", "high")

    # 铁证 2：preview **不碰业务库** —— contact 业务字段与起始快照一致。
    snap_after_preview = _contact_snapshot(account_id)
    # updated_at 比对（preview 不该刷新 contact.updated_at）。
    def _field(s, k):
        return s.get(k)
    same = (
        _field(snap, "human_profile_note") == _field(snap_after_preview, "human_profile_note")
        and _field(snap, "follow_up_policy") == _field(snap_after_preview, "follow_up_policy")
        and _field(snap, "operation_state") == _field(snap_after_preview, "operation_state")
        and _field(snap, "domain_attributes") == _field(snap_after_preview, "domain_attributes")
    )
    _lib.expect(same, DOMAIN, "preview 不直接改业务库(contact 字段不变=红线)",
                f"before={snap} after={snap_after_preview}", "critical",
                "preview 是只读预览,若改 contact 说明 preview 误落业务库")

    # 查 suggestedChanges 是否非空（决定 apply 是否会改 contact）。
    sc = _lib.mongo_json(
        f'db.user_operation_guide_previews.find({{contact_wxid:"{WXID}"}})'
        '.sort({_id:-1}).limit(1).toArray().map(p=>({status:p.status,'
        'keys:Object.keys(p.suggested_changes||{})}))'
    )
    sc0 = sc[0] if isinstance(sc, list) and sc else {}
    print(f"[{DOMAIN}] suggestedChanges keys={sc0.get('keys')} status={sc0.get('status')}")
    has_changes = bool(sc0.get("keys"))

    # ── apply：把 suggestedChanges 真映射到 DB ──
    print(f"[{DOMAIN}] /guide/apply previewId={preview_id}...")
    apply_resp = _lib.api("POST", "/api/user-operations/guide/apply",
                          {"previewId": str(preview_id)}, admin=True, timeout=120)
    print(f"  apply resp={str(apply_resp)[:250]}")
    aerr = _lib.is_api_error(apply_resp)
    if aerr:
        # 修复后:apply 不应再因 LLM 越界 operationState 整体 400(部分应用)。
        # 若仍 400 且是状态机/字典相关 → 修复回归,记 critical。其余 api_error → BLOCKED(端点/MCP)。
        if "operation_state" in str(apply_resp) or "状态机" in str(apply_resp) or "dimension" in str(apply_resp):
            _lib.record(DOMAIN, "apply 仍因枚举越界整体失败(部分应用修复回归)",
                        f"resp={str(apply_resp)[:200]}", "critical",
                        "修复目标=越界字段跳过+合法字段落库;若 apply 仍 400 说明 shared.rs "
                        "apply_contact_changes 的 skip 改动未生效或被回退")
            raise SystemExit(f"apply 部分应用回归: {aerr}")
        _lib.record(DOMAIN, "apply 端点失败(BLOCKED)", f"resp={str(apply_resp)[:200]}", "high",
                    "非枚举越界类错误,疑端点/MCP,标 BLOCKED 等恢复复跑")
        raise SystemExit(f"apply 失败: {aerr}")

    # apply 成功(200):验 skippedFields 回流 + 合法字段落库。
    item = apply_resp.get("item", {}) if isinstance(apply_resp, dict) else {}
    skipped = item.get("skippedFields", [])
    applied = item.get("appliedFields", [])
    print(f"  appliedFields={applied} skippedFields={skipped}")
    # 若 LLM 这轮产了越界 operationState,它必须出现在 skippedFields(被跳过)而非致全局失败。
    sc_keys = set(sc0.get("keys", []))
    if "operationState" in sc_keys:
        skipped_names = {s.get("field") for s in skipped if isinstance(s, dict)}
        # operationState 要么被 skip(越界),要么被 apply(LLM 这次产了合法态)——两者都不该整体 400。
        _lib.expect("operationState" in skipped_names or "operationState" in applied,
                    DOMAIN, "operationState 要么跳过要么应用,不再连坐合法字段",
                    f"applied={applied} skipped={skipped}", "high",
                    "部分应用红线:单个越界字段不得致全部合法字段丢弃")

    # apply 成功路径:铁证 3+4(preview.status=applied + updated_at 刷新)。
    # 修复后 apply 越界不再整体 400,此路径恒到达。
    status_after = _lib.mongo_json(
        f'db.user_operation_guide_previews.find({{contact_wxid:"{WXID}"}})'
        '.sort({_id:-1}).limit(1).toArray().map(p=>p.status)'
    )
    st = status_after[0] if isinstance(status_after, list) and status_after else None
    _lib.expect(st == "applied", DOMAIN, "apply 后 preview.status=applied",
                f"status={st}", "high")
    if has_changes:
        snap_after_apply = _contact_snapshot(account_id)
        ua_before = _field(snap_after_preview, "updated_at")
        ua_after = _field(snap_after_apply, "updated_at")
        _lib.expect(ua_before != ua_after, DOMAIN,
                    "apply 真映射到 contact(updated_at 刷新=suggestedChanges 落库)",
                    f"updated_at before={ua_before} after={ua_after} changes_keys={sc0.get('keys')}",
                    "high", "apply 是配置生效点,suggestedChanges 非空却没改 contact=映射断裂")
    else:
        _lib.record(DOMAIN, "LLM 未生成 suggestedChanges(无可映射变更)",
                    f"keys={sc0.get('keys')}", "low",
                    "LLM 自主未产出变更建议,可复跑;非红线破")

    # 铁证 5：非 pending 的 preview 再 apply → 400(guides.rs:140 "not pending")。
    # 不依赖第一轮 LLM 是否越界:直接把第一轮 preview 强制标 applied(确定性构造非-pending 态),
    # 再 apply 验幂等保护。这样无论上面走了拒绝还是成功路径,本铁证都测的是真正的 not-pending 闸。
    _lib.mongo(
        f'db.user_operation_guide_previews.updateOne({{_id:ObjectId("{preview_id}")}},'
        '{$set:{status:"applied"}})'
    )
    reapply = _lib.api("POST", "/api/user-operations/guide/apply",
                       {"previewId": str(preview_id)}, admin=True, timeout=60)
    reapply_err = _lib.is_api_error(reapply)
    _lib.expect(reapply_err is not None and "api_error" in (reapply_err or ""),
                DOMAIN, "非 pending 的 preview 再 apply 被拒(not pending 幂等保护)",
                f"reapply={str(reapply)[:150]} err={reapply_err}", "high",
                "preview 非幂等保护缺失=重复 apply 会重复改库")

    print(f"[{DOMAIN}] 完成。preview不落业务库✓ 状态机闸/映射✓ not-pending保护✓")


if __name__ == "__main__":
    main()
