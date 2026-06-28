"""探针 v4:完全复刻生产 LLM 调用,复现真正的 blob 形态。
v3 漏了两个生产要素 → 只看到 raw tool_use 劫持,没复现 blob。v4 补齐:
1. ANTHROPIC_JSON_GUARD(llm.rs:616 原文)追加到 system 末尾(生产 guarded_system)。
2. max_tokens=8192(生产值,非 2000)。
3. system/user 分离(生产是 system=guarded_system, user=拼接内容),非全塞 user。

假设链:超长 prompt→claude 倾向劫持→guard 压制劫持→claude 改"敷衍式 blob 输出"绕过约束。
v4 串行多跑 R 次(无并发,隔离 prompt+guard 单变量),统计 blob/漏dim/劫持率。
若 blob 复现 → 坐实"guard 压制劫持但诱发 blob"。
跑法(server 本地):python3 _probe4.py
"""
import json
import re
import subprocess
import time

import requests

_js = (
    'const r=db.llm_provider_configs.findOne({isActive:true});'
    'const sys=db.prompt_templates.findOne({prompt_key:"user.memory_consolidator.system",status:"active"});'
    'const tsk=db.prompt_templates.findOne({prompt_key:"user.memory_consolidator.task",status:"active"});'
    'print(JSON.stringify({baseUrl:r.baseUrl,apiKey:r.apiKey,model:r.model,sys:(sys?sys.content:""),task:(tsk?tsk.content:"")}));'
)
out = subprocess.run(
    ["mongosh", "wechatagent", "--quiet", "--eval", _js],
    capture_output=True, text=True, timeout=30,
).stdout.strip()
prov = json.loads([l for l in out.splitlines() if l.startswith("{")][0])
BASE, KEY, MODEL = prov["baseUrl"].rstrip("/"), prov["apiKey"], prov["model"]
SYS_PROMPT, TASK_PROMPT = prov["sys"], prov["task"]
print(f"[probe4] model={MODEL} sys_len={len(SYS_PROMPT)} task_len={len(TASK_PROMPT)}")

# 生产 ANTHROPIC_JSON_GUARD 原文(llm.rs:616)
GUARD = "\n\n[OUTPUT FORMAT — STRICT] 当前是**对话生成模式**，不是 agent / 工具调用模式。禁止调用任何工具（不要 WebFetch、不要联网搜索、不要任何 tool_use），直接基于你已有的知识一次性生成完整内容。你必须只输出一个 JSON 对象，不要任何前导说明、寒暄、共情、思考过程或代码块围栏。第一个字符必须是 `{`，最后一个字符必须是 `}`。禁止在 JSON 前后写任何自然语言（包括「好的」「我理解」「让我」「希望有帮助」之类）。"
GUARDED_SYS = SYS_PROMPT + GUARD

DIRTY_CARD = {
    "coreFacts": [
        {"id": "f0", "text": "客户孩子8岁零基础，预算5000左右，意向明确", "importance": 8},
        {"id": "f1", "text": "家长", "importance": 5},
    ],
    "recentFacts": [], "coreProfile": {"identity": "", "businessContext": ""},
}
CONVO = "0 客户：孩子8岁零基础\n1 客户：预算5000左右\n2 客户：哦说错了，孩子其实10岁不是8岁"
USER = (
    TASK_PROMPT
    + f"\n\n当前 memoryCard:\n{json.dumps(DIRTY_CARD, ensure_ascii=False)}"
    + f"\n\n候选记忆:\n[]\n\n客户昵称: biztest\n客户阶段: \n意向等级: \n\n对话原文（0-based 升序）:\n{CONVO}\n\n当前确信标签:\n[]\n\n待重判标签观察:\n[]\n"
)


def call_prod(user: str) -> tuple[str, str, str]:
    """复刻生产 body:guarded_system + max_tokens 8192。返回 (text, stop_reason, err)。"""
    try:
        r = requests.post(
            f"{BASE}/v1/messages",
            headers={"x-api-key": KEY, "anthropic-version": "2023-06-01", "content-type": "application/json"},
            json={"model": MODEL, "max_tokens": 8192, "temperature": 0.7,
                  "system": GUARDED_SYS, "messages": [{"role": "user", "content": user}]},
            timeout=170,
        )
        if r.status_code != 200:
            return "", "", f"HTTP{r.status_code}"
        data = r.json()
        blocks = data.get("content", [])
        stop = data.get("stop_reason", "")
        has_tool = any(b.get("type") == "tool_use" for b in blocks)
        text = "".join(b.get("text", "") for b in blocks if b.get("type") == "text")
        if has_tool or stop == "tool_use":
            return text, stop, "tool_use_hijack"
        return text, stop, ""
    except Exception as e:
        return "", "", f"exc:{type(e).__name__}"


def grade(text: str, stop: str, err: str) -> dict:
    if err:
        return {"err": err, "stop": stop}
    try:
        j = json.loads(text.strip().strip("`").lstrip("json").strip())
        facts = (j.get("memoryCard") or {}).get("coreFacts") or j.get("coreFacts") or []
    except Exception as e:
        return {"err": f"json_fail:{e}", "stop": stop, "text_head": text[:120]}
    blob = [i for i, f in enumerate(facts) if isinstance(f, dict) and (("\n" in (f.get("text") or "")) or len(f.get("text") or "") > 60)]
    no_dim = sum(1 for f in facts if isinstance(f, dict) and not (f.get("dimension") or "").strip())
    ages = sorted({int(m) for f in facts if isinstance(f, dict) for m in re.findall(r"(\d+)\s*岁", f.get("text", ""))})
    return {"err": "", "stop": stop, "facts": len(facts), "blob_idx": blob, "no_dim": no_dim, "ages": ages}


if __name__ == "__main__":
    R = 6
    print(f"\n===== 复刻生产(guard+8192) 串行 x {R} =====")
    blob_hits, dim_miss, hijack, jsonfail = 0, 0, 0, 0
    for k in range(R):
        text, stop, err = call_prod(USER)
        g = grade(text, stop, err)
        tag = ""
        if g.get("err") == "tool_use_hijack":
            hijack += 1; tag = "【劫持】"
        elif "json_fail" in str(g.get("err", "")):
            jsonfail += 1; tag = "【json解析失败】"
        elif g.get("err"):
            tag = "【" + g["err"] + "】"
        else:
            if g.get("blob_idx"):
                blob_hits += 1; tag += "【BLOB!】"
            if g.get("no_dim"):
                dim_miss += 1; tag += "【漏dim】"
        print(f"  #{k}: {g} {tag}")
        # 出 blob 时打印原始 fact text 看形态
        if g.get("blob_idx"):
            try:
                j = json.loads(text.strip().strip("`").lstrip("json").strip())
                facts = (j.get("memoryCard") or {}).get("coreFacts") or []
                for i in g["blob_idx"]:
                    print(f"      blob[{i}] text={facts[i].get('text','')[:200]!r}")
            except Exception:
                pass
        time.sleep(3)
    print(f"\n=== 统计({R}次,复刻生产): BLOB={blob_hits} 漏dim={dim_miss} 劫持={hijack} json失败={jsonfail} ===")
    print("=== 探针v4完成 ===")
