"""探针 v2:定位生产 consolidator blob 的真因。
v1 已证伪"多职责→blob"(claude 简化多职责也产原子 fact)。v2 测两个更可能的真因:

H1 脏卡沿用:注入一张"当前 memoryCard"其 coreFacts[0] 已是 411 字 blob(模拟上一轮坏数据 +
   seed 标签污染),看 LLM 是重新原子化清理它,还是沿用/累加 blob。
H2 prompt 救回措辞诱导:生产 prompt 有"系统会自动保留你没显式 discarded 的旧 coreFacts"这段,
   可能让 LLM 觉得"旧 blob 我不动它就好"→不重写成原子条。

两组都喂同一改口对话 + 同一脏卡注入,区别只在 prompt 是否含"救回保留"措辞。
跑法(server 本地):python3 _probe2.py
"""
import json
import re
import subprocess

import requests

_js = (
    'const r=db.llm_provider_configs.findOne({isActive:true});'
    'print(JSON.stringify({baseUrl:r.baseUrl,apiKey:r.apiKey,model:r.model}));'
)
out = subprocess.run(
    ["mongosh", "wechatagent", "--quiet", "--eval", _js],
    capture_output=True, text=True, timeout=30,
).stdout.strip()
prov = json.loads([l for l in out.splitlines() if l.startswith("{")][0])
BASE, KEY, MODEL = prov["baseUrl"].rstrip("/"), prov["apiKey"], prov["model"]
print(f"[probe2] provider={MODEL} base={BASE}")

# ── 注入的"当前 memoryCard":coreFacts[0] 是 blob,[1-3] 是 seed 标签污染(复刻真测现场)──
DIRTY_CARD = {
    "coreFacts": [
        {"id": "f0", "text": "客户孩子8岁零基础，预算5000左右，意向明确\n首次接触，孩子8岁零基础想报编程课\n孩子8岁零基础男孩，预算5000，家长沟通直接高效", "importance": 8},
        {"id": "f1", "text": "家长", "importance": 5},
        {"id": "f2", "text": "编程课咨询", "importance": 5},
        {"id": "f3", "text": "初次接触", "importance": 5},
    ],
    "recentFacts": [],
    "coreProfile": {"identity": "", "businessContext": "", "communicationStyle": "", "operationGoal": ""},
}

CONVO = """对话原文（0-based 升序）:
0 客户：你好，我想给孩子报编程课
1 客户：我孩子今年8岁，零基础
2 客户：预算大概5000左右
3 我：好的，记下了孩子8岁、预算5000
4 客户：哦我说错了，孩子其实10岁了，不是8岁"""

# 共同 schema 头
SCHEMA = """请基于「当前 memoryCard」和「对话原文」，输出 JSON：
{"memoryCard":{"coreFacts":[{"id":"沿用旧id或留空","text":"一条只讲一个事实的原子陈述","dimension":"语义维度名(如 孩子年龄/预算)","importance":8}],"recentFacts":[],"deprecatedFacts":[]},"summary":"","discarded":[]}
- 每条 fact 必须原子化:只讲一个事实,不要把多个事实揉进一条长句。
- dimension:语义维度归类,同属性同名;改口时新旧值各成一条同 dimension。
- 严格 JSON,不输出 markdown。"""

# H2 救回措辞(生产 prompt 原文那段)
RECALL_NOTE = """
关键机制:系统会自动保留上一版 memoryCard 中你没有显式弃用(既不在 deprecatedFacts、也不在 discarded)的 coreFacts——这是为了防止有价值的早期事实被新一轮整理意外丢掉。要让某条旧 coreFact 失效,必须显式列入 discarded。"""

def build(prompt_with_recall: bool) -> str:
    p = SCHEMA
    if prompt_with_recall:
        p += RECALL_NOTE
    p += f"\n\n当前 memoryCard:\n{json.dumps(DIRTY_CARD, ensure_ascii=False)}\n\n{CONVO}"
    return p

def call_llm(prompt: str) -> str:
    r = requests.post(
        f"{BASE}/v1/messages",
        headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"},
        json={"model": MODEL, "max_tokens": 2000, "messages": [{"role": "user", "content": prompt}]},
        timeout=170,
    )
    r.raise_for_status()
    return "".join(p.get("text", "") for p in r.json().get("content", []) if p.get("type") == "text")

def analyze(label: str, raw: str) -> None:
    print(f"\n===== {label} =====")
    print(raw[:900])
    try:
        j = json.loads(raw.strip().strip("`").lstrip("json").strip())
        facts = (j.get("memoryCard") or {}).get("coreFacts") or j.get("coreFacts")
    except Exception as e:
        print(f"[{label}] JSON解析失败: {e}")
        return
    if not isinstance(facts, list):
        print(f"[{label}] 无 coreFacts")
        return
    blob = sum(1 for f in facts if isinstance(f, dict) and (("\n" in (f.get("text") or "")) or len(f.get("text") or "") > 60))
    no_dim = sum(1 for f in facts if isinstance(f, dict) and not (f.get("dimension") or "").strip())
    ages = set()
    for f in facts:
        for m in re.findall(r"(\d+)\s*岁", f.get("text", "") if isinstance(f, dict) else ""):
            ages.add(int(m))
    print(f"\n[{label}] fact条数={len(facts)} blob={blob} 无dim={no_dim} 年龄={sorted(ages)}")
    print(f"  清理脏blob成原子条: {'✅是' if blob==0 else '❌否(沿用/累加了blob)'}")
    print(f"  改口裁决(只剩10岁或8/10各独立条): {'✅' if (10 in ages) else '?'}")

if __name__ == "__main__":
    for label, recall in [("H无救回措辞", False), ("H有救回措辞(生产原文)", True)]:
        try:
            analyze(label, call_llm(build(recall)))
        except Exception as e:
            print(f"[{label}] 调用失败: {e}")
    print("\n=== 探针v2完成 ===")
