"""一次性探针:实证"单一职责事实抽取调用"是否比"巨型多职责调用"更可靠地产出
原子 fact + dimension。在 server117 上跑(它能直连 active LLM 端点)。不改任何生产代码。

A 组 = 单一职责(只抽事实);B 组 = 模拟巨型多职责(事实+人格+标签+profile 一把梭)。
同一段"孩子8岁→改口10岁"对话喂两组,对比输出形态(是否原子化 / 是否带 dimension)。

跑法(server 本地):python3 _probe_split.py   —— key/baseUrl/model 从 mongo active provider 读。
"""
import json
import subprocess
import sys

import requests

# ── 从 mongo 读 active provider(不在 argv 明文传 key)──
_js = (
    'const r=db.llm_provider_configs.findOne({isActive:true});'
    'print(JSON.stringify({baseUrl:r.baseUrl,apiKey:r.apiKey,model:r.model,format:r.format}));'
)
out = subprocess.run(
    ["mongosh", "wechatagent", "--quiet", "--eval", _js],
    capture_output=True, text=True, timeout=30,
).stdout.strip()
prov = json.loads([l for l in out.splitlines() if l.startswith("{")][0])
BASE, KEY, MODEL = prov["baseUrl"].rstrip("/"), prov["apiKey"], prov["model"]
print(f"[probe] provider={MODEL} base={BASE} format={prov['format']}")

# ── 改口对话(与 batch_a_domain9 同款语义:8岁→改口10岁)──
CONVO = """[对话原文，0-based 升序]
0 客户：你好，我想给孩子报个少儿编程课
1 我：您好！请问孩子多大了，有没有编程基础？
2 客户：孩子8岁，零基础，男孩
3 我：好的，8岁零基础很适合从图形化编程入门。预算方面有考虑吗？
4 客户：预算5000左右吧
5 我：明白，我记下了孩子8岁、预算5000。
6 客户：哦对，我说错了，孩子其实是10岁，不是8岁，记错了
7 我：好的，已更正，孩子10岁零基础，预算5000。"""

# ── A 组:单一职责事实抽取 prompt(从巨型 consolidator 里剥出的那一个职责)──
PROMPT_A = """你是对话事实抽取器。从下面的对话原文里抽取关于"客户"的长期事实，输出严格 JSON：
{"facts":[{"text":"一条只讲一个事实的原子陈述","dimension":"该事实的语义维度名"}]}

硬规则：
- 每条 fact 必须原子化：只讲一个属性/一个数值/一个角色。绝不把多个事实揉进一条长句。
- dimension：对该事实做语义维度归类（如 孩子年龄 / 预算 / 基础水平 / 性别）。同一属性必须用同一 dimension 名。
- 当客户改口/更正时，新旧两个值都各自成为独立一条 fact，且用相同 dimension 名（让系统能识别冲突）。
- 只输出 JSON，不要 markdown，不要解释。

对话原文：
""" + CONVO

# ── B 组:模拟巨型多职责(同时事实+人格+标签+profile,复刻生产 consolidator 的认知负载)──
PROMPT_B = """你是用户运营长期记忆整理 Agent。基于对话原文，一次性输出严格 JSON：
{
 "coreFacts":[{"text":"原子事实","dimension":"语义维度名","importance":8}],
 "coreProfile":{"identity":"","businessContext":"","communicationStyle":"","operationGoal":""},
 "relationshipState":{"stage":"","trustLevel":"","temperature":""},
 "preferences":[], "doNotDo":[], "commitments":[], "objections":[], "openLoops":[],
 "reconfirmedTags":[{"value":"标签","evidenceTurns":[]}],
 "personality":{"openness":{"score":0,"confidence":0,"evidenceTurns":[]},
   "conscientiousness":{"score":0,"confidence":0,"evidenceTurns":[]},
   "extraversion":{"score":0,"confidence":0,"evidenceTurns":[]},
   "agreeableness":{"score":0,"confidence":0,"evidenceTurns":[]},
   "neuroticism":{"score":0,"confidence":0,"evidenceTurns":[]}},
 "summary":""
}
规则：coreFacts 每条原子化(只讲一个事实)、带 dimension(同属性同名)；改口时新旧值各成一条同 dimension；
人格五维行为锚定；标签需 evidenceTurns。只输出 JSON。

对话原文：
""" + CONVO


def call_llm(prompt: str) -> str:
    """Anthropic messages 格式(format=messages)。"""
    url = f"{BASE}/v1/messages"
    headers = {
        "x-api-key": KEY,
        "anthropic-version": "2023-06-01",
        "content-type": "application/json",
    }
    body = {
        "model": MODEL,
        "max_tokens": 2000,
        "messages": [{"role": "user", "content": prompt}],
    }
    r = requests.post(url, headers=headers, json=body, timeout=170)
    r.raise_for_status()
    data = r.json()
    # Anthropic: content 是 block 数组,取 text block
    parts = data.get("content", [])
    return "".join(p.get("text", "") for p in parts if p.get("type") == "text")


def analyze(label: str, raw: str) -> None:
    print(f"\n===== {label} 原始输出 =====")
    print(raw[:1500])
    # 解析 facts/coreFacts
    facts = None
    try:
        j = json.loads(raw.strip().strip("`").lstrip("json").strip())
        facts = j.get("facts") or j.get("coreFacts")
    except Exception as e:
        print(f"[{label}] JSON 解析失败: {e}")
        return
    if not isinstance(facts, list):
        print(f"[{label}] 无 facts/coreFacts 数组")
        return
    print(f"\n----- {label} 形态分析 -----")
    print(f"fact 条数: {len(facts)}")
    blob = 0
    no_dim = 0
    ages = set()
    import re
    for i, f in enumerate(facts):
        t = (f.get("text") or "") if isinstance(f, dict) else str(f)
        d = (f.get("dimension") or "") if isinstance(f, dict) else ""
        is_blob = ("\n" in t) or (len(t) > 60)
        if is_blob:
            blob += 1
        if not d.strip():
            no_dim += 1
        for m in re.findall(r"(\d+)\s*岁", t):
            ages.add(int(m))
        print(f"  [{i}] dim={d or '<无>'} len={len(t)} {'<BLOB>' if is_blob else ''} text={t[:80]}")
    print(f"\n判定: blob条数={blob} 无dimension条数={no_dim} 出现年龄={sorted(ages)}")
    atomic_ok = (blob == 0) and (no_dim == 0)
    age_split = (8 in ages and 10 in ages and blob == 0)  # 8和10各自独立成条(非揉一起)
    print(f"  原子化+全带dimension: {'✅PASS' if atomic_ok else '❌FAIL'}")
    print(f"  改口双值各成独立原子条(可裁决): {'✅' if age_split else ('—' if 8 not in ages else '?')}")


if __name__ == "__main__":
    for label, prompt in [("A组单一职责", PROMPT_A), ("B组模拟巨型", PROMPT_B)]:
        try:
            raw = call_llm(prompt)
            analyze(label, raw)
        except Exception as e:
            print(f"[{label}] 调用失败: {e}")
    print("\n=== 探针完成 ===")
