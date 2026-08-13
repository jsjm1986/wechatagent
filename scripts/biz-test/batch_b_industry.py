"""Industry-profile acceptance: AI draft -> publish -> activate -> exact runtime evidence.

The test records and restores the exact original active artifact. It never resolves a profile by
``profile_id`` alone and never activates an unpublished draft.
"""
import json
import sys
from pathlib import Path
from typing import Optional

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑦行业兼容/⑫画像playbook"
RESTORE_MARKER_ID = "biztest_industry_profile_restore"
INDUSTRIES = [
    ("biztest_psych", "心理陪伴",
     "为情绪困扰用户提供陪伴式倾听，不做诊断不卖课，引导用户表达和梳理情绪",
     "我最近压力很大，总是睡不着，想先找个人聊聊。"),
    ("biztest_edu", "教育培训",
     "少儿编程培训机构，按试听、评估、报名和续费推进，关注学习兴趣与家长预算",
     "想给孩子了解一下编程课，他之前没有基础。"),
    ("biztest_med", "医美咨询",
     "轻医美项目咨询，严格合规，不承诺效果，关注需求与到院面诊预约",
     "想先了解适合我的项目和面诊流程。"),
]


def _require(condition: bool, description: str, evidence: object) -> None:
    if not _lib.expect(condition, DOMAIN, description, str(evidence), "critical"):
        raise RuntimeError(f"{description}: {evidence}")


def _profile(row_id: str) -> dict:
    row = _lib.mongo_json(
        f'db.domain_profiles.findOne({{_id:ObjectId({json.dumps(row_id)})}})'
    )
    return row if isinstance(row, dict) else {}


def _active_profile() -> dict:
    row = _lib.mongo_json(
        'db.domain_profiles.findOne({is_active:true},'
        '{_id:1,profile_id:1,version:1,current_version:1,release_status:1})'
    )
    return row if isinstance(row, dict) else {}


def _state_keys(profile: dict) -> set[str]:
    machine = profile.get("generated_state_machine")
    states = machine.get("states", []) if isinstance(machine, dict) else []
    return {
        str(state.get("key")) for state in states
        if isinstance(state, dict) and str(state.get("key", "")).strip()
    }


def _restore_original(original: dict) -> None:
    original_id = _lib.bson_object_id(original.get("_id"))
    _require(bool(original_id), "原 active profile 身份可恢复", original)
    current = _profile(original_id)
    _require(current.get("release_status") == "published",
             "原 active profile 仍为 published", current)
    if not current.get("current_version"):
        rollout = _lib.api(
            "POST", f"/api/admin/domain-profiles/{original_id}/rollout", {}, admin=True,
        )
        _require(_lib.is_api_error(rollout) is None,
                 "恢复前把原版本精确 rollout 为 current", rollout)
    activated = _lib.api(
        "POST", f"/api/admin/domain-profiles/{original_id}/activate", {}, admin=True,
        timeout=180,
    )
    _require(_lib.is_api_error(activated) is None, "精确恢复原 active profile", activated)
    restored = _active_profile()
    _require(_lib.bson_object_id(restored.get("_id")) == original_id,
             "恢复后 active 指向原 immutable row", restored)


def _write_restore_marker(original_id: Optional[str]) -> None:
    _lib.mongo(
        "db.biztest_control.replaceOne("
        f"{{_id:{json.dumps(RESTORE_MARKER_ID)}}},"
        f"{{_id:{json.dumps(RESTORE_MARKER_ID)},"
        f"original_active_id:{json.dumps(original_id)},"
        'workspace_id:"default",created_at:new Date()}, {upsert:true})'
    )


