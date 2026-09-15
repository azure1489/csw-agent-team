#!/usr/bin/env bash
# 一键部署 csw-task 前后端到生产服务器（幂等，可反复执行）。
#
#   ./deploy.sh [user@host] [domain]
#   默认：root@8.138.43.109  tasks.aworld.ltd
#
# 做什么：
#   1. 本机构建：server/adminsrv/adminctl（linux/amd64 静态）+ 前端 dist（VITE_API_BASE=https://<domain>）
#   2. 上传：二进制 → /opt/csw-task/bin（停服替换）；dist → nginx 容器挂载的 html/csw-task-web
#   3. 远端配置（幂等）：迁移前自动备份数据库、迁移、两个 systemd 服务、adminsrv 的 JWT secret（仅首次生成）、
#      server.env（notifier 开关与飞书凭证占位，仅首次生成，默认关闭）、
#      superadmin 引导（仅首次，随机密码打印一次）、nginx 反代（/ 前端、/admin→8081、/api/v1→8080）
#   4. 验证：healthz / 前端 200 / adminsrv 401（未登录即活着）
#
# 不做什么：agent 角色 token 签发（一次性引导，跑 `ssh <host> /opt/csw-task/bin/adminctl token issue <role>`）；
#           config.yaml 凭证管理（远端已有则绝不覆盖；远端没有且本机 csw-task-svc/config.yaml 存在则首次上传）。
set -euo pipefail

HOST="${1:-root@8.138.43.109}"
DOMAIN="${2:-tasks.aworld.ltd}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SVC="$SCRIPT_DIR/csw-task-svc"
WEB="$SCRIPT_DIR/csw-task-web"
ROSTER="$SCRIPT_DIR/../skill/roster.json"
STAGE="$(mktemp -d /tmp/csw-deploy.XXXX)"
trap 'rm -rf "$STAGE"' EXIT

log() { printf '\n\033[1;36m── %s\033[0m\n' "$*"; }

log "1/5 本机构建（go ×3 + 前端 dist）"
(cd "$SVC" &&
  GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build -o "$STAGE/server"   ./cmd/server &&
  GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build -o "$STAGE/adminsrv" ./cmd/adminsrv &&
  GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build -o "$STAGE/adminctl" ./cmd/adminctl)
(cd "$WEB" && VITE_API_BASE="https://$DOMAIN" pnpm build | tail -2)

log "2/5 上传（二进制 → upload 暂存；dist/roster 直达；config.yaml 仅远端缺失时）"
ssh "$HOST" 'mkdir -p /opt/csw-task/bin /opt/csw-task/data /opt/csw-task/upload'
scp -q "$STAGE/server" "$STAGE/adminsrv" "$STAGE/adminctl" "$HOST":/opt/csw-task/upload/
[ -f "$ROSTER" ] && scp -q "$ROSTER" "$HOST":/opt/csw-task/roster.json
tar -C "$WEB/dist" -cf - . | ssh "$HOST" 'mkdir -p /opt/docker/nginx/html/csw-task-web && tar -xf - -C /opt/docker/nginx/html/csw-task-web'
if [ -f "$SVC/config.yaml" ]; then
  ssh "$HOST" 'test -f /opt/csw-task/config.yaml' ||
    { scp -q "$SVC/config.yaml" "$HOST":/opt/csw-task/config.yaml && ssh "$HOST" 'chmod 600 /opt/csw-task/config.yaml' && echo "   config.yaml 首次上传"; }
fi

log "3/5 远端配置（systemd / secret / superadmin / nginx）"
ssh "$HOST" DOMAIN="$DOMAIN" 'bash -s' <<'REMOTE'
set -euo pipefail
cd /opt/csw-task

