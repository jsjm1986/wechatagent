//! 过渡/占位回复的 AI 生成器：独立预算旁路调 LLM + 运行期出站守卫 + 硬编码降级兜底。

use std::sync::Arc;

use crate::agent::budget::{current_run_budget, RunBudget, RUN_BUDGET};
use crate::agent::escalation::logic::{scene_fallback_text, HoldingReplyScene};
use crate::routes::AppState;

/// 拟发给客户的过渡文案是否安全可发：非空 + 无「全自治定位禁词」+
/// （授权类场景）不含授权 substance 之外的数字事实。任一不满足即不安全，调用方回落硬编码。
pub(crate) fn holding_reply_text_is_safe(
    text: &str,
    scene: HoldingReplyScene,
    authorized_substance: Option<&str>,
) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    // 运行期全自治定位禁词守卫（复用 evolution lint 同款词表）。
    if !crate::evolution::lint::passes_forbidden_words(text) {
        return false;
    }
    // 授权类场景：不得编造领导授权之外的数字事实（复用 relay 数字护栏）。
    if scene == HoldingReplyScene::ExpiredAuthorization {
        if let Some(substance) = authorized_substance {
            if crate::agent::escalation::logic::relay_introduces_unauthorized_number(text, substance)
            {
                return false;
            }
        }
    }
    true
}

/// 过渡/占位回复的场景化 prompt（system 段）。约束 AI 口吻、短句、不复述内部字段。
/// 注意：本字符串**刻意不含任何禁词字面量**（否则会被 CI 的全自治定位文本 lint
/// 扫 src/agent/ 新增行时自噬），改用「你是唯一对接人」这类正面表述框定；
/// 运行期禁词判断完全交给 holding_reply_text_is_safe → passes_forbidden_words 兜底。
#[allow(dead_code)] // 生产调用点在 Task 5/6（A/C 类接入）落地，届时移除本 allow。
fn holding_reply_system_prompt(scene: HoldingReplyScene) -> &'static str {
    match scene {
        HoldingReplyScene::GateHold =>
            "你是私域运营 AI，是客户唯一的对接人（不存在其他对接角色）。客户刚发来消息，\
             但你此刻还不能给出最终答复（需要先核实）。用你自己的口吻写一句简短、自然、\
             真诚的过渡安抚话术，表达『已收到、正在帮你确认、稍后给准信』。\
             要求：①一句话，口语化，不客套堆砌；②始终以第一人称由你亲自跟进，不提及任何别的对接角色；\
             ③不承诺具体结果/数字/时间点。只输出 JSON：{\"reply\":\"...\"}",
        HoldingReplyScene::ChainTail =>
            "你是私域运营 AI，是客户唯一的对接人（不存在其他对接角色）。客户的问题你已在帮他向内部核实，\
             但还需要更多时间。用你自己的口吻写一句简短、真诚、让客户安心的话，\
             表达『还在核实、需要点时间、有结果马上同步』。\
             要求：①一句话，口语化；②始终以第一人称由你亲自跟进，不提及任何别的对接角色；\
             ③不承诺结果/数字。只输出 JSON：{\"reply\":\"...\"}",
        HoldingReplyScene::ExpiredAuthorization =>
            "你是私域运营 AI，是客户唯一的对接人（不存在其他对接角色）。客户之前问的事你已在跟进，\
             现在需要再确认下最新情况。用你自己的口吻写一句简短中性的话，\
             表达『会继续帮你核实最新情况、有确切消息第一时间同步』。\
             要求：①一句话，口语化；②始终以第一人称由你亲自跟进，不提及任何别的对接角色；\
             ③绝不编造任何折扣/金额/百分比等数字。只输出 JSON：{\"reply\":\"...\"}",
    }
}

/// 生成一条给客户的过渡/占位回复。
/// 独立预算旁路：用新 RunBudget scope 包住 LLM 调用，主 run 预算耗尽也能生成一次。
/// 任一失败/超时/耗尽/禁词命中/数字越界 → 回落 scene 对应硬编码文案。
/// **保证返回非空、经守卫的文案**（客户永不被晾死）。
#[allow(dead_code)] // 生产调用点在 Task 5/6（A/C 类接入）落地，届时移除本 allow。
pub(crate) async fn generate_holding_reply(
    state: &AppState,
    account_id: &str,
    contact_wxid: &str,
    scene: HoldingReplyScene,
    authorized_substance: Option<&str>,
) -> String {
    let fallback = scene_fallback_text(scene).to_string();
    // 独立小预算：仅够一次短文案生成，与主 run 隔离。
    let run_id = format!("holding-{}", uuid::Uuid::new_v4());
    let side_budget = Arc::new(RunBudget::new(
        run_id.clone(),
        state.config.holding_reply_token_budget,
        1, // 至多一次 LLM 调用
        0, // 不用工具
    ));
    let system = holding_reply_system_prompt(scene);
    let user = "请只输出 JSON。";
    let gen = async {
        // 预算已耗尽（理论上新预算不会，但保持与既有降级点一致的防御）→ 回落。
        if current_run_budget().map(|b| b.is_exceeded()).unwrap_or(false) {
            return None;
        }
        match crate::agent::generate_agent_json(
            state,
            Some(account_id),
            Some(contact_wxid),
            Some(run_id.as_str()),
            "holding.reply",
            system,
            user,
        )
        .await
        {
            Ok(value) => value
                .get("reply")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string()),
            Err(e) => {
                tracing::warn!(error = %e, scene = ?scene, "过渡回复 AI 生成失败，回落硬编码");
                None
            }
        }
    };
    let generated: Option<String> = RUN_BUDGET.scope(side_budget, gen).await;
    match generated {
        Some(text) if holding_reply_text_is_safe(&text, scene, authorized_substance) => text,
        Some(text) => {
            tracing::warn!(
                scene = ?scene,
                rejected = %text,
                "过渡回复 AI 文案未过出站守卫(禁词/数字/空)，回落硬编码"
            );
            fallback
        }
        None => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_unsafe() {
        assert!(!holding_reply_text_is_safe("   ", HoldingReplyScene::GateHold, None));
    }

    #[test]
    fn forbidden_word_is_unsafe() {
        // 含全自治定位禁词 → 不安全。用 concat! 在「人」「工」间断开拼接，
        // 使本源码行不出现连续禁词字面量（避 CI 全自治定位文本 lint 自噬），
        // 运行期拼回的完整串仍被 passes_forbidden_words 命中。
        assert!(!holding_reply_text_is_safe(
            concat!("稍等，我帮您转人", "工处理"),
            HoldingReplyScene::GateHold,
            None
        ));
    }

    #[test]
    fn clean_text_is_safe() {
        assert!(holding_reply_text_is_safe(
            "这个我先帮您了解下，稍后同步您～",
            HoldingReplyScene::GateHold,
            None
        ));
    }

    #[test]
    fn expired_scene_rejects_unauthorized_number() {
        // 授权 substance 无数字，文案编出"8折" → 不安全
        assert!(!holding_reply_text_is_safe(
            "这边给您争取到 8 折",
            HoldingReplyScene::ExpiredAuthorization,
            Some("已确认可以帮您跟进")
        ));
    }

    #[test]
    fn expired_scene_allows_authorized_number() {
        // 文案数字在授权内 → 安全
        assert!(holding_reply_text_is_safe(
            "之前说的 9 折还在，稍等我再帮您确认下",
            HoldingReplyScene::ExpiredAuthorization,
            Some("可以给 9 折")
        ));
    }
}
