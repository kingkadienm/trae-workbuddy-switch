#!/bin/bash
set -euo pipefail

APP_DIR="/root/trae-workbuddy-switch"
BIN="$APP_DIR/target/release/buddy-switch"
PORT=57890
LOG="$APP_DIR/buddy-switch.log"

echo "=== Buddy Switch 部署脚本 ==="

cd "$APP_DIR"

# 拉代码
echo "[1/4] 拉取最新代码..."
git pull origin main

# 构建前端
echo "[2/4] 构建前端..."
npx vite build

# 编译 Rust
echo "[3/4] 编译 Rust..."
source ~/.cargo/env
cargo build -p buddy-switch-server --release

# 重启服务
echo "[4/4] 重启服务..."
pkill -f "buddy-switch serve" 2>/dev/null || true
sleep 1
nohup "$BIN" serve --port "$PORT" --no-open > "$LOG" 2>&1 &
sleep 2

# 验证
if curl -s http://127.0.0.1:$PORT/ > /dev/null 2>&1; then
    echo "✅ 部署成功！访问 https://api.wangzs.vip"
else
    echo "❌ 服务启动失败，查看日志："
    tail -20 "$LOG"
    exit 1
fi