def run_industry(profile_id: str, name: str, description: str, opener: str,
                 app_id: str, account_id: str) -> None:
    # Pre-suite cleanup guarantees no active biztest profile. Delete only this test lineage.
    _lib.mongo(
        f'db.domain_profiles.deleteMany({{profile_id:{json.dumps(profile_id)},is_active:false}})'
    )
    generated = _lib.api_bg(
        "POST", "/api/admin/domain-profiles/generate",
        {"businessDescription": description, "profileId": profile_id, "displayName": name},
        admin=True, max_wait=720, tag=f"profile_{profile_id}",
    )
    error = _lib.is_api_error(generated)
    if error:
        raise RuntimeError(f"{name} profile generation failed: {error} {generated}")
    _lib.assert_llm_success(720, "guide.domain_profile.draft", f"{DOMAIN}/{name}")
    row_id, _ = _lib.domain_profile_identity(generated)
    draft = _profile(row_id)
    _require(
        draft.get("profile_id") == profile_id
        and draft.get("release_status") == "draft"
        and draft.get("current_version") is False
        and draft.get("is_active") is False,
        f"{name} AI 产物是未发布、未激活的 immutable draft", draft,
    )
    _require(draft.get("seeded_by") == "generated_by_ai",
             f"{name} draft 保留 AI 来源", draft.get("seeded_by"))
    keys = _state_keys(draft)
    _require(bool(keys), f"{name} 生成可执行状态机而非静默回落", draft.get("generated_state_machine"))

    published = _lib.api(
        "POST", f"/api/admin/domain-profiles/{row_id}/publish", {}, admin=True, timeout=120,
    )
    _require(_lib.is_api_error(published) is None
             and published.get("status") == "published"
             and published.get("requiresActivation") is True
             and published.get("id") == row_id,
             f"{name} 显式 publish 只移动 current、不自动生效", published)
    after_publish = _profile(row_id)
    _require(after_publish.get("current_version") is True
             and after_publish.get("is_active") is False,
             f"{name} publish 后仍未 activate", after_publish)

    activated = _lib.api(
        "POST", f"/api/admin/domain-profiles/{row_id}/activate", {}, admin=True, timeout=180,
    )
    _require(_lib.is_api_error(activated) is None
             and activated.get("status") == "completed"
             and ((activated.get("steps") or {}).get("stateMachine") or {}).get("status") == "completed",
             f"{name} activate 完整发布状态机及附属投影", activated)
    active = _active_profile()
    _require(_lib.bson_object_id(active.get("_id")) == row_id,
             f"{name} runtime active 精确指向发布版本", active)

    wxid = f"{profile_id}_c"
    _lib.ensure_managed_contact(account_id, wxid, f"biztest {name}客户")
    _lib.reset_contact_conversation(account_id, wxid)
    run = _lib.send_and_wait(app_id, wxid, opener, f"{profile_id}_runtime", max_wait=720)
    _require(isinstance(run, dict) and bool(run.get("run_id")),
             f"{name} 行业对话产生精确 run", run)
    run_id = run["run_id"]
    review = _lib.decision_review_for_run(wxid, run_id)
    operation_state = str(review.get("operation_state", ""))
    _require(operation_state in keys,
             f"{name} operation_state 属于该版本生成状态机", {"state": operation_state, "keys": sorted(keys)})

    if profile_id == "biztest_psych":
        outbox = _lib.outbox_for_run(wxid, run_id)
        _require(bool(outbox), "心理陪伴纯情感回复未被销售 grounding 误拦", outbox)


def main() -> None:
    account_id, app_id = _lib.biztest_account()
    original = _active_profile()
    original_id = _lib.bson_object_id(original.get("_id"))
    _require(
        not original or bool(original_id),
        "切换行业前 active profile 身份可读，或合法回落内置 DEFAULT",
        original,
    )
    # Persist the rollback identity before the first global pointer switch. If this process is
    # killed before `finally`, cleanup will recover either the exact active row or the legal
    # zero-active-profile DEFAULT fallback.
    _write_restore_marker(original_id)
    failure: BaseException | None = None
    try:
        for args in INDUSTRIES:
            print(f"\n===== 行业: {args[1]} =====", flush=True)
            run_industry(*args, app_id, account_id)
    except BaseException as error:
        failure = error
    finally:
        try:
            if original_id:
                _restore_original(original)
            else:
                restored_default = _lib.restore_default_domain_profile_fallback()
                _require(
                    not _active_profile(),
                    "精确恢复零 active profile 的内置 DEFAULT 回落",
                    restored_default,
                )
            _lib.mongo(
                f'db.biztest_control.deleteOne({{_id:{json.dumps(RESTORE_MARKER_ID)}}})'
            )
        except BaseException as restore_error:
            # Keep the marker for cleanup recovery. Never hide a failed runtime rollback.
            if failure is None:
                failure = restore_error
            else:
                print(f"[CRITICAL] 原 profile 恢复同时失败: {restore_error}", flush=True)
    if failure is not None:
        raise failure
    print(f"[{DOMAIN}] 完成：draft→publish→activate✓ 行业状态机✓ 精确恢复✓")


if __name__ == "__main__":
    main()
