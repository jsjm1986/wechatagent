#!/bin/bash
# WechatAgent 快速部署脚本
# 分支: fix/dispatcher-send-timeout-alignment (15 commits)
# 日期: 2026-07-07

set -e  # 遇到错误立即退出

echo "=================================================="
echo "WechatAgent MCP 接入版本 - 快速部署脚本"
echo "=================================================="
echo ""

# 变量配置
PROJECT_DIR="/root/wechatagent"
BRANCH="fix/dispatcher-send-timeout-alignment"
SERVICE_NAME="wechatagent"

: "${MCP_API_KEY:?MCP_API_KEY must be injected by the deployment environment}"

echo "📍 步骤 1: 拉取最新代码"
cd $PROJECT_DIR
git fetch origin
echo "✓ 代码拉取完成"
echo ""

echo "📋 待合并的 commits:"
git log --oneline origin/$BRANCH ^origin/main | head -15
echo ""

read -p "是否继续合并到 main? (y/n): " confirm
if [ "$confirm" != "y" ]; then
    echo "❌ 部署已取消"
    exit 1
fi

echo "🔀 步骤 2: 合并代码到 main"
git checkout main
git merge origin/$BRANCH --no-edit
git push origin main
echo "✓ 代码合并完成"
echo ""

echo "⚙️  步骤 3: 检查环境变量"
if ! grep -q "MCP_BASE_URL" .env; then
    echo "⚠️  .env 缺少 MCP_BASE_URL，正在添加..."
    echo "MCP_BASE_URL=http://117.72.54.28:3001" >> .env
fi

if ! grep -q "MCP_API_KEY" .env; then
    echo "⚠️  .env 缺少 MCP_API_KEY，使用部署环境注入值..."
    printf 'MCP_API_KEY=%s\n' "$MCP_API_KEY" >> .env
fi

if ! grep -q "RUN_TOKEN_BUDGET_ESCALATED" .env; then
    echo "⚠️  .env 缺少 RUN_TOKEN_BUDGET_ESCALATED，正在添加..."
    echo "RUN_TOKEN_BUDGET_ESCALATED=100000" >> .env
fi

if ! grep -q "WEBHOOK_VERIFY_SIGNATURE" .env; then
    echo "⚠️  .env 缺少 WEBHOOK_VERIFY_SIGNATURE，正在添加..."
    echo "WEBHOOK_VERIFY_SIGNATURE=true" >> .env
fi

if ! grep -q "AUTH_RATE_LIMIT_WINDOW_SECONDS" .env; then
    echo "AUTH_RATE_LIMIT_WINDOW_SECONDS=300" >> .env
fi
if ! grep -q "AUTH_RATE_LIMIT_CLIENT_CAPACITY" .env; then
    echo "AUTH_RATE_LIMIT_CLIENT_CAPACITY=20" >> .env
fi
if ! grep -q "AUTH_RATE_LIMIT_TARGET_CAPACITY" .env; then
    echo "AUTH_RATE_LIMIT_TARGET_CAPACITY=10" >> .env
fi
if ! grep -q "AUTH_RATE_LIMIT_GLOBAL_CAPACITY" .env; then
    echo "AUTH_RATE_LIMIT_GLOBAL_CAPACITY=100" >> .env
fi

echo "✓ 环境变量检查完成"
echo ""

echo "🔨 步骤 4: 编译后端 (预计 10-15 分钟)"
cargo build --release
echo "✓ 后端编译完成"
echo ""

echo "🎨 步骤 5: 编译前端 (预计 2-3 分钟)"
cd frontend
npm install
npm run build
cd ..
echo "✓ 前端编译完成"
echo ""

echo "🔄 步骤 6: 重启后端服务"
if systemctl is-active --quiet $SERVICE_NAME; then
    echo "正在停止旧服务..."
    systemctl stop $SERVICE_NAME
    sleep 2
fi

echo "正在启动新服务..."
systemctl start $SERVICE_NAME
sleep 3

if systemctl is-active --quiet $SERVICE_NAME; then
    echo "✓ 服务启动成功"
else
    echo "❌ 服务启动失败，请检查日志："
    echo "   journalctl -u $SERVICE_NAME -n 50"
    exit 1
fi
echo ""

echo "✅ 步骤 7: 验证部署"
echo "正在检查后端健康..."
HEALTH_CHECK=$(curl -s http://localhost:8080/api/health || echo "failed")

if [[ $HEALTH_CHECK == *"ok"* ]]; then
    echo "✓ 后端健康检查通过"
else
    echo "❌ 后端健康检查失败，响应: $HEALTH_CHECK"
    exit 1
fi
echo ""

echo "=================================================="
echo "🎉 部署完成！"
echo "=================================================="
echo ""
echo "📝 下一步操作："
echo ""
echo "1. 配置 MCP Server Webhook URL:"
echo "   访问 http://117.72.54.28:3001/admin"
echo "   配置 messageWebhookUrl = http://117.72.54.28:8080/webhooks/wechat"
echo ""
echo "2. 端到端测试:"
echo "   - 访问前端: http://117.72.54.28:8080"
echo "   - 点击'账号管理'频道"
echo "   - 点击'登录微信账号'并扫码"
echo "   - 发送测试消息验证 webhook 推送"
echo ""
echo "3. 查看服务日志:"
echo "   journalctl -u $SERVICE_NAME -f"
echo ""
echo "4. 详细部署文档:"
echo "   docs/DEPLOYMENT-STEPS.md"
echo ""
echo "=================================================="
