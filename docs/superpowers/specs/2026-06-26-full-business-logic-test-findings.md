# 全量业务逻辑测试 问题清单

| 域 | 现象 | severity | 根因初判 | 证据 |
|---|---|---|---|---|
| ①文章进库 | 断言失败: LLM 析出至少 1 个 chunk | critical | 断言不成立，见证据 | `preview={'error': 'llm_unavailable', 'kind': 'http_5xx', 'retryCount': 2, 'detail': 'LLM HTTP 503 Service Unavailable: {"error":{"message":"Service temporarily unavailable. Please try again.","type":"api_error"},"type":"error"}', 'hint': '上游 LLM 返回 5xx 错误，已多次重试仍失败。这通常是 LLM 平台侧问题，请稍后再试。'}` |
