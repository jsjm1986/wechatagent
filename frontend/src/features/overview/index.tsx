import { useEffect } from "react";
import { RefreshCw, ArrowRight } from "lucide-react";
import { Avatar } from "../../components/ui/Avatar";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { EmptyState } from "../../components/ui/EmptyState";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import { useNavigationStore } from "../../stores/navigationStore";
import { useProfileStore, labelFor } from "../../stores/profileStore";
import type { Contact } from "../../types";
import styles from "./Overview.module.css";

const STATE_LABEL: Record<string, string> = {
  managed: "自主运营中",
  normal: "未托管",
};

function contactTone(contact: Contact): StatusTone {
  if (contact.agentStatus !== "managed") return "inactive";
  if (contact.cooldownUntil) return "held";
  return "running";
}

function contactStateLabel(contact: Contact): { tone: StatusTone; label: string } {
  if (contact.agentStatus !== "managed") return { tone: "inactive", label: "未托管" };
  if (contact.cooldownUntil) return { tone: "held", label: "AI 策略暂缓" };
  return { tone: "running", label: "自主回复" };
}

export default function OverviewFeature() {
  const contacts = useContactStore((s) => s.contacts);
  const dataAccountId = useContactStore((s) => s.dataAccountId);
  const loadContacts = useContactStore((s) => s.loadContacts);
  const managedCount = useContactStore((s) => s.managedCount());
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const normalCount = useContactStore((s) => s.normalCount());
  const onlineCount = useAccountStore((s) => s.onlineCount());
  const accountCount = useAccountStore((s) => s.accounts.length);
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const setChannel = useNavigationStore((s) => s.setChannel);

  const scopedContacts = dataAccountId === currentAccountId ? contacts : [];

  // 工作台只消费当前账号快照。scope 不一致时立即以空态渲染，并静默加载当前账号；
  // 迟到响应由 contactStore 的 generation 守卫丢弃。
  useEffect(() => {
    if (!currentAccountId) return;
    if (useContactStore.getState().dataAccountId === currentAccountId) return;
    void loadContacts(currentAccountId, undefined, { silent: true });
  }, [currentAccountId, loadContacts]);

  const managed = scopedContacts.filter((c) => c.agentStatus === "managed");
  const deliveryRate = scopedContacts.length > 0 ? Math.round((managedCount / scopedContacts.length) * 100) : 0;
  const now = new Date();
  const clock = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  const liveContacts = managed.slice(0, 5);

  return (
    <div className={styles.page}>
      <div className={styles.statsHead}>
        <span className={styles.statsTitle}>运营态势</span>
        <div className={styles.clock}><span className={styles.d} />实时更新 · {clock}</div>
      </div>

      <div className={styles.stats}>
        <button className={styles.stat} onClick={() => setChannel("userOps")}>
          <div className={styles.sh}><div className={styles.k}>托管联系人</div><div className={styles.badge}>累计</div></div>
          <div className={styles.v}>{managedCount}</div>
          <div className={`${styles.t} ${styles.up}`}>共 {scopedContacts.length} 位联系人 <span className={styles.mut}>· {normalCount} 普通</span></div>
        </button>
        <button className={`${styles.stat} ${styles.key}`} onClick={() => setChannel("userOps")}>
          <div className={styles.sh}><div className={styles.k}>托管覆盖率</div><div className={styles.badge}>实时</div></div>
          <div className={styles.v}>{deliveryRate}<small>%</small></div>
          <div className={`${styles.t} ${styles.up}`}>{managedCount} / {scopedContacts.length} <span className={styles.mut}>已纳入运营</span></div>
        </button>
        <button className={styles.stat} onClick={() => setChannel("overview")}>
          <div className={styles.sh}><div className={styles.k}>在线账号</div><div className={styles.badge}>MCP</div></div>
          <div className={styles.v}>{onlineCount}<small>/{accountCount}</small></div>
          <div className={styles.spark}>
            <i style={{ height: "32%" }} /><i style={{ height: "52%" }} /><i style={{ height: "44%" }} />
            <i style={{ height: "78%" }} /><i style={{ height: "60%" }} /><i style={{ height: "72%" }} /><i style={{ height: "100%" }} />
          </div>
        </button>
      </div>

      <div className={styles.panel}>
        <div className={styles.ph}>
          <div className={styles.pl}>
            <b>实时运营流</b>
            <span className={styles.pc}>{managedCount} 位托管联系人</span>
          </div>
          <span className={styles.chip}><span className={styles.d} /><span className={styles.txt}>AI 自主运行中</span></span>
        </div>
        {liveContacts.length === 0 ? (
          <EmptyState title="暂无托管联系人" hint="到「用户运营」导入好友并开启自主运营" />
        ) : (
          liveContacts.map((c) => {
            const { tone, label } = contactStateLabel(c);
            const name = c.remark || c.nickname || c.wxid;
            return (
              <div key={c.id} className={styles.item}>
                <Avatar name={name} tone={contactTone(c)} live={tone === "running"} />
                <div className={styles.itxt}>
                  <div className={styles.n}>
                    {name}
                    {c.operationState && <em>{labelFor(taxonomies, "customer_stage", c.operationState).text}</em>}
                  </div>
                  <div className={styles.s}>
                    {c.memorySummary || c.humanProfileNote || "尚无运营备注"}
                    {c.lastMessageAt && <span className={styles.meta}>· {STATE_LABEL[c.agentStatus] ?? c.agentStatus}</span>}
                  </div>
                </div>
                <div className={styles.iend}>
                  <StatusBadge tone={tone}>{label}</StatusBadge>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
