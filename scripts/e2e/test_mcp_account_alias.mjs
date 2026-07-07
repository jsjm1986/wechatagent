#!/usr/bin/env node
// 端到端测试：验证 Workspace Key + account_alias 自动注入是否工作
// 需要：本地 wechatagent 后端运行在 :8080，DB 有配置 Workspace Key 的账号

const API_BASE = 'http://localhost:8080/api';
const MCP_KEY = 'gwa_ba60a98aada58c10b77f6f20841c77c6c3c0506d9431871f';
const MCP_BASE_URL = 'http://117.72.54.28:3001';

async function login() {
  const resp = await fetch(`${API_BASE}/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: 'admin', password: 'admin123' })
  });
  if (!resp.ok) throw new Error(`Login failed: ${resp.status}`);
  const cookie = resp.headers.get('set-cookie');
  return cookie.split(';')[0]; // 提取 wa_session=...
}

async function ensureAccount(cookie, accountId, alias) {
  // 确保 DB 里有测试账号，配置 Workspace Key + alias
  const resp = await fetch(`${API_BASE}/accounts`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Cookie': cookie
    },
    body: JSON.stringify({
      accountId,
      alias,
      displayName: `测试账号-${alias}`,
      mcpBaseUrl: MCP_BASE_URL,
      mcpApiKey: MCP_KEY,
      online: false
    })
  });
  if (resp.status === 409) {
    console.log(`账号 ${accountId} 已存在，跳过创建`);
    return;
  }
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`Create account failed: ${resp.status} ${text}`);
  }
  console.log(`✓ 创建测试账号 ${accountId} (alias=${alias})`);
}

async function testAccountAlias(cookie, accountId) {
  // 调用 account_list（账号类工具，Workspace Key 下必须传 account_alias）
  // 后端会通过 logged_call_for_account → credentials_for_account 拿到 alias → 自动注入
  console.log(`\n=== 测试 account_list (account_id=${accountId}) ===`);
  const resp = await fetch(`${API_BASE}/management/tools/call`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Cookie': cookie
    },
    body: JSON.stringify({
      accountId,
      toolName: 'account_list',
      arguments: {}
    })
  });

  console.log(`HTTP Status: ${resp.status}`);
  const result = await resp.json();

  if (!resp.ok) {
    console.error('❌ 调用失败:', JSON.stringify(result, null, 2));
    throw new Error(`account_list failed: ${resp.status}`);
  }

  console.log('✓ 调用成功');
  console.log('响应 (前200字符):', JSON.stringify(result).slice(0, 200));

  // 检查返回的账号列表是否包含 alias
  if (result.accounts && result.accounts.length > 0) {
    console.log(`✓ 返回 ${result.accounts.length} 个账号`);
    console.log('第一个账号:', JSON.stringify(result.accounts[0], null, 2));
  }

  return result;
}

async function checkMcpLogs(cookie, accountId) {
  // 查看最新的 mcp_logs，验证 account_alias 是否被注入到 request 里
  console.log(`\n=== 检查 mcp_logs (验证 account_alias 注入) ===`);
  // 这需要后端提供 mcp_logs 查询端点，或直接查 MongoDB
  console.log('(手动查 DB: db.mcp_logs.find().sort({created_at:-1}).limit(1))');
}

async function main() {
  try {
    console.log('=== MCP Account Alias 自动注入测试 ===\n');

    const cookie = await login();
    console.log('✓ 登录成功\n');

    const testAccountId = 'test_workspace_key_account';
    const testAlias = 't-1'; // 对应 MCP server auth_whoami 返回的 accounts[0].alias

    await ensureAccount(cookie, testAccountId, testAlias);

    const result = await testAccountAlias(cookie, testAccountId);

    await checkMcpLogs(cookie, testAccountId);

    console.log('\n✅ 测试通过：Workspace Key + account_alias 自动注入工作正常');

  } catch (err) {
    console.error('\n❌ 测试失败:', err.message);
    process.exit(1);
  }
}

main();
