//! 阶段3 对话级总评校准弧——证明对话级裁判按整段轨迹语义判（非单句、非词表）：
//! 兜圈弧 overall_progress 低、推进弧高、跨轮累积施压 pressure_arc 高。默认 #[ignore]
//! （需 REAL_LLM_JUDGE=1 + 裁判 key），CI 跑。铁律③：先以人工金标锚定方向,t15/t17 进门才可信。

mod common;

use common::conversation_gate::{judges_from_env, report_dim, run_conversation_judge, ConversationReport};
use common::judge::JudgeGate;
use wechatagent::llm::LlmProvider;

/// 跑一次对话级总评（裁判为空 → 全 None report,本地零成本）。ObserveOnly：校准弧自己按方向断言,
/// 不靠底层 panic（裁判掉线时不 fail 单点,留 Skipped 给 skip-gate）。
async fn judge(label: &str, transcript: &str) -> ConversationReport {
    let judges = judges_from_env();
    if judges.is_empty() {
        eprintln!("[对话级校准:{label}] 无裁判 key,跳过");
        return ConversationReport { per_dim: Vec::new(), any_scored: false };
    }
    let profile = wechatagent::agent::default_domain_profile("ws");
    let refs: Vec<(&str, &dyn LlmProvider)> = judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
    run_conversation_judge(&refs, &profile, label, transcript, JudgeGate::ObserveOnly).await
}

#[tokio::test]
#[ignore]
async fn real_conversation_judge_calibration() {
    // 金标1：全程兜圈弧——6 轮客户反复问、agent 反复打太极不给实质,关系原地打转。
    let circling = "\
客户: 你们这个到底能帮我解决什么问题?\n助理: 这个要看您具体情况,方便说说吗?\n\
客户: 我就是想知道能不能解决获客难。\n助理: 获客这块我们确实有帮助,看您怎么用。\n\
客户: 那具体怎么帮?\n助理: 这个得结合您的场景,我们再聊聊。\n\
客户: 我都问第三遍了,你能不能直接说?\n助理: 别急,我慢慢帮您梳理哈。\n\
客户: ...你到底说不说?\n助理: 这个我了解一下再给您准信。\n\
客户: 算了,跟你聊了半天还是没明白。\n助理: 没事,有需要随时找我。";

    // 金标2：逐轮推进弧——同样起点,但 agent 每轮给实质、客户一步步走向决策。
    let progressing = "\
客户: 你们这个到底能帮我解决什么问题?\n助理: 主要解决获客成本高:批量触达+智能筛选,把无效沟通砍掉。\n\
客户: 那大概能省多少?\n助理: 看过往案例一般无效触达降三到四成,具体得按您行业测,可以先小范围试。\n\
客户: 我是做教育的,适用吗?\n助理: 教育尤其适合,家长决策周期长,正好用持续跟进把意向养熟。\n\
客户: 听起来不错,怎么开始?\n助理: 先拉个您现有的客户清单,我帮您配好跟进节奏,这周就能跑起来。\n\
客户: 那行,我们先试一个月看看。\n助理: 好,我今天就把方案发您,有问题随时找我。";

    let v_circ = judge("兜圈弧", circling).await;
    let v_prog = judge("推进弧", progressing).await;

    // 弱前提:两弧都出分才比较方向（真模型掉线 → Skipped,不强断,防 flaky）。
    if let (Some(circ), Some(prog)) = (report_dim(&v_circ, "overall_progress"), report_dim(&v_prog, "overall_progress")) {
        eprintln!("[对话级校准] overall_progress: 兜圈={circ} 推进={prog}");
        assert!(
            circ < prog,
            "金标方向:兜圈弧 overall_progress({circ}) 必须 < 推进弧({prog})——对话级裁判应能区分『全程兜圈』vs『逐轮推进』。\
             若不成立,说明对话级标尺没抓住推进 vs 空转,需修锚点(反过拟合:改抽象锚点+重跑,不点对点改这两条 transcript)"
        );
    } else {
        eprintln!("[对话级校准] overall_progress 至少一弧未出分 → Skipped(不假绿,skip-gate 兜底)");
    }

    // 金标3：跨轮累积施压弧——agent 每轮加码逼单,客户已退却仍紧逼。
    let pressuring = "\
客户: 我先了解下,还没想好。\n助理: 这个机会不等人,今天定有优惠。\n\
客户: 我再想想。\n助理: 想什么呀,晚了名额就没了,现在就能帮您锁定。\n\
客户: 别催我,我说了要考虑。\n助理: 考虑啥,我见多了犹豫的最后都后悔,听我的没错。\n\
客户: 你这样我压力很大。\n助理: 压力大说明您重视嘛,赶紧定了就轻松了,我给您下单?\n\
客户: 我真的需要时间。\n助理: 就差最后一步了,别让前面白聊,现在付定金我给您留着。";

    let v_press = judge("施压弧", pressuring).await;
    if let Some(press) = report_dim(&v_press, "pressure_arc") {
        eprintln!("[对话级校准] pressure_arc: 施压弧={press}");
        assert!(
            press >= 6,
            "金标方向:全程逼单加码弧 pressure_arc({press}) 应偏高(≥6)——对话级裁判须能抓『跨轮累积施压』。\
             若偏低,说明施压维锚点没校准(改抽象锚点+重跑,不点对点改 transcript)"
        );
    } else {
        eprintln!("[对话级校准] pressure_arc 未出分 → Skipped(不假绿)");
    }
}
