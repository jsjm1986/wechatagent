"""探针 v3:坐实"并发争用 → consolidator 降级吐 blob/漏 dimension"假设。
端点 rsxermu claude-opus-4.8 只有 2 并发线程。v1/v2 单路串行从不复现 blob;v3 用线程池
制造 >2 路并发争用(模拟生产 gateway 单轮并发打 decision/reaction/knowledge/reply/consolidator),
看被挤的 consolidator 那路是否降级(吐 blob / 漏 dimension / tool_use 劫持 / failed)。

对照:
- 串行组(baseline):consolidator 单独跑 1 次,无争用 —— 预期完美(复刻 v1/v2)。
- 并发组:6 路同时打(5 路陪练 noise + 1 路 consolidator),重复 R 轮,统计 consolidator 降级率。

consolidator 用 DB 里 status=active 的真实生产 prompt(去 Rust 包裹的纯文本)。
跑法(server 本地):python3 _probe3.py
"""
import json
import re
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

import requests

# ── 读 active provider + 真实生产 consolidator prompt ──
_js = (
    'const r=db.llm_provider_configs.findOne({isActive:true});'
    'const t=db.prompt_templates.findOne({prompt_key:"user.memory_consolidator.task",status:"active"});'
    'print(JSON.stringify({baseUrl:r.baseUrl,apiKey:r.apiKey,model:r.model,prompt:(t?t.content:"")}));'
)
out = subprocess.run(
    ["mongosh", "wechatagent", "--quiet", "--eval", _js],
    capture_output=True, text=True, timeout=30,
).stdout.strip()
prov = json.loads([l for l in out.splitlines() if l.startswith("{")][0])
BASE, KEY, MODEL = prov["baseUrl"].rstrip("/"), prov["apiKey"], prov["model"]
PROD_PROMPT = prov["prompt"]
print(f"[probe3] provider={MODEL} prod_prompt_len={len(PROD_PROMPT)}")

# 注入脏卡(复刻真测现场:blob + 标签污染)+ 改口对话,拼到生产 prompt 后
DIRTY_CARD = {
    "coreFacts": [
        {"id": "f0", "text": "客户孩子8岁零基础，预算5000左右，意向明确", "importance": 8},
        {"id": "f1", "text": "家长", "importance": 5},
    ],
    "recentFacts": [], "coreProfile": {"identity": "", "businessContext": ""},
}
CONVO = "0 客户：孩子8岁零基础\n1 客户：预算5000左右\n2 客户：哦说错了，孩子其实10岁不是8岁"
CONSOLIDATOR_USER = (
    PROD_PROMPT
    + f"\n\n当前 memoryCard:\n{json.dumps(DIRTY_CARD, ensure_ascii=False)}"
    + f"\n\n候选记忆:\n[]\n\n客户昵称: biztest\n客户阶段: \n意向等级: \n\n对话原文（0-based 升序）:\n{CONVO}\n\n当前确信标签:\n[]\n\n待重判标签观察:\n[]\n"
)
# 陪练 noise prompt(模拟同轮其它 LLM 调用,占线程)
NOISE_USER = "请用 300 字分析一段客户对话的情绪与意图。对话：客户说想给孩子报编程课，孩子10岁零基础，预算5000。输出自然语言分析。"


def call_llm(prompt: str, max_tokens: int = 2000) -> tuple[str, str]:
    """返回 (text, err)。err 非空表示失败/降级。"""
    try:
        r = requests.post(
            f"{BASE}/v1/messages",
            headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"},
            json={"model": MODEL, "max_tokens": max_tokens, "messages": [{"role": "user", "content": prompt}]},
            timeout=170,
        )
        if r.status_code != 200:
            return "", f"HTTP{r.status_code}"
        data = r.json()
        # tool_use 劫持检测:content 里有 tool_use block 而 text 极短
        blocks = data.get("content", [])
        has_tool = any(b.get("type") == "tool_use" for b in blocks)
        text = "".join(b.get("text", "") for b in blocks if b.get("type") == "text")
        if has_tool:
            return text, "tool_use_hijack"
        return text, ""
    except Exception as e:
        return "", f"exc:{type(e).__name__}"


def grade_consolidator(text: str, err: str) -> dict:
    """判 consolidator 输出形态:blob? 漏dimension? 改口裁决?"""
    if err:
        return {"err": err, "blob": None, "no_dim": None, "ages": []}
    try:
        j = json.loads(text.strip().strip("`").lstrip("json").strip())
        facts = (j.get("memoryCard") or {}).get("coreFacts") or j.get("coreFacts") or []
    except Exception as e:
        return {"err": f"json_fail:{e}", "blob": None, "no_dim": None, "ages": []}
    blob = sum(1 for f in facts if isinstance(f, dict) and (("\n" in (f.get("text") or "")) or len(f.get("text") or "") > 60))
    no_dim = sum(1 for f in facts if isinstance(f, dict) and not (f.get("dimension") or "").strip())
    ages = sorted({int(m) for f in facts if isinstance(f, dict) for m in re.findall(r"(\d+)\s*岁", f.get("text", ""))})
    return {"err": "", "facts": len(facts), "blob": blob, "no_dim": no_dim, "ages": ages}


def run_round(rnd: int, concurrency: int) -> dict:
    """一轮:concurrency 路同时发,第0路是 consolidator,其余 noise。返回 consolidator 评分。"""
    tasks = [("consolidator", CONSOLIDATOR_USER)] + [("noise", NOISE_USER)] * (concurrency - 1)
    results = {}
    with ThreadPoolExecutor(max_workers=concurrency) as ex:
        futs = {ex.submit(call_llm, p): (kind, i) for i, (kind, p) in enumerate(tasks)}
        for fut in as_completed(futs):
            kind, i = futs[fut]
            text, err = fut.result()
            if kind == "consolidator":
                results["cons"] = grade_consolidator(text, err)
            else:
                results.setdefault("noise_err", []).append(err or "ok")
    return results


if __name__ == "__main__":
    R = 5
    CONC = 6  # 6 路并发 >> 2 线程,必争用
    print(f"\n===== 串行baseline(无争用,1路)=====")
    for k in range(2):
        t, e = call_llm(CONSOLIDATOR_USER)
        print(f"  baseline#{k}: {grade_consolidator(t, e)}")
        time.sleep(2)
    print(f"\n===== 并发组({CONC}路争用 x {R}轮)=====")
    blob_hits, dim_miss, errs = 0, 0, 0
    for rnd in range(R):
        res = run_round(rnd, CONC)
        c = res.get("cons", {})
        noise = res.get("noise_err", [])
        tag = ""
        if c.get("err"):
            errs += 1; tag = "【降级:" + c["err"] + "】"
        else:
            if c.get("blob"): blob_hits += 1; tag += "【BLOB】"
            if c.get("no_dim"): dim_miss += 1; tag += "【漏dim】"
        print(f"  轮{rnd}: cons={c} noise={noise} {tag}")
        time.sleep(3)
    print(f"\n=== 统计(共{R}轮并发): blob出现={blob_hits} 漏dimension={dim_miss} 调用降级/失败={errs} ===")
    print("=== 探针v3完成 ===")
