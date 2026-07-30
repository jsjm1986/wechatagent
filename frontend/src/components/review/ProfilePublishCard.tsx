// Ask-Human Phase 2 Task 8：单 DomainProfile 的「发布 / 激活」处置卡。源自
// system-strategy ProfileEditor 表单按钮行（发布/激活/灰度）。中立化到 components/review/
// 后，老页（system-strategy）与统一收件箱频道都从这里 import，发布/激活逻辑不再各持一份。
//
// 解耦 strategyStore（用户裁定 B 的核心）：本卡片不 import stores/strategyStore，直调
// 发布与激活严格分离：publish 只把 draft 变成 published current，所有字段都必须再经
// 管理员显式 activate 后才进入运行时。activate 的核心指针先提交，附属同步失败会返回 partial，
// 卡片保留幂等重试入口。
import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { GeneratedStateMachine } from "../../types";
import { useConfirm } from "../ui/ConfirmDialog";
import { useToast } from "../ui/Toast";

// 字段子集（DomainProfile 的 snake_case，types/index.ts:521 实证）。只取卡片渲染 + 动作门控需要的。
interface ProfileLite {
  id: string;
  display_name?: string;
  is_active?: boolean;
  current_version?: boolean;
  release_status?: "draft" | "published";
  // H13：AI 生成状态机本体（draft）。外层 snake_case，内层 key camelCase（绕过 normalize_json_keys）。
  // 激活前供管理员审阅 states/goal/advanceSignals/riskRules（AI 不自我核验、审阅后才激活）。
  generated_state_machine?: GeneratedStateMachine | null;
}

export function ProfilePublishCard({
  profileId,
  onDone,
}: {
  profileId: string;
  onDone?: () => void;
}) {
  const confirm = useConfirm();
  const toast = useToast();
  const [profile, setProfile] = useState<ProfileLite | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      const { item } = await api.get<{ item: ProfileLite }>(
        `/api/admin/domain-profiles/${encodeURIComponent(profileId)}`,
      );
      setProfile(item);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [profileId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function publish() {
    const ok = await confirm({
      title: "确认发布此版本？",
      body: "发布后进入待激活状态，当前运行中的行业配置不会改变。请审阅发布结果后再单独激活。",
      confirmText: "确认发布",
    });
    if (!ok) return;
    setBusy(true);
    try {
      await api.post<{ riskyFields?: string[] }>(
        `/api/admin/domain-profiles/${encodeURIComponent(profileId)}/publish`,
        {},
      );
      toast.success("已发布，等待管理员激活");
      onDone?.();
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function activate() {
    const ok = await confirm({
      title: "确认激活此行业配置？",
      body: "激活后所有 AI 对话将使用此配置。",
      confirmText: "确认激活",
    });
    if (!ok) return;
    setBusy(true);
    try {
      const result = await api.post<{
        status: "completed" | "partial";
        retryable?: boolean;
        errors?: Array<{ step?: string; message?: string }>;
      }>(`/api/admin/domain-profiles/${encodeURIComponent(profileId)}/activate`, {});
      if (result.status === "partial") {
        const failed = (result.errors ?? [])
          .map((item) => item.step)
          .filter((step): step is string => Boolean(step))
          .join("、");
        toast.error(
          `行业配置核心已激活，但附属同步未完成${failed ? `（${failed}）` : ""}。请点击“重试附属同步”。`,
        );
      } else {
        toast.success("已完整激活");
      }
      onDone?.();
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (error) return <div className="profilePublishError">加载失败：{error}</div>;
  if (!profile) return <div className="profilePublishLoading">加载中…</div>;

  const states = profile.generated_state_machine?.states ?? [];
  const releaseStatus = profile.release_status ?? "published";
  const statusText = profile.is_active
    ? profile.current_version
      ? "生效中"
      : "生效中（旧发布）"
    : releaseStatus === "draft"
      ? "草稿"
      : profile.current_version
        ? "已发布 · 待激活"
        : "已发布历史";

  return (
    <div className="profilePublishCard">
      <div className="profilePublishName">{profile.display_name ?? profileId}</div>
      <div className="profilePublishStatus">
        {statusText}
      </div>
      {states.length > 0 && (
        <div className="profilePublishStateMachine">
          <div className="profilePublishStateMachineTitle">状态机（激活前审阅）</div>
          <ul className="profilePublishStateList">
            {states.map((s, i) => (
              <li key={s.key ?? `state-${i}`} className="profilePublishState">
                <div className="profilePublishStateHead">
                  {s.name && <span className="profilePublishStateName">{s.name}</span>}
                  {s.key && <code className="profilePublishStateKey">{s.key}</code>}
                  {s.initial && <span className="profilePublishStateInitial">初始态</span>}
                </div>
                {s.goal && (
                  <div className="profilePublishStateGoal">
                    目标：<span className="profilePublishGoalText">{s.goal}</span>
                  </div>
                )}
                {s.advanceSignals && s.advanceSignals.length > 0 && (
                  <div className="profilePublishStateSignals">
                    推进信号：
                    {s.advanceSignals.map((sig, j) => (
                      <span key={`adv-${i}-${j}`} className="profilePublishTag">
                        {sig}
                      </span>
                    ))}
                  </div>
                )}
                {s.riskRules && s.riskRules.length > 0 && (
                  <div className="profilePublishStateRisks">
                    风控规则：
                    {s.riskRules.map((rule, j) => (
                      <span key={`risk-${i}-${j}`} className="profilePublishTag">
                        {rule}
                      </span>
                    ))}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="profilePublishActions">
        {releaseStatus === "draft" && !profile.is_active && (
          <button type="button" disabled={busy} onClick={() => void publish()}>
            发布
          </button>
        )}
        {releaseStatus === "published" && profile.current_version && !profile.is_active && (
          <button type="button" disabled={busy} onClick={() => void activate()}>
            激活生效
          </button>
        )}
        {releaseStatus === "published" && profile.current_version && profile.is_active && (
          <button type="button" disabled={busy} onClick={() => void activate()}>
            重试附属同步
          </button>
        )}
      </div>
    </div>
  );
}
