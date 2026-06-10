import { useState, useEffect, useCallback, useMemo, Fragment } from "react";
import {
  Activity,
  ArrowRight,
  CheckCircle2,
  LibraryBig,
  RefreshCw,
  ShieldCheck,
  Undo2,
  Workflow,
} from "lucide-react";
import { parseApiError } from "../../lib/api";
import { useConfirm } from "../../components/ui/ConfirmDialog";
import { focusChunk, type TreeChunkItem } from "./shared";

// ── P1 · ChunkGraphView · 关系图谱（SVG 原生布局，0 新依赖）─────────────
//
// 数据：GET /api/operation-knowledge/chunks → 每条 chunk 一个节点。
// 边来源：
//   - relatedChunks: { chunk_id, kind } 6 类（references / requires / contradicts / clarifies / refines / superseded_by）
//   - supersededBy:  归档链尾 → 现役新版本（隐式 superseded_by）
//   - previousVersionId: split/merge/rollback 维护的前一版指针
//
// 两种布局模式：
//   1) polar（确定性）：按 wikiType 分扇区，扇区内按 id 哈希分布角度，
//      半径按"被引用次数" 微调（核心 chunk 向中心收）。0 抖动。
//   2) force（力导向）：以 polar 为初始解 → 200 步 spring + 排斥力迭代，
//      时间步逐步降温（dt *= 0.99）。同步算完一次 setLayout，无每帧重排。
//
// 颜色：默认按 wikiType；切到"社区"模式时用并查集找连通分量，按 component
// 索引分配 HSL 等距色环。
//
// 交互：节点 click → focusChunk(id)；hover → 浮窗显示 title + wikiType。
export function ChunkGraphView() {
  const [items, setItems] = useState<TreeChunkItem[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>("all"); // wikiType filter
  const [layoutMode, setLayoutMode] = useState<"polar" | "force">("polar");
  const [colorMode, setColorMode] = useState<"wikiType" | "community">("wikiType");

  useEffect(() => {
    setLoading(true);
    fetch("/api/operation-knowledge/chunks")
      .then(async (r) => {
        if (!r.ok) throw await parseApiError(r);
        return r.json() as Promise<{ items: TreeChunkItem[] }>;
      })
      .then((data) => setItems(data.items ?? []))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, []);

  const wikiTypes = useMemo(() => {
    if (!items) return [] as string[];
    const set = new Set<string>();
    for (const it of items) if (it.wikiType) set.add(it.wikiType);
    return Array.from(set).sort();
  }, [items]);

  const visible = useMemo(() => {
    if (!items) return [] as TreeChunkItem[];
    if (filter === "all") return items;
    return items.filter((it) => it.wikiType === filter);
  }, [items, filter]);

  const indexById = useMemo(() => {
    const m = new Map<string, TreeChunkItem>();
    for (const it of visible) m.set(it.id, it);
    return m;
  }, [visible]);

  // FNV-1a 32-bit：deterministic id → 0..1 用作角度噪声。
  const hash01 = (s: string): number => {
    let h = 0x811c9dc5;
    for (let i = 0; i < s.length; i += 1) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    return ((h >>> 0) % 100000) / 100000;
  };

  // 入度（被引用次数）— 半径压缩量。
  const inDegree = useMemo(() => {
    const m = new Map<string, number>();
    for (const it of visible) {
      if (it.relatedChunks) {
        for (const r of it.relatedChunks) {
          m.set(r.chunk_id, (m.get(r.chunk_id) ?? 0) + 1);
        }
      }
      if (it.supersededBy) m.set(it.supersededBy, (m.get(it.supersededBy) ?? 0) + 1);
      if (it.previousVersionId) m.set(it.previousVersionId, (m.get(it.previousVersionId) ?? 0) + 1);
    }
    return m;
  }, [visible]);

  // 边渲染：按 kind 决定线条样式。
  type Edge = { from: string; to: string; kind: string };
  const edges: Edge[] = useMemo(() => {
    const out: Edge[] = [];
    const idSet = new Set(visible.map((it) => it.id));
    for (const it of visible) {
      if (it.relatedChunks) {
        for (const r of it.relatedChunks) {
          if (idSet.has(r.chunk_id)) out.push({ from: it.id, to: r.chunk_id, kind: r.kind });
        }
      }
      if (it.supersededBy && idSet.has(it.supersededBy)) {
        out.push({ from: it.id, to: it.supersededBy, kind: "superseded_by" });
      }
      if (it.previousVersionId && idSet.has(it.previousVersionId)) {
        out.push({ from: it.id, to: it.previousVersionId, kind: "previous_version" });
      }
    }
    return out;
  }, [visible]);

  // 社区检测：把节点 + 边喂并查集，输出 nodeId → componentIdx。
  // 同 component 节点同色（用于"社区染色"模式）。
  const community = useMemo(() => {
    const parent = new Map<string, string>();
    const find = (x: string): string => {
      let p = parent.get(x) ?? x;
      if (p === x) return x;
      const root = find(p);
      parent.set(x, root);
      return root;
    };
    const union = (a: string, b: string) => {
      const ra = find(a);
      const rb = find(b);
      if (ra !== rb) parent.set(ra, rb);
    };
    for (const it of visible) parent.set(it.id, it.id);
    for (const e of edges) {
      if (parent.has(e.from) && parent.has(e.to)) union(e.from, e.to);
    }
    const idxByRoot = new Map<string, number>();
    const result = new Map<string, number>();
    for (const it of visible) {
      const root = find(it.id);
      let idx = idxByRoot.get(root);
      if (idx === undefined) {
        idx = idxByRoot.size;
        idxByRoot.set(root, idx);
      }
      result.set(it.id, idx);
    }
    return { byId: result, count: idxByRoot.size };
  }, [visible, edges]);

  // 计算节点坐标。layoutMode 决定 polar / force。
  const layout = useMemo(() => {
    const W = 720;
    const H = 560;
    const cx = W / 2;
    const cy = H / 2;
    const types = wikiTypes.length ? wikiTypes : ["__none"];
    const sectorByType = new Map<string, number>();
    types.forEach((t, i) => sectorByType.set(t, i));
    const sectorWidth = (Math.PI * 2) / types.length;
    const positions = new Map<string, { x: number; y: number }>();
    for (const it of visible) {
      const t = it.wikiType ?? "__none";
      const sector = sectorByType.get(t) ?? 0;
      const noise = hash01(it.id);
      const angle = sector * sectorWidth + noise * sectorWidth * 0.92 + sectorWidth * 0.04;
      const deg = inDegree.get(it.id) ?? 0;
      const radius = 230 - Math.min(deg, 8) * 18;
      positions.set(it.id, {
        x: cx + Math.cos(angle) * radius,
        y: cy + Math.sin(angle) * radius
      });
    }

    if (layoutMode === "force" && visible.length > 0) {
      // 200 步弹簧 + 排斥力迭代。常量在 100-500 节点规模上调过：
      //   k_spring = 0.06     边 → 拉近
      //   rest_len = 80       边目标长度
      //   k_repel  = 1400     节点间 → 推开
      //   dt0      = 0.5      每步位移系数；逐步退火 dt *= 0.99
      //   bounds   = 边界回拉，避免节点飞出 viewBox
      const ids = visible.map((it) => it.id);
      const adj = new Map<string, Set<string>>();
      for (const id of ids) adj.set(id, new Set());
      for (const e of edges) {
        adj.get(e.from)?.add(e.to);
        adj.get(e.to)?.add(e.from);
      }
      const k_spring = 0.06;
      const rest_len = 80;
      const k_repel = 1400;
      let dt = 0.5;
      const padding = 40;
      for (let step = 0; step < 200; step += 1) {
        const fx = new Map<string, number>();
        const fy = new Map<string, number>();
        for (const id of ids) {
          fx.set(id, 0);
          fy.set(id, 0);
        }
        // 排斥力 O(N²)
        for (let i = 0; i < ids.length; i += 1) {
          const a = positions.get(ids[i])!;
          for (let j = i + 1; j < ids.length; j += 1) {
            const b = positions.get(ids[j])!;
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            let dist2 = dx * dx + dy * dy;
            if (dist2 < 1) dist2 = 1;
            const dist = Math.sqrt(dist2);
            const force = k_repel / dist2;
            dx /= dist;
            dy /= dist;
            fx.set(ids[i], (fx.get(ids[i]) ?? 0) + dx * force);
            fy.set(ids[i], (fy.get(ids[i]) ?? 0) + dy * force);
            fx.set(ids[j], (fx.get(ids[j]) ?? 0) - dx * force);
            fy.set(ids[j], (fy.get(ids[j]) ?? 0) - dy * force);
          }
        }
        // 弹簧力
        for (const e of edges) {
          const a = positions.get(e.from);
          const b = positions.get(e.to);
          if (!a || !b) continue;
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const dist = Math.sqrt(dx * dx + dy * dy) || 1;
          const f = k_spring * (dist - rest_len);
          const ux = dx / dist;
          const uy = dy / dist;
          fx.set(e.from, (fx.get(e.from) ?? 0) + ux * f);
          fy.set(e.from, (fy.get(e.from) ?? 0) + uy * f);
          fx.set(e.to, (fx.get(e.to) ?? 0) - ux * f);
          fy.set(e.to, (fy.get(e.to) ?? 0) - uy * f);
        }
        // 应用位移 + 边界回拉
        for (const id of ids) {
          const p = positions.get(id)!;
          let nx = p.x + (fx.get(id) ?? 0) * dt;
          let ny = p.y + (fy.get(id) ?? 0) * dt;
          if (nx < padding) nx = padding;
          if (nx > W - padding) nx = W - padding;
          if (ny < padding) ny = padding;
          if (ny > H - padding) ny = H - padding;
          positions.set(id, { x: nx, y: ny });
        }
        dt *= 0.99;
      }
    }

    return { positions, W, H, cx, cy };
  }, [visible, wikiTypes, inDegree, edges, layoutMode]);

  // 颜色：wikiType 模式按 token 色板；community 模式按 component 索引等距分布 HSL。
  const colorFor = (it: TreeChunkItem): string => {
    if (colorMode === "community") {
      const idx = community.byId.get(it.id) ?? 0;
      const total = Math.max(community.count, 1);
      const hue = Math.round((idx * 360) / total);
      return `hsl(${hue}, 48%, 42%)`;
    }
    const palette = [
      "#7a4a30",
      "#3d6a52",
      "#5a4d8a",
      "#8a6a3a",
      "#3d6a8a",
      "#8a3a5a",
      "#3a6a3a",
      "#6a3a3a"
    ];
    if (!it.wikiType) return "#888";
    const h = hash01(it.wikiType);
    return palette[Math.floor(h * palette.length)] ?? palette[0];
  };

  // 图例颜色（只取 wikiType；community 模式下只显示组数）。
  const legendColorFor = (wikiType: string): string => {
    const palette = [
      "#7a4a30",
      "#3d6a52",
      "#5a4d8a",
      "#8a6a3a",
      "#3d6a8a",
      "#8a3a5a",
      "#3a6a3a",
      "#6a3a3a"
    ];
    const h = hash01(wikiType);
    return palette[Math.floor(h * palette.length)] ?? palette[0];
  };

  const focused = hovered ? indexById.get(hovered) : null;

  if (loading) return <div className="wikiInspectorEmpty">加载中…</div>;
  if (error) return <div className="wikiAlert error">{error}</div>;
  if (!visible.length) return <div className="wikiInspectorEmpty">无 chunk 可绘图</div>;

  return (
    <div className="wikiGraphPane">
      <header className="wikiArchiveHeader">
        <h2>关系图谱</h2>
        <div className="wikiArchiveSubtitle">
          {visible.length} chunks · {edges.length} edges{filter !== "all" ? ` · 过滤 ${filter}` : ""}
        </div>
      </header>
      <div className="wikiGraphToolbar">
        <label className="wikiGraphFilterLabel">wiki_type：</label>
        <select
          className="wikiGraphFilterSelect"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        >
          <option value="all">全部</option>
          {wikiTypes.map((t) => (
            <option key={t} value={t}>{t}</option>
          ))}
        </select>
        <label className="wikiGraphFilterLabel">布局：</label>
        <select
          className="wikiGraphFilterSelect"
          value={layoutMode}
          onChange={(e) => setLayoutMode(e.target.value as "polar" | "force")}
        >
          <option value="polar">极坐标（确定性）</option>
          <option value="force">力导向（200 步）</option>
        </select>
        <label className="wikiGraphFilterLabel">染色：</label>
        <select
          className="wikiGraphFilterSelect"
          value={colorMode}
          onChange={(e) => setColorMode(e.target.value as "wikiType" | "community")}
        >
          <option value="wikiType">按 wiki_type</option>
          <option value="community">按社区（{community.count} 组）</option>
        </select>
        <span className="wikiGraphLegend">
          {colorMode === "wikiType"
            ? wikiTypes.slice(0, 8).map((t) => (
                <span key={t} className="wikiGraphLegendItem">
                  <span className="wikiGraphLegendDot" style={{ background: legendColorFor(t) }} />
                  {t}
                </span>
              ))
            : (
                <span className="wikiGraphCommunityHint">
                  {community.count} 个连通分量 · 颜色按分量索引等距分布
                </span>
              )}
        </span>
      </div>
      <div className="wikiGraphSvgWrap">
        <svg
          viewBox={`0 0 ${layout.W} ${layout.H}`}
          className="wikiGraphSvg"
          xmlns="http://www.w3.org/2000/svg"
        >
          <defs>
            <marker
              id="wikiGraphArrow"
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="5"
              markerHeight="5"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" fill="#666" />
            </marker>
          </defs>
          <g className="wikiGraphEdges">
            {edges.map((e, i) => {
              const a = layout.positions.get(e.from);
              const b = layout.positions.get(e.to);
              if (!a || !b) return null;
              const stroke = e.kind === "contradicts"
                ? "#a13a3a"
                : e.kind === "superseded_by"
                ? "#7a4a30"
                : e.kind === "previous_version"
                ? "#5a4d8a"
                : "#999";
              const dash = e.kind === "contradicts" || e.kind === "superseded_by" ? "4 3" : undefined;
              const isHovered = hovered === e.from || hovered === e.to;
              return (
                <line
                  key={`${e.from}-${e.to}-${i}`}
                  x1={a.x}
                  y1={a.y}
                  x2={b.x}
                  y2={b.y}
                  stroke={stroke}
                  strokeWidth={isHovered ? 2 : 1}
                  strokeOpacity={isHovered ? 0.85 : 0.35}
                  strokeDasharray={dash}
                  markerEnd="url(#wikiGraphArrow)"
                />
              );
            })}
          </g>
          <g className="wikiGraphNodes">
            {visible.map((it) => {
              const p = layout.positions.get(it.id);
              if (!p) return null;
              const deg = inDegree.get(it.id) ?? 0;
              const r = 5 + Math.min(deg, 6) * 1.5;
              const fill = colorFor(it);
              const isHovered = hovered === it.id;
              const isArchived = it.status === "archived";
              return (
                <g
                  key={it.id}
                  transform={`translate(${p.x},${p.y})`}
                  className="wikiGraphNode"
                  onMouseEnter={() => setHovered(it.id)}
                  onMouseLeave={() => setHovered(null)}
                  onClick={() => focusChunk(it.id)}
                  style={{ cursor: "pointer" }}
                >
                  <circle
                    r={r}
                    fill={isArchived ? "#fff" : fill}
                    stroke={fill}
                    strokeWidth={isHovered ? 2.5 : isArchived ? 1.5 : 1}
                    opacity={isArchived ? 0.7 : 1}
                  />
                  {isHovered ? (
                    <text
                      x={r + 4}
                      y={4}
                      fontFamily="var(--font-mono)"
                      fontSize="11"
                      fill="var(--ink, #222)"
                    >
                      {it.title?.slice(0, 28) || it.id.slice(0, 8)}
                    </text>
                  ) : null}
                </g>
              );
            })}
          </g>
        </svg>
        {focused ? (
          <div className="wikiGraphTooltip">
            <div className="wikiGraphTooltipTitle">{focused.title || "（无标题）"}</div>
            <div className="wikiGraphTooltipMeta">
              <span className="wikiArchiveTag">{focused.wikiType ?? "—"}</span>
              <span className="wikiBadge">{focused.status ?? "—"}</span>
              <span className="wikiGraphTooltipDeg">入度 {inDegree.get(focused.id) ?? 0}</span>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

interface DomainSchemaField {
  name: string;
  label: string;
  kind: string;
  required: boolean;
  allowedValues?: string[] | null;
  aliasOf?: string | null;
}
interface DomainSchemaItem {
  schemaId: string;
  workspaceId: string;
  name: string;
  version: number;
  fields: DomainSchemaField[];
  aliasDict: Record<string, unknown>;
  guardDsl?: string | null;
  isActive: boolean;
  createdAt: number;
  updatedAt: number;
}

export function DomainSchemaTab() {
  const [items, setItems] = useState<DomainSchemaItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [activating, setActivating] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const confirm = useConfirm();

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/admin/domain-schemas");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: DomainSchemaItem[] };
      setItems(data.items ?? []);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function activate(schemaId: string) {
    const ok = await confirm({
      title: "切换为当前在用 Schema？",
      body: "切换后 AI 将按这套行业 Schema 判断字段与状态，立即影响在途会话。",
      tone: "danger",
      confirmText: "确认切换",
    });
    if (!ok) return;
    setActivating(schemaId);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(`/api/admin/domain-schemas/${encodeURIComponent(schemaId)}/activate`, {
        method: "POST",
      });
      if (!r.ok) throw await parseApiError(r);
      setInfo(`已切换 active：${schemaId}`);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setActivating(null);
    }
  }

  return (
    <div className="wikiPanelBody">
      <div className="wikiToolbar">
        <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} />
          {loading ? "加载中…" : "刷新"}
        </button>
        <span className="wikiHint">
          新建 / 编辑 schema 走后端 API（POST/PUT /api/admin/domain-schemas）；UI 仅做激活与只读浏览。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      {!loading && items.length === 0 ? (
        <div className="wikiEmpty">暂无 schema。可通过后端 API 创建一条 fields 数组（≤ 64 项）。</div>
      ) : null}
      <div className="wikiList">
        {items.map((s) => (
          <div className={s.isActive ? "wikiCard active" : "wikiCard"} key={`${s.schemaId}-${s.version}`}>
            <div className="wikiCardHead">
              <div>
                <span className="wikiCardTitle">{s.name}</span>
                <span className="wikiCardMeta">
                  {s.schemaId} · v{s.version} · {s.fields.length} fields
                </span>
              </div>
              <div className="wikiCardActions">
                {s.isActive ? (
                  <span className="wikiBadge active">active</span>
                ) : (
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void activate(s.schemaId)}
                    disabled={activating === s.schemaId}
                  >
                    {activating === s.schemaId ? "切换中…" : "设为 active"}
                  </button>
                )}
              </div>
            </div>
            <div className="wikiCardBody">
              <div className="wikiFieldList">
                {s.fields.map((f) => (
                  <div className="wikiField" key={f.name}>
                    <span className="wikiFieldName">{f.name}</span>
                    <span className="wikiFieldKind">{f.kind}</span>
                    {f.required ? <span className="wikiFieldFlag">required</span> : null}
                    {f.allowedValues && f.allowedValues.length > 0 ? (
                      <span className="wikiFieldFlag">enum({f.allowedValues.length})</span>
                    ) : null}
                  </div>
                ))}
              </div>
              {Object.keys(s.aliasDict ?? {}).length > 0 ? (
                <div className="wikiAlias">
                  <span className="wikiAliasTitle">aliasDict</span>
                  <code>{JSON.stringify(s.aliasDict)}</code>
                </div>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// MetricsTab：进程级 knowledge agent 指标透出。当前只显示 answer cache
// 命中率，后续可扩展。
//
// E5：拉 /api/knowledge/metrics → 渲染 cache hits / misses / entries / TTL。
// 5 秒手动刷新一次（不做 SSE，避免 EventSource 资源滥用）。
export function MetricsTab() {
  const [data, setData] = useState<{ answerCache?: AnswerCacheMetrics } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/knowledge/metrics");
      if (!r.ok) throw await parseApiError(r);
      setData(await r.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载指标失败");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  const cache = data?.answerCache;
  const total = cache ? cache.hits + cache.misses : 0;
  const hitRate = total > 0 ? ((cache!.hits / total) * 100).toFixed(1) : "—";

  return (
    <div className="wikiPanelBody">
      <div className="wikiMetricsHead">
        <div className="wikiMetricsTitle">Answer Cache</div>
        <button type="button" className="wikiMetricsRefresh" onClick={refresh} disabled={loading}>
          {loading ? "刷新中…" : "刷新"}
        </button>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {cache ? (
        <div className="wikiMetricsGrid">
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">命中</div>
            <div className="wikiMetricValue">{cache.hits}</div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">未命中</div>
            <div className="wikiMetricValue">{cache.misses}</div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">命中率</div>
            <div className="wikiMetricValue">{hitRate}%</div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">条目</div>
            <div className="wikiMetricValue">
              {cache.entries}
              <span className="wikiMetricSub"> / {cache.maxEntries}</span>
            </div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">TTL</div>
            <div className="wikiMetricValue">
              {cache.ttlSeconds}
              <span className="wikiMetricSub"> 秒</span>
            </div>
          </div>
        </div>
      ) : !loading && !error ? (
        <div className="wikiHint">暂无指标数据。</div>
      ) : null}
    </div>
  );
}

interface AnswerCacheMetrics {
  entries: number;
  hits: number;
  misses: number;
  maxEntries: number;
  ttlSeconds: number;
}

// LintView：8 类 kind 树 + signal 列表 + 处置三按钮。替代旧 GapSignalsTab。
//
// 树是计数视图：左侧每行 [kind label] [count]；点击切换右侧 filter。
// 处置：dismiss（忽略）/ apply（标记已处理）；外加 sweep 一键扫描刷新。

// ──────────────────────────────────────────────────────────────────────
// G5 · AdminGovernanceView / MetadataDashboard / PublishBar
// ──────────────────────────────────────────────────────────────────────

interface MetadataResp {
  wikiTypeCounts?: Array<{ wikiType?: string; count?: number }>;
  verifiedRatioByType?: Array<{
    wikiType?: string;
    total?: number;
    verified?: number;
    ratio?: number;
  }>;
  topEditors?: Array<{ author?: string; count?: number }>;
  recentActivity7d?: Array<{ date?: string; op?: string; count?: number }>;
}

function MetadataDashboard() {
  const [data, setData] = useState<MetadataResp | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/metadata");
      if (!r.ok) throw await parseApiError(r);
      setData((await r.json()) as MetadataResp);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 计算 wikiType 柱状图最大值，做归一化。
  const maxCount = useMemo(() => {
    const arr = data?.wikiTypeCounts ?? [];
    return arr.reduce((m, x) => Math.max(m, Number(x.count ?? 0)), 0);
  }, [data]);

  // 7d 活跃数据按日期归并 + total。
  const activityByDate = useMemo(() => {
    const arr = data?.recentActivity7d ?? [];
    const map: Record<string, number> = {};
    for (const a of arr) {
      const d = a.date ?? "";
      if (!d) continue;
      map[d] = (map[d] ?? 0) + Number(a.count ?? 0);
    }
    return Object.entries(map)
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([date, count]) => ({ date, count }));
  }, [data]);

  const maxActivity = useMemo(
    () => activityByDate.reduce((m, x) => Math.max(m, x.count), 0),
    [activityByDate]
  );

  return (
    <div className="wikiMetadataDashboard">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">atlas / governance</div>
          <h2>元信息总览</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <div className="wikiBannerError">{error}</div> : null}

      <div className="wikiMetadataGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">counts</span>
            <h4>wiki_type 切片分布</h4>
          </header>
          {data?.wikiTypeCounts && data.wikiTypeCounts.length > 0 ? (
            <div className="wikiCoverageBars">
              {data.wikiTypeCounts.map((row, i) => {
                const ratio = maxCount > 0 ? Number(row.count ?? 0) / maxCount : 0;
                return (
                  <div className="wikiCoverageBarRow" key={i}>
                    <span className="wikiCoverageBarLabel">{row.wikiType ?? "?"}</span>
                    <div className="wikiCoverageBar">
                      <div
                        className="wikiCoverageBarFill"
                        style={{ width: `${(ratio * 100).toFixed(0)}%` }}
                      />
                    </div>
                    <span className="wikiCoverageBarValue">{row.count ?? 0}</span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="wikiEmpty">暂无切片</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">ratio</span>
            <h4>verified 占比</h4>
          </header>
          {data?.verifiedRatioByType && data.verifiedRatioByType.length > 0 ? (
            <div className="wikiCoverageBars">
              {data.verifiedRatioByType.map((row, i) => {
                const ratio = Math.max(0, Math.min(1, Number(row.ratio ?? 0)));
                return (
                  <div className="wikiCoverageBarRow" key={i}>
                    <span className="wikiCoverageBarLabel">{row.wikiType ?? "?"}</span>
                    <div className="wikiCoverageBar">
                      <div
                        className="wikiCoverageBarFill"
                        style={{ width: `${(ratio * 100).toFixed(0)}%` }}
                      />
                    </div>
                    <span className="wikiCoverageBarValue">
                      {(ratio * 100).toFixed(0)}% · {row.verified ?? 0}/{row.total ?? 0}
                    </span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="wikiEmpty">暂无数据</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">editors</span>
            <h4>近期编辑者</h4>
          </header>
          {data?.topEditors && data.topEditors.length > 0 ? (
            <dl className="wikiArchiveMeta">
              {data.topEditors.map((row, i) => (
                <Fragment key={i}>
                  <dt>{row.author ?? "unknown"}</dt>
                  <dd>{row.count ?? 0}</dd>
                </Fragment>
              ))}
            </dl>
          ) : (
            <div className="wikiEmpty">暂无编辑记录</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">activity</span>
            <h4>7 天活跃</h4>
          </header>
          {activityByDate.length > 0 ? (
            <div className="wikiActivityChart">
              {activityByDate.map((d) => {
                const h = maxActivity > 0 ? (d.count / maxActivity) * 100 : 0;
                return (
                  <div className="wikiActivityBar" key={d.date} title={`${d.date}: ${d.count}`}>
                    <div
                      className="wikiActivityBarFill"
                      style={{ height: `${h.toFixed(0)}%` }}
                    />
                    <span className="wikiActivityBarLabel">{d.date.slice(5)}</span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="wikiEmpty">7d 内无修订</div>
          )}
        </article>
      </div>
    </div>
  );
}

interface PublishBarProps {
  resourceKind: "taxonomies" | "operation-state-policies" | "operation-domains";
  id: string;
  onChange?: () => void;
}

function PublishBar({ resourceKind, id, onChange }: PublishBarProps) {
  const [busy, setBusy] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const confirm = useConfirm();

  async function call(action: "publish" | "rollout" | "rollback") {
    if (action === "publish") {
      const ok = await confirm({
        title: "发布新版？",
        body: "发布后将成为该资源的当前在用版本，影响 AI 后续判断。",
        tone: "danger",
        confirmText: "确认发布",
      });
      if (!ok) return;
    } else if (action === "rollout") {
      const ok = await confirm({
        title: "灰度全量？",
        body: "将把新版本推送给全部会话，立即对所有客户生效，且不可逆。",
        tone: "danger",
        requireText: "全量",
        confirmText: "确认全量发布",
      });
      if (!ok) return;
    } else {
      const ok = await confirm({
        title: "回退到上一版本？",
        body: "将放弃当前版本，恢复为上一个已发布版本。",
        tone: "danger",
        confirmText: "确认回退",
      });
      if (!ok) return;
    }
    setBusy(action);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/admin/${resourceKind}/${encodeURIComponent(id)}/${action}`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(`${action} ok`);
      onChange?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  return (
    <div className="wikiPublishBar">
      <button
        type="button"
        onClick={() => void call("publish")}
        disabled={busy !== ""}
        className="wikiActionBtn--verify"
      >
        <CheckCircle2 size={12} /> {busy === "publish" ? "发布中…" : "发布新版"}
      </button>
      <button type="button" onClick={() => void call("rollout")} disabled={busy !== ""}>
        <ArrowRight size={12} /> {busy === "rollout" ? "灰度中…" : "灰度全量"}
      </button>
      <button
        type="button"
        onClick={() => void call("rollback")}
        disabled={busy !== ""}
        className="wikiActionBtn--reject"
      >
        <Undo2 size={12} /> {busy === "rollback" ? "回退中…" : "回退上版"}
      </button>
      {info ? <span className="wikiPublishBarInfo">{info}</span> : null}
      {error ? <span className="wikiPublishBarError">{error}</span> : null}
    </div>
  );
}

interface TaxonomyEntryView {
  id: string;
  scope?: string;
  kind?: string;
  value?: { id?: string; displayName?: string; status?: string };
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  updatedAt?: string;
}

export function AdminGovernanceView() {
  const [tab, setTab] = useState<"meta" | "taxonomies" | "policies" | "domains">("meta");
  return (
    <div className="wikiArchiveShell wikiAdminGovernance">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">atlas / governance</div>
          <h2>治理工坊</h2>
        </div>
      </header>
      <div className="wikiAdminTabs">
        <button
          type="button"
          className={tab === "meta" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("meta")}
        >
          <Activity size={12} /> 元信息
        </button>
        <button
          type="button"
          className={tab === "taxonomies" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("taxonomies")}
        >
          <LibraryBig size={12} /> 分类系统
        </button>
        <button
          type="button"
          className={tab === "policies" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("policies")}
        >
          <ShieldCheck size={12} /> 状态策略
        </button>
        <button
          type="button"
          className={tab === "domains" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("domains")}
        >
          <Workflow size={12} /> 域配置
        </button>
      </div>
      <div className="wikiAdminPanel">
        {tab === "meta" && <MetadataDashboard />}
        {tab === "taxonomies" && <TaxonomiesGovernance />}
        {tab === "policies" && <StatePoliciesGovernance />}
        {tab === "domains" && <DomainGovernance />}
      </div>
    </div>
  );
}

function TaxonomiesGovernance() {
  const [items, setItems] = useState<TaxonomyEntryView[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includeAll, setIncludeAll] = useState(false);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      if (includeAll) params.set("includeAllVersions", "true");
      const r = await fetch(
        `/api/admin/taxonomies${params.toString() ? "?" + params : ""}`
      );
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as { items?: TaxonomyEntryView[] };
      setItems(d.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, [includeAll]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section>
      <div className="wikiAdminToolbar">
        <label className="wikiAdminToolbarLabel">
          <input
            type="checkbox"
            checked={includeAll}
            onChange={(e) => setIncludeAll(e.target.checked)}
          />
          显示历史版本
        </label>
        <button type="button" onClick={() => void load()} disabled={pending}>
          <RefreshCw size={12} /> 刷新
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      <table className="wikiAdminTable">
        <thead>
          <tr>
            <th>scope</th>
            <th>kind</th>
            <th>value</th>
            <th>label</th>
            <th>status</th>
            <th>version</th>
            <th>active</th>
            <th>updated</th>
            <th>actions</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && !pending ? (
            <tr>
              <td colSpan={9}>
                <div className="wikiEmpty">暂无分类</div>
              </td>
            </tr>
          ) : null}
          {items.map((it) => (
            <tr key={it.id} className={it.currentVersion ? "is-active" : ""}>
              <td>{it.scope}</td>
              <td>{it.kind}</td>
              <td className="wikiArchiveTimelineTime">{it.value?.id}</td>
              <td>{it.value?.displayName}</td>
              <td>
                <span className="wikiArchiveTag">{it.value?.status ?? "?"}</span>
              </td>
              <td className="wikiArchiveTimelineTime">v{it.version ?? 0}</td>
              <td>{it.currentVersion ? "✓" : ""}</td>
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
              <td>
                <PublishBar
                  resourceKind="taxonomies"
                  id={it.id}
                  onChange={() => void load()}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

interface StatePolicyEntryView {
  id: string;
  domain?: string;
  version?: number;
  currentVersion?: boolean;
  updatedAt?: string;
  states?: unknown[];
}

function StatePoliciesGovernance() {
  const [items, setItems] = useState<StatePolicyEntryView[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/admin/operation-state-policies");
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as { items?: StatePolicyEntryView[] };
      setItems(d.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section>
      <div className="wikiAdminToolbar">
        <button type="button" onClick={() => void load()} disabled={pending}>
          <RefreshCw size={12} /> 刷新
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      <table className="wikiAdminTable">
        <thead>
          <tr>
            <th>domain</th>
            <th>version</th>
            <th>active</th>
            <th>states</th>
            <th>updated</th>
            <th>actions</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && !pending ? (
            <tr>
              <td colSpan={6}>
                <div className="wikiEmpty">暂无状态策略</div>
              </td>
            </tr>
          ) : null}
          {items.map((it) => (
            <tr key={it.id} className={it.currentVersion ? "is-active" : ""}>
              <td>{it.domain}</td>
              <td className="wikiArchiveTimelineTime">v{it.version ?? 0}</td>
              <td>{it.currentVersion ? "✓" : ""}</td>
              <td className="wikiArchiveTimelineTime">{(it.states ?? []).length} 状态</td>
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
              <td>
                <PublishBar
                  resourceKind="operation-state-policies"
                  id={it.id}
                  onChange={() => void load()}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

interface DomainEntryView {
  id: string;
  domain?: string;
  version?: number;
  currentVersion?: boolean;
  updatedAt?: string;
}

function DomainGovernance() {
  const [items, setItems] = useState<DomainEntryView[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-domains");
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as { items?: DomainEntryView[] };
      setItems(d.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section>
      <div className="wikiAdminToolbar">
        <button type="button" onClick={() => void load()} disabled={pending}>
          <RefreshCw size={12} /> 刷新
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      <table className="wikiAdminTable">
        <thead>
          <tr>
            <th>domain</th>
            <th>version</th>
            <th>active</th>
            <th>updated</th>
            <th>actions</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && !pending ? (
            <tr>
              <td colSpan={5}>
                <div className="wikiEmpty">暂无域配置</div>
              </td>
            </tr>
          ) : null}
          {items.map((it) => (
            <tr key={it.id} className={it.currentVersion ? "is-active" : ""}>
              <td>{it.domain}</td>
              <td className="wikiArchiveTimelineTime">v{it.version ?? 0}</td>
              <td>{it.currentVersion ? "✓" : ""}</td>
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
              <td>
                <PublishBar
                  resourceKind="operation-domains"
                  id={it.id}
                  onChange={() => void load()}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

// ── Phase F · Atlas Mode：运营记忆抽屉 ──────────────────────────────────

interface OperatorMemoryView {
  id: string | null;
  workspaceId: string;
  accountId: string;
  operatorId: string;
  kind: string;
  content: string;
  createdAt?: string | null;
  lastUsedAt?: string | null;
  expiresAt?: string | null;
}

const OPERATOR_MEMORY_KINDS: Array<{ key: string; label: string }> = [
  { key: "", label: "全部" },
  { key: "preference", label: "偏好" },
  { key: "rejection", label: "拒绝" },
  { key: "context", label: "上下文" }
];

export function MemoryDrawer() {
  const [items, setItems] = useState<OperatorMemoryView[]>([]);
  const [kind, setKind] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    setPending(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      if (kind) params.set("kind", kind);
      params.set("limit", "100");
      const r = await fetch(`/api/knowledge/operator-memory?${params.toString()}`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: OperatorMemoryView[] };
      setItems(data.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setItems([]);
    } finally {
      setPending(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind]);

  return (
    <div className="wikiMemoryDrawer">
      <div className="wikiMemoryHead">
        <h3>运营记忆</h3>
        <span className="wikiHint">注入到 reply prompt 的长期偏好/拒绝/上下文</span>
      </div>
      <div className="wikiMemoryFilter">
        {OPERATOR_MEMORY_KINDS.map((k) => (
          <button
            key={k.key || "all"}
            type="button"
            className={kind === k.key ? "wikiMemoryKindBtn active" : "wikiMemoryKindBtn"}
            onClick={() => setKind(k.key)}
          >
            {k.label}
          </button>
        ))}
        <button
          type="button"
          className="wikiMemoryKindBtn"
          onClick={() => void load()}
          disabled={pending}
        >
          <RefreshCw size={12} /> {pending ? "刷新中" : "刷新"}
        </button>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {!error && !pending && items.length === 0 ? (
        <div className="wikiEmpty">该筛选下暂无运营记忆。</div>
      ) : null}
      <ul className="wikiMemoryList">
        {items.map((m) => (
          <li className={`wikiMemoryItem kind-${m.kind}`} key={m.id ?? `${m.kind}-${m.createdAt}`}>
            <div className="wikiMemoryItemHead">
              <span className={`wikiMemoryKind kind-${m.kind}`}>{m.kind}</span>
              <span className="wikiMemoryOperator">{m.operatorId}</span>
            </div>
            <div className="wikiMemoryContent">{m.content}</div>
            <div className="wikiMemoryFoot">
              <span>last_used_at: {m.lastUsedAt ?? "—"}</span>
              <span>expires_at: {m.expiresAt ?? "—"}</span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