# 停服替换二进制（运行中 ELF 不能原地覆盖）
systemctl stop csw-task csw-task-admin 2>/dev/null || true
mv -f upload/server upload/adminsrv upload/adminctl bin/ && chmod +x bin/*

export CSW_CONFIG=/opt/csw-task/config.yaml CSW_DATA_DIR=/opt/csw-task/data

# 迁移前备份（服务已停；含表重建的迁移出问题时可整库回退）
if [ -f data/csw-task.db ]; then
  mkdir -p data/backup
  BK="data/backup/csw-task.$(date +%Y%m%d%H%M%S).db"
  if command -v sqlite3 >/dev/null 2>&1; then
    sqlite3 data/csw-task.db ".backup '$BK'"
  else
    cp data/csw-task.db "$BK"; for x in wal shm; do [ -f "data/csw-task.db-$x" ] && cp "data/csw-task.db-$x" "$BK-$x"; done
  fi
  echo "   迁移前备份：$BK"
fi
./bin/adminctl migrate up 2>&1 | tail -1

# adminsrv 的 secret/env：仅首次生成（重生成会踢掉全部后台登录态）
if [ ! -f adminsrv.env ]; then
  { echo "CSW_JWT_SECRET=$(openssl rand -hex 32)";
    echo "CSW_ADMIN_ADDR=:8081";
    echo "CSW_ADMIN_CORS_ORIGIN=https://$DOMAIN";
    echo "CSW_COOKIE_SECURE=true"; } > adminsrv.env
  chmod 600 adminsrv.env
  echo "   adminsrv.env 已生成（JWT secret 固化）"
fi

# server 的 notifier 开关与飞书凭证：仅首次生成占位（默认关闭）。填入主编 bot 的 app_id / app_secret 后，
# 先 CSW_NOTIFIER_DRY_RUN=true 演练一轮看日志，再改 CSW_NOTIFIER_ENABLED=true 并重启 csw-task。
if [ ! -f server.env ]; then
  { echo "CSW_NOTIFIER_ENABLED=false";
    echo "CSW_NOTIFIER_DRY_RUN=true";
    echo "CSW_LARK_APP_ID=";
    echo "CSW_LARK_APP_SECRET="; } > server.env
  chmod 600 server.env
  echo "   server.env 已生成（notifier 默认关闭，飞书凭证待填）"
fi

# 首个 superadmin：仅当不存在时创建，随机密码只打印这一次
if ! ./bin/adminctl user list 2>/dev/null | grep -qw admin; then
  PW="$(openssl rand -base64 12)"
  ./bin/adminctl user create admin --role superadmin --password "$PW" >/dev/null
  echo "   ★ 首个后台账号 admin / $PW （仅此一次显示，请立即保存或登录后改密）"
fi

cat > /etc/systemd/system/csw-task.service <<UNIT
[Unit]
Description=csw-task run-plane server
After=network.target docker.service
[Service]
WorkingDirectory=/opt/csw-task
Environment=CSW_CONFIG=/opt/csw-task/config.yaml
Environment=CSW_DATA_DIR=/opt/csw-task/data
Environment=CSW_ADDR=:8080
Environment=CSW_BASE_URL=https://$DOMAIN
Environment=CSW_OSS_PREFIX=csw
EnvironmentFile=-/opt/csw-task/server.env
ExecStart=/opt/csw-task/bin/server
Restart=always
RestartSec=3
TimeoutStopSec=15
[Install]
WantedBy=multi-user.target
UNIT

cat > /etc/systemd/system/csw-task-admin.service <<UNIT
[Unit]
Description=csw-task admin-plane server
After=network.target docker.service
[Service]
WorkingDirectory=/opt/csw-task
Environment=CSW_CONFIG=/opt/csw-task/config.yaml
Environment=CSW_DATA_DIR=/opt/csw-task/data
EnvironmentFile=/opt/csw-task/adminsrv.env
ExecStart=/opt/csw-task/bin/adminsrv
Restart=always
RestartSec=3
[Install]
WantedBy=multi-user.target
UNIT

cat > /opt/docker/nginx/conf.d/$DOMAIN.conf <<NG
# csw-task：/ 管理后台前端（静态 SPA）；/admin/* 后台 API :8081；/api/v1/* 运行面 :8080
server {
    listen 80;
    server_name $DOMAIN;
    return 301 https://\$host\$request_uri;
}

server {
    listen 443 ssl;
    server_name $DOMAIN;

    ssl_certificate     /data/www/nginx/cert/aworld.ltd/aworld.ltd.crt;
    ssl_certificate_key /data/www/nginx/cert/aworld.ltd/aworld.ltd.key;
    ssl_session_timeout 5m;
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_ciphers         ECDHE-RSA-AES128-GCM-SHA256:ECDHE:ECDH:AES:HIGH:!NULL:!aNULL:!MD5:!ADH:!RC4;
    ssl_prefer_server_ciphers on;

    # 一步式 submit 的 zip 上传，与服务端 64MiB 上限对齐留余量
    client_max_body_size    128m;
    client_body_buffer_size 1024k;
    proxy_connect_timeout   30s;
    proxy_send_timeout      300s;
    proxy_read_timeout      300s;

    # 运行面 API（agent bearer）
    location /api/v1/ {
        proxy_pass http://172.17.0.1:8080;
        proxy_redirect off;
        proxy_set_header Host              \$host;
        proxy_set_header X-Real-IP         \$remote_addr;
        proxy_set_header X-Forwarded-For   \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }
    location = /healthz {
        proxy_pass http://172.17.0.1:8080;
        proxy_set_header Host \$host;
    }

    # 管理后台 API（JWT + refresh cookie）
    location /admin/ {
        proxy_pass http://172.17.0.1:8081;
        proxy_redirect off;
        proxy_set_header Host              \$host;
        proxy_set_header X-Real-IP         \$remote_addr;
        proxy_set_header X-Forwarded-For   \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    # 管理后台前端（SPA，fallback 到 index.html）
    location / {
        root /usr/share/nginx/html/csw-task-web;
        try_files \$uri /index.html;
    }

    access_log /data/www/nginx/log/$DOMAIN.log;
}
NG

systemctl daemon-reload
systemctl enable --now csw-task csw-task-admin >/dev/null 2>&1
systemctl restart csw-task csw-task-admin
docker exec base-nginx nginx -t >/dev/null && docker exec base-nginx nginx -s reload
echo "   服务: server=$(systemctl is-active csw-task) adminsrv=$(systemctl is-active csw-task-admin) nginx=reloaded"
REMOTE

log "4/5 验证"
sleep 1
printf '   healthz:  %s\n' "$(curl -fsS "https://$DOMAIN/healthz")"
printf '   前端:     HTTP %s\n' "$(curl -s -o /dev/null -w '%{http_code}' "https://$DOMAIN/")"
printf '   adminsrv: HTTP %s（未登录 401 即正常）\n' "$(curl -s -o /dev/null -w '%{http_code}' "https://$DOMAIN/admin/me")"

log "5/5 完成 → https://$DOMAIN （后台登录） · API https://$DOMAIN/api/v1"
