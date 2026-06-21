// Ask-Human Phase 2 Task 8：单 DomainProfile 的「发布 / 激活」处置卡。源自
// system-strategy ProfileEditor 表单按钮行（发布/激活/灰度）。中立化到 components/review/
// 后，老页（system-strategy）与统一收件箱频道都从这里 import，发布/激活逻辑不再各持一份。
//
// 解耦 strategyStore（用户裁定 B 的核心）：本卡片不 import stores/strategyStore，直调
// api 同一 RAW 端点（publish 的 RAW 返回 {ok,pendingActivation?,riskyFields?,id} 见
// strategyStore.ts:355 实证）。零跨 feature import：只依赖 react/lib/api/ui providers。
//
// 高风险发布两段式（不可省）：publish 返 pendingActivation=true（改了高风险开关，新版本
// 定稿但未生效）→ useConfirm 模态二次确认 → POST /rollout 才真正生效；普通字段发布即生效。
import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useConfirm } from "../ui/ConfirmDialog";
import { useToast } from "../ui/Toast";

// 字段子集（DomainProfile 的 snake_case，types/index.ts:521 实证）。只取卡片渲染 + 动作门控需要的。
interface ProfileLite {
  id: string;
  display_name?: string;
  is_active?: boolean;
  current_version?: boolean;
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
      // 单 profile 无专用 GET：拉列表后按 id 过滤（与老页 loadDomainProfiles 一致）。
      const { items } = await api.get<{ items: ProfileLite[] }>("/api/admin/domain-profiles");
      setProfile(items.find((p) => p.id === profileId) ?? null);
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
      body:
        "普通字段（名称/简介/业务上下文等）发布后即时生效；若改动了高风险开关" +
        "（人格本体/方法论/风控阈值/自学习极性等），会要求二次确认后才生效。",
      confirmText: "确认发布",
    });
    if (!ok) return;
    setBusy(true);
    try {
      const res = await api.post<{ pendingActivation?: boolean; riskyFields?: string[] }>(
        `/api/admin/domain-profiles/${encodeURIComponent(profileId)}/publish`,
        {},
      );
      if (res.pendingActivation) {
        const fields =
          res.riskyFields && res.riskyFields.length > 0 ? res.riskyFields.join("、") : "（未知字段）";
        const proceed = await confirm({
          title: "高风险字段改动，确认立即生效？",
          body: `本次改动涉及高风险字段：${fields}。新版本已定稿但尚未生效，当前仍运行旧版本。`,
          tone: "danger",
          confirmText: "确认生效",
        });
        if (!proceed) {
          // 已定稿未生效；不 rollout，刷新展示让管理员看到「待激活」态。
          toast.info("新版本已定稿，未生效");
          onDone?.();
          await load();
          return;
        }
        await api.post(`/api/admin/domain-profiles/${encodeURIComponent(profileId)}/rollout`, {});
      }
      toast.success("已发布");
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
      await api.post(`/api/admin/domain-profiles/${encodeURIComponent(profileId)}/activate`, {});
      toast.success("已激活");
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

  return (
    <div className="profilePublishCard">
      <div className="profilePublishName">{profile.display_name ?? profileId}</div>
      <div className="profilePublishStatus">
        {profile.is_active ? "已激活" : profile.current_version ? "待激活" : "草稿"}
      </div>
      <div className="profilePublishActions">
        {!profile.is_active && !profile.current_version && (
          <button type="button" disabled={busy} onClick={() => void publish()}>
            发布
          </button>
        )}
        {!profile.is_active && profile.current_version && (
          <button type="button" disabled={busy} onClick={() => void activate()}>
            激活生效
          </button>
        )}
      </div>
    </div>
  );
}
