/* ============================================================
   WechatAgent 官网 — 双语文案字典（参考用）
   说明：本站采用 data-lang-zh / data-lang-en 双块就地切换方案，
   主要文案直接写在 HTML 中（利于 SEO 与零闪烁）。
   本文件集中存放跨页复用的品牌常量与规模数字，供脚本引用。
   ============================================================ */
window.WeAgentI18N = {
  brand: { zh: "WeAgent", en: "WeAgent" },
  tagline: { zh: "私域自主运营", en: "Autonomous Private-Domain Operations" },

  /* 经代码核实的真实规模数字（来自 src/ 与 frontend/src/ 统计） */
  scale: {
    endpoints:   { value: 250, suffix: "+", zh: "REST API 端点", en: "REST API endpoints" },
    collections: { value: 57,  suffix: "",  zh: "MongoDB 集合",  en: "MongoDB collections" },
    migrations:  { value: 28,  suffix: "",  zh: "数据库迁移",     en: "database migrations" },
    workers:     { value: 11,  suffix: "",  zh: "后台自治 Worker", en: "background workers" },
    configs:     { value: 66,  suffix: "",  zh: "可配置项",       en: "config switches" },
    models:      { value: 100, suffix: "+", zh: "业务数据模型",    en: "data models" },
    backendLoc:  { value: 11,  suffix: "万行", zh: "行 Rust 后端", en: "lines of Rust" },
    components:  { value: 105, suffix: "",  zh: "前端组件",       en: "frontend components" },
    tests:       { value: 107, suffix: "",  zh: "测试文件",        en: "test files" }
  }
};
