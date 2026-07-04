import { useState } from "react";
import { api } from "../../lib/api";
import { useCampaignStore } from "../../stores/campaignStore";
import { useUiStore } from "../../stores/uiStore";
import { ProductMultiSelect } from "./ProductMultiSelect";
import { StageSelect } from "./StageSelect";
import styles from "./Campaign.module.css";

interface PreviewResult { campaignId: string; targetCount: number; samples: { wxid: string; name: string }[]; }

export default function CampaignCreate() {
  const setView = useCampaignStore((s) => s.setView);
  const openReport = useCampaignStore((s) => s.openReport);
  const setError = useUiStore((s) => s.setError);

  const [title, setTitle] = useState("");
  const [intentText, setIntentText] = useState("");
  const [productIds, setProductIds] = useState<string[]>([]);
  const [customerStage, setCustomerStage] = useState("");
  const [aftercare, setAftercare] = useState("");
  const [valueTier, setValueTier] = useState("");
  const [draftCampaignId, setDraftCampaignId] = useState<string | null>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [busy, setBusy] = useState(false);

  const canPreview = title.trim() !== "" && intentText.trim() !== "" && !busy;

  const segmentFilter = () => {
    const f: Record<string, unknown> = {};
    if (productIds.length) f.productIds = productIds;
    if (aftercare) f.aftercare = aftercare;
    if (valueTier) f.valueTier = valueTier;
    if (customerStage) f.customerStage = customerStage;
    return f;
  };

  const handlePreview = async () => {
    if (!canPreview) return;
    setBusy(true);
    try {
      let id = draftCampaignId;
      if (!id) {
        const created = await api.post<{ id: string }>("/api/campaigns", {
          title: title.trim(), intentText: intentText.trim(), segmentFilter: segmentFilter(),
        });
        id = created.id;
        setDraftCampaignId(id);
      }
      const r = await api.post<PreviewResult>(`/api/campaigns/${id}/preview`, {});
      setPreview(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // 改任一条件 → 作废旧 preview（但保留 draftCampaignId 复用），下次预览用同一 draft 重新圈人
  const onCondChange = <T,>(setter: (v: T) => void) => (v: T) => { setter(v); setPreview(null); };

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>New Campaign</span>
            <span className={styles.title}>新建活动</span>
          </div>
          <button type="button" className={styles.pagerBtn} onClick={() => setView("list")}>返回列表</button>
        </div>

        <div className={styles.form}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>活动标题</span>
            <input className={styles.input} value={title} onChange={(e) => setTitle(e.target.value)} placeholder="如：双11老客续费7折" />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>活动意图</span>
            <textarea className={styles.textarea} value={intentText} onChange={(e) => setIntentText(e.target.value)} placeholder="活动要点，将作为给客户推送的语境，由 AI 据各自画像生成个性化话术" />
          </label>

          <div className={styles.fieldLabel}>圈人条件（各项可选，留空即不限）</div>
          <label className={styles.field}>
            <span className={styles.fieldSub}>买过的产品</span>
            <ProductMultiSelect value={productIds} onChange={onCondChange(setProductIds)} />
          </label>
          <div className={styles.fieldRow}>
            <label className={styles.field}>
              <span className={styles.fieldSub}>客户阶段</span>
              <StageSelect value={customerStage} onChange={onCondChange(setCustomerStage)} />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldSub}>售后状态</span>
              <select className={styles.select} value={aftercare} onChange={(e) => onCondChange(setAftercare)(e.target.value)}>
                <option value="">不限</option>
                <option value="in_aftercare">售后中</option>
                <option value="expired">已到期</option>
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.fieldSub}>价值分层</span>
              <select className={styles.select} value={valueTier} onChange={(e) => onCondChange(setValueTier)(e.target.value)}>
                <option value="">不限</option>
                <option value="high">高</option>
                <option value="mid">中</option>
                <option value="low">低</option>
              </select>
            </label>
          </div>

          <button type="button" className={styles.primaryBtn} disabled={!canPreview} onClick={handlePreview}>
            {busy ? "圈人中…" : "圈人预览"}
          </button>
        </div>
      </section>

      {preview && (
        <section className={styles.panel}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>Preview</span>
              <span className={styles.title}>圈人预览：命中 {preview.targetCount} 人</span>
            </div>
          </div>
          <p className={styles.previewNote}>实际推送时会重新圈选，人数可能微调。</p>
          {preview.samples.length > 0 && (
            <div className={styles.samples}>
              {preview.samples.map((s) => (
                <span key={s.wxid} className={styles.sampleChip}>{s.name || s.wxid}</span>
              ))}
            </div>
          )}
          {preview.targetCount === 0 && <p className={styles.fieldHint}>命中 0 人，调整条件再试。</p>}
          <div className={styles.previewActions}>
            <p className={styles.dispatchHint}>确认推送请在 AI 总控对话中对该活动下发推送（高风险动作由 AI 恒确认门把关）。</p>
            <div className={styles.previewBtns}>
              <button type="button" className={styles.pagerBtn} onClick={() => setView("list")}>返回列表</button>
              {draftCampaignId && (
                <button type="button" className={styles.exportBtn} onClick={() => openReport(draftCampaignId)}>查看结果看板</button>
              )}
            </div>
          </div>
        </section>
      )}
    </div>
  );
}
