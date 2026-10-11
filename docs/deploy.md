# Buddy Switch 服务器部署文档

## 环境要求

- Linux 服务器（腾讯云 OpenCloudOS / CentOS / Ubuntu 等）
- 域名已解析到服务器 IP（如 `api.wangzs.vip`）
- 服务器防火墙放行 80/443 端口

## 一、服务器环境准备

### 1.1 安装系统依赖

```bash
# OpenCloudOS / CentOS 系
dnf install -y curl build-essential pkg-config sqlite-devel nodejs

# Ubuntu / Debian 系
# apt update && apt install -y curl build-essential pkg-config libsqlite3-dev nodejs
```

### 1.2 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
```

> 如果服务器配置了代理（如 Clash），确保代理端口正确，或临时关闭代理后执行：
> ```bash
> unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY
> ```

## 二、克隆代码

```bash
git clone https://github.com/kingkadienm/trae-workbuddy-switch.git
cd trae-workbuddy-switch
```

## 三、部署脚本

项目根目录包含 `deploy.sh` 一键部署脚本，会自动完成拉代码、构建前端、编译 Rust、重启服务、验证。

```bash
chmod +x deploy.sh
./deploy.sh
```

脚本行为：
1. `git pull` 拉取最新代码
2. `npx vite build` 构建前端产物到 `dist/`
3. `cargo build -p buddy-switch-server --release` 编译 Release 二进制
4. 杀掉旧进程，启动新进程（端口 57890）
5. curl 验证服务是否正常

## 四、服务进程管理

### 4.1 手动启停

```bash
# 前台运行（调试用）
./target/release/buddy-switch serve --port 57890 --no-open

# 后台运行
nohup ./target/release/buddy-switch serve --port 57890 --no-open > buddy-switch.log 2>&1 &

# 停止服务
pkill -f "buddy-switch serve"
```

### 4.2 systemd 常驻服务（推荐）

创建 `/etc/systemd/system/buddy-switch.service`：

```ini
[Unit]
Description=Buddy Switch
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/trae-workbuddy-switch
ExecStart=/root/trae-workbuddy-switch/target/release/buddy-switch serve --port 57890 --no-open
Restart=always
RestartSec=5
Environment=BUDDY_SWITCH_HOME=/root/.buddy-switch

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
systemctl enable --now buddy-switch

# 查看状态
systemctl status buddy-switch

# 查看日志
journalctl -u buddy-switch -f
```

> 注意：`User` 和 `WorkingDirectory`、`ExecStart` 需根据实际部署路径调整。

## 五、BT 面板 Nginx 反代

### 5.1 添加站点

1. BT 面板 → **网站** → **添加站点**
2. 域名填 `api.wangzs.vip`
3. 根目录随便填（如 `/www/wwwroot/api.wangzs.vip`）
4. 数据库不选，PHP 版本选 **纯静态**
5. 提交

### 5.2 申请 SSL 证书

1. 站点 → **设置** → **SSL** → **Let's Encrypt**
2. 勾选域名，申请证书
3. 成功后开启 **强制 HTTPS**

### 5.3 配置反向代理

**不要直接编辑 Nginx 配置文件**（容易产生重复 `location /` 冲突），改用 BT 自带的反向代理功能：

1. 站点 → **设置** → **反向代理** → **添加反向代理**
2. 名称填 `buddy-switch`
3. 目标 URL 填 `http://127.0.0.1:57890`
4. 保存

BT 会自动生成正确的 Nginx 配置并重载。

## 六、验证

```bash
# 本地验证
curl -s http://127.0.0.1:57890/

# 公网验证（浏览器）
https://api.wangzs.vip
```

能看到 Buddy Switch 页面即部署成功。

## 七、后续更新

```bash
cd trae-workbuddy-switch
git pull
./deploy.sh
```

## 八、架构说明

```
公网 (HTTPS 443)
  └─ BT/Nginx（SSL 终结 + 反向代理）
      └─ 127.0.0.1:57890（buddy-switch serve）
          ├─ Axum HTTP 服务
          ├─ rust-embed 内嵌前端资源（dist/ → 编译进二进制）
          └─ 后台任务：自动轮换、积分任务、账号池治理
```

## 九、webui 与桌面端差异

| 功能 | 桌面端 (Tauri) | WebUI (浏览器) |
|------|---------------|----------------|
| 导入本地 WorkBuddy 账号 | 支持 | 不支持（依赖本地文件系统） |
| 导入 panel auths 目录 | 支持 | 不支持（依赖本地文件系统） |
| 导入备份 JSON | 支持 | 支持（浏览器上传文件） |
| 导出账号 JSON | 支持 | 支持（浏览器下载文件） |
| 所有 API 操作 | 支持 | 支持 |

## 十、注意事项

1. **数据备份**：账号数据在 `BUDDY_SWITCH_HOME`（默认 `~/.buddy-switch`），定期备份
2. **不要直接暴露 57890 端口**：只通过 Nginx 反代访问，内部端口不对外
3. **前端缓存**：更新后浏览器访问需强制刷新（`Ctrl+Shift+R`）以清除旧缓存
4. **腾讯云防火墙**：控制台 → 轻量应用服务器 → 防火墙，确保 80/443 已开放
