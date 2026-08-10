// 客户端随机 id 生成（单一真相源）。
//
// 为什么不直接调 `crypto.randomUUID()`：该 API 只在**安全上下文**(secure context)
// 暴露 —— HTTPS、或 host 为 localhost/127.0.0.1。生产当前以纯 HTTP + IP 提供服务
// (`http://<ip>:3003`)，于是 `window.crypto` 存在但 `crypto.randomUUID` 是
// undefined，裸调会抛 `crypto.randomUUID is not a function`。
//
// 这个坑在开发与测试环境都照不出来：`npm run dev` 跑在 localhost（规范特批为安全
// 上下文），vitest 跑 jsdom + Node webcrypto，两边 `randomUUID` 都健在。因此只有
// 「HTTP + 非 localhost」这一种组合会现形，必须由本模块兜底，而不是依赖环境。
//
// 三级降级，逐级放宽对宿主的要求：
//   1. `crypto.randomUUID()` —— 安全上下文下的标准实现
//   2. `crypto.getRandomValues()` 手拼 RFC 4122 v4 —— **非**安全上下文也可用
//   3. 时间戳 + `Math.random()` —— 无 WebCrypto 的兜底
//
// 用途仅限「给一次前端操作起个唯一名字」(sessionId 之类)，不承担任何安全职责，
// 故第 3 级退化到非加密随机是可接受的。切勿用于 token / 密钥 / 防猜测场景。

function webCrypto(): Crypto | undefined {
  const c = typeof globalThis === "undefined" ? undefined : globalThis.crypto;
  return c && typeof c === "object" ? c : undefined;
}

/** 用 getRandomValues 拼 RFC 4122 v4（version=4 / variant=10xx）。 */
function v4FromRandomValues(bytes: Uint8Array): string {
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex: string[] = [];
  for (let i = 0; i < bytes.length; i += 1) hex.push(bytes[i].toString(16).padStart(2, "0"));
  const s = hex.join("");
  return `${s.slice(0, 8)}-${s.slice(8, 12)}-${s.slice(12, 16)}-${s.slice(16, 20)}-${s.slice(20, 32)}`;
}

/** 非加密兜底：时间戳(36 进制) + 两段随机，形态仍是 8-4-4-4-12。 */
function v4FromMathRandom(): string {
  let s = "";
  while (s.length < 32) {
    s += Math.floor(Math.random() * 0x100000000)
      .toString(16)
      .padStart(8, "0");
  }
  const bytes = new Uint8Array(16);
  for (let i = 0; i < 16; i += 1) bytes[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  // 混入时间戳低位，降低同毫秒多标签页撞车概率。
  const now = Date.now();
  bytes[0] = (now >>> 24) & 0xff;
  bytes[1] = (now >>> 16) & 0xff;
  bytes[2] = (now >>> 8) & 0xff;
  bytes[3] = now & 0xff;
  return v4FromRandomValues(bytes);
}

/**
 * 返回一个 UUID v4 形态的字符串，在任何浏览器上下文都不抛异常。
 *
 * 每一级都套 try/catch：某些宿主会把 `randomUUID` 定义成会抛的 stub
 * （非安全上下文下的 polyfill、隐私模式下被禁用的 WebCrypto），只判断
 * `typeof === "function"` 不足以保证调用成功。
 */
export function randomUuid(): string {
  const c = webCrypto();
  if (c && typeof c.randomUUID === "function") {
    try {
      return c.randomUUID();
    } catch {
      /* 落到下一级 */
    }
  }
  if (c && typeof c.getRandomValues === "function") {
    try {
      return v4FromRandomValues(c.getRandomValues(new Uint8Array(16)));
    } catch {
      /* 落到下一级 */
    }
  }
  return v4FromMathRandom();
}
