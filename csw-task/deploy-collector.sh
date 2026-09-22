#!/usr/bin/env bash
# 部署情报收集员工作台（csw-collector）到 agent 主机。幂等，可反复执行。
#
#   ./deploy-collector.sh [user@host] [domain] [ssh_port]
#   默认：root@8.138.23.218  collector.aworld.ltd  9318
#
#   --preview   **只把页面部署上去**，完全不碰生产：
#               运行面地址指向打不通的 127.0.0.1:1（于是接不到单、也不会去敲生产引擎的
#               运行面），定时任务全关，密钥填占位符，并把本地造好的样例数据传上去。
#               登录仍然转发到真引擎的后台 —— 那是只读的一次 POST /admin/login，
#               用你自己的账号，工作台这边不存口令。
#               走查页面用这个；确认没问题后去掉 --preview 再跑一次，就是正式部署。
#
#   --seed      （只在 --preview 下有效）把本地造的样例数据传上去，**仅当远端还没有库时**才放。
#               默认不带：样例一旦进了之后要跑真数据的库，就会和真数据混在一起。
#
# ⚠️ **这是一次生产动作，跑之前要单独取得同意。**
#    它会停掉并重启 agent 主机上的 csw-collector，改写那台机上的 nginx 站点配置。
#    它**不碰** Hermes 的任何网关、profile、也不动 base-nginx 的其他站点。
#
# 做什么：
#   1. 本机构建：cargo zigbuild 出 linux/amd64 二进制 + 前端 dist
#   2. 上传：二进制 → /opt/csw-collector/upload（停服后再换，运行中的 ELF 不能原地覆盖）
#            dist → base-nginx 容器挂的 html/csw-collector-web
#   3. 远端配置（幂等）：目录、collector.env（**仅首次生成占位，绝不覆盖已有的**）、
#      collector.toml（同上）、systemd 单元、nginx 站点
#   4. 检查：healthz、前端、**8090 不能从公网打通**
#
# 不做什么：
#   - 不装 codex，也不装 csw MCP。两者要在 /opt/csw-collector/ 下独立固定安装
#     （不与 Hermes 共用，免得它那边一升级就把深核带崩）。脚本只检查在不在，
#     不在就停下来告诉你怎么装——在生产机上跑 npm 是有副作用的事，不该藏在部署脚本里。
#   - 不填密钥。首次会生成一份占位 collector.env（0600），填完再跑一次。
#   - 不激活 daily_news v9，不停 Hermes 收集员网关。那两件是切换清单里的步骤，
#     要在白天、无进行中 run、放行验证通过之后单独做。
set -euo pipefail

PREVIEW=0
WITH_SEED=0
ARGS=()
for a in "$@"; do
  case "$a" in
    --preview) PREVIEW=1 ;;
    --seed) WITH_SEED=1 ;;
    *) ARGS+=("$a") ;;
  esac
done
set -- "${ARGS[@]:-}"

HOST="${1:-root@8.138.23.218}"
DOMAIN="${2:-collector.aworld.ltd}"
PORT="${3:-9318}"                     # agent 主机的 ssh 不是 22
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SVC="$SCRIPT_DIR/csw-collector"
WEB="$SCRIPT_DIR/csw-collector-web"
STAGE="$(mktemp -d /tmp/csw-collector-deploy.XXXX)"
trap 'rm -rf "$STAGE"' EXIT

SSH=(ssh -p "$PORT" "$HOST")
scp_() { scp -q -P "$PORT" "$@"; }

log() { printf '\n\033[1;36m── %s\033[0m\n' "$*"; }
die() { printf '\n\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

log "1/6 本机构建（cargo zigbuild + 前端 dist）"
# 编译不拿管道收尾巴：管道的退出码是最后一节的，编译失败会被报成成功（踩过）
(cd "$SVC" && make build-linux) > "$STAGE/build.log" 2>&1 || { tail -30 "$STAGE/build.log"; die "构建失败，完整日志 $STAGE/build.log"; }
BIN="$SVC/target/x86_64-unknown-linux-gnu/release/csw-collector"
[ -f "$BIN" ] || die "没找到二进制 $BIN"
printf '   二进制 %s\n' "$(ls -lh "$BIN" | awk '{print $5}')"
(cd "$WEB" && pnpm build) > "$STAGE/web.log" 2>&1 || { tail -30 "$STAGE/web.log"; die "前端构建失败，完整日志 $STAGE/web.log"; }
printf '   前端 dist 就绪\n'

log "2/6 检查 codex 与 csw MCP（不装，只检查）"
if [ "$PREVIEW" = "1" ]; then
  echo "   预览模式，跳过 —— 深核（第 6 步）用不到，其余九步不依赖它"
else
"${SSH[@]}" 'test -x /opt/csw-collector/codex/bin/codex' || die "agent 主机上没有 /opt/csw-collector/codex/bin/codex

深核要它。独立装一份（不与 Hermes 共用）：
  ssh -p $PORT $HOST
  mkdir -p /opt/csw-collector/codex
  npm i -g @openai/codex@0.155.1 --prefix /opt/csw-collector/codex
  /opt/csw-collector/codex/bin/codex --version    # 应为 0.155.1

装好后把 config.toml 放到 /opt/csw-collector/codex/home/（指向 csw-subapi + gpt-6-astra
+ csw MCP 只读白名单），再跑一次本脚本。"
"${SSH[@]}" 'test -d /opt/csw-collector/csw-mcp' || die "agent 主机上没有 /opt/csw-collector/csw-mcp

csw MCP 要固定安装，不走 npx 拉 GitHub（拉不到的那天深核就全废）。装好再跑一次。"
printf '   codex %s\n' "$("${SSH[@]}" '/opt/csw-collector/codex/bin/codex --version 2>/dev/null || echo 未知')"
fi

log "3/6 上传"
"${SSH[@]}" 'mkdir -p /opt/csw-collector/{bin,data,upload,codex,csw-mcp}'
scp_ "$BIN" "$HOST":/opt/csw-collector/upload/csw-collector
tar -C "$WEB/dist" -cf - . | "${SSH[@]}" 'mkdir -p /opt/docker/nginx/html/csw-collector-web && tar -xf - -C /opt/docker/nginx/html/csw-collector-web'

# 样例数据：**只有显式加 `--seed` 才传**（且只在 `--preview` 下）。
#
# 空库上走查等于没查，所以当初预览模式默认就带样例。后果是 09-22 第一次部署时
# 库还不存在，样例就放进了**之后要跑真数据的那个库**：一轮假的「派单轮」（r48 任务#311、
# 12 条候选）混在真数据里，判断台账默认打开的就是它，看门狗也把它当成真的派单轮。
# 09-23 备份后清掉了。以后要样例得自己说要。
if [ "$PREVIEW" = "1" ] && [ "$WITH_SEED" = "1" ]; then
  SEED="${SEED_DB:-/tmp/csw-seed/collector.db}"
  if [ ! -f "$SEED" ]; then
    echo "   造样例数据（$SEED）"
    mkdir -p "$(dirname "$SEED")"
    (cd "$SVC" && cargo run -q -p csw-collector-core --example seed_demo -- "$SEED") ||
      die "造样例数据失败"
  fi
  # WAL 里的内容并进主库再传，否则传过去的是半份
  sqlite3 "$SEED" "PRAGMA wal_checkpoint(TRUNCATE);" >/dev/null 2>&1 || true
  scp_ "$SEED" "$HOST":/opt/csw-collector/upload/seed.db
  echo "   样例数据已上传"
fi

log "4/6 远端配置（env / toml / systemd / nginx）"
"${SSH[@]}" DOMAIN="$DOMAIN" PREVIEW="$PREVIEW" 'bash -s' <<'REMOTE'
set -euo pipefail
cd /opt/csw-collector

# 停服再换：运行中的 ELF 不能原地覆盖
systemctl stop csw-collector 2>/dev/null || true
mv -f upload/csw-collector bin/ && chmod +x bin/csw-collector

# 预览模式的样例库：**只在本来没有库的时候放**，绝不覆盖已经跑出来的数据
if [ "${PREVIEW:-0}" = "1" ] && [ -f upload/seed.db ]; then
  mkdir -p data
  if [ -f data/collector.db ]; then
    echo "   data/collector.db 已存在，样例数据不覆盖（要换就先手动删掉那个库）"
    rm -f upload/seed.db
  else
    mv -f upload/seed.db data/collector.db
    echo "   放好了样例数据"
  fi
fi

# 密钥：仅首次生成，**已有的绝不覆盖**
if [ "${PREVIEW:-0}" = "1" ] && [ ! -f collector.env ]; then
  # 预览模式填的是**非空的假值**。不能留空——启动时 `Secrets::missing()` 判的是
  # 「是不是空字符串」，留空等于缺，服务会直接拒绝启动。
  # 假值拿去调也调不动，正好是预览模式要的。
  cat > collector.env <<'ENV'
# PREVIEW —— 全是假值，调不动任何外部服务。正式部署前整个文件要重写。
CSW_API_KEY=preview-not-a-real-key
SUB2API_API_KEY=preview-not-a-real-key
CSW_ENGINE_TOKEN=preview-not-a-real-key
RUST_LOG=info
ENV
  chmod 600 collector.env
  echo "   写了预览用的 env（全是假值）"
elif [ ! -f collector.env ]; then
  cat > collector.env <<'ENV'
# 密钥只从 env 取，不写进 collector.toml、不进日志。
# 填完 systemctl restart csw-collector。缺哪个启动时会直接报出来。
CSW_API_KEY=
SUB2API_API_KEY=
CSW_ENGINE_TOKEN=
# Jev 不填就是关着，判断退回生成模型
TYPESAFE_API_KEY=
# 运维告警群。谁拿到都能往群里发消息，所以它也算密钥。不填＝告警关着只进日志
CSW_COLLECTOR_ALERT_WEBHOOK=
# 不设就一行日志都没有（默认过滤全关）
RUST_LOG=info
ENV
  chmod 600 collector.env
  echo "   ★ 首次生成 /opt/csw-collector/collector.env（占位），**填完密钥再重启**"
fi

# 预览模式的配置**每次都重写**——它是临时的，而且几项开关正是「不碰生产」的全部依据。
# 但绝不能把正式配置冲掉，所以先看文件头的标记。
if [ "${PREVIEW:-0}" = "1" ]; then
  if [ -f collector.toml ] && ! head -1 collector.toml | grep -q "PREVIEW"; then
    echo "   ✗ collector.toml 是正式配置，预览模式不敢覆盖它。" >&2
    echo "     要么先备份挪走，要么别用 --preview。" >&2
    exit 1
  fi
  cat > collector.toml <<'TOML'
# PREVIEW —— 这是「只看页面」的临时配置，正式部署前要删掉重生成。
#
# 三处让它碰不到生产：
#   1. 运行面地址指向 127.0.0.1:1，**连不通** —— 于是接不到单，也不会去敲生产引擎
#      的运行面。轮询循环第一次是立刻跑的，不这么设就会拿占位 token 去打一次 401。
#   2. 定时任务的时刻全填 99:99 —— 小时要 <24 才解析得出来，解析不出就永不触发。
#      否则 01:40 的预取轮会拿占位密钥去 csw 取数。
#   3. 密钥在 collector.env 里是占位符，真去调也调不动。
#
# 登录仍然转发到真引擎的后台：那是一次只读的 POST /admin/login，
# 用你自己的账号，引擎的 token 只留在这台机器的服务端会话里。

listen = "0.0.0.0:8090"

[engine]
base_url = "http://127.0.0.1:1/api/v1"
admin_url = "https://tasks.aworld.ltd"

[schedule]
prefetch_at_utc = "99:99"
kb_sync_at_utc = ["99:99"]
backup_at_utc = "99:99"

[web]
secure_cookie = true
van_usernames = []

[features]
xhs_collector = false
web_collector = false
TOML
  chmod 600 collector.toml
  echo "   写了预览配置（运行面断开、定时全关）"
elif [ ! -f collector.toml ]; then
  cat > collector.toml <<'TOML'
# env > 本文件 > 内置默认。密钥不在这里。
# 完整字段见 crates/core/src/config.rs，这里只列与默认不同的。

# nginx 在容器里，要经 172.17.0.1 访问宿主，所以不能只听回环。
# 8090 必须挡在公网之外——部署脚本最后会检查一次。
listen = "0.0.0.0:8090"

[web]
# **不配就没有 Van 模式**：引擎里没有 van 这个角色，她登录进来是个普通 viewer，
# 看到的是总览页而不是她那一页。填她在引擎后台的用户名。
van_usernames = []

[features]
# 小红书与网页采集器默认关。开之前 intake_sources 里对应来源要置 required=0，
# 否则覆盖判据会判红
xhs_collector = false
web_collector = false
TOML
  chmod 600 collector.toml
  echo "   首次生成 /opt/csw-collector/collector.toml"
fi

cat > /etc/systemd/system/csw-collector.service <<'UNIT'
[Unit]
Description=csw-collector 情报收集员工作台
# codex 与 MCP 是子进程；向量服务在 docker 里，nginx 也是
After=network.target docker.service

[Service]
WorkingDirectory=/opt/csw-collector
Environment=CSW_COLLECTOR_CONFIG=/opt/csw-collector/collector.toml
EnvironmentFile=/opt/csw-collector/collector.env
ExecStart=/opt/csw-collector/bin/csw-collector serve
Restart=always
RestartSec=5
# 一轮要下一千多张图、跑 codex 子进程与 MCP。4G 给主进程与子进程一起用；
# 实测单进程 RSS 峰值约 90 MB，余量留给深核那几个 codex 线程
MemoryMax=4G
# 这台机器上还有向量服务和 Hermes 的几个网关，别把 CPU 抢光
CPUWeight=30
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target
UNIT

cat > /opt/docker/nginx/conf.d/$DOMAIN.conf <<NG
# csw-collector 工作台：/ 前端 SPA，/api/* → 8090。
# **/healthz 与 /metrics 只给内网**：它们没有鉴权，会露出轮次数、失败数、磁盘百分比。
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

    client_max_body_size    64m;
    # 一轮四十分钟，但 HTTP 这边不干活（写接口只排队），300 秒足够
    proxy_connect_timeout   30s;
    proxy_send_timeout      300s;
    proxy_read_timeout      300s;

    # 工作台接口。每一个都要服务端会话，读接口也不例外
    location /api/ {
        proxy_pass http://172.17.0.1:8090;
        proxy_redirect off;
        proxy_set_header Host              \$host;
        proxy_set_header X-Real-IP         \$remote_addr;
        proxy_set_header X-Forwarded-For   \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        # 会话 cookie 的 HttpOnly / Secure / SameSite 由服务端自己带，nginx 不改
    }

    # 页面要显示服务状态，走 /api/healthz（在上面那个 location /api/ 里，要会话）。
    # 下面这两个裸的**根本不转发**。
    #
    # 一开始这里写的是 allow 127.0.0.1 / 172.16.0.0/12 / 10.0.0.0/8 + deny all，
    # 实测**挡不住**：nginx 跑在 docker 里，公网请求经 docker-proxy 进来之后，
    # 容器看到的 \$remote_addr 是网关 172.17.0.1 —— 正好落在 172.16.0.0/12 里，
    # 于是每一个公网请求都被当成内网放行了。这是 docker NAT 的经典陷阱：
    # **容器内的 IP 判断对外网毫无意义**。
    #
    # 这两个接口没有鉴权，会露出轮次数、失败数、磁盘百分比。要读就从机器内部读
    # （看门狗读的也正是 127.0.0.1:8090），不从公网开口子。
    location = /healthz { return 403; }
    location = /metrics { return 403; }

    location / {
        root /usr/share/nginx/html/csw-collector-web;
        try_files \$uri /index.html;
    }

    access_log /data/www/nginx/log/$DOMAIN.log;
}
NG

systemctl daemon-reload
systemctl enable --now csw-collector >/dev/null 2>&1
systemctl restart csw-collector

# nginx 校验不过就**把刚写的站点删掉**再退出。
# 留着一个坏配置比不部署严重得多：下一个人 reload nginx 时整台机器的站点一起失败，
# 而那时没人会想到是这个新站点干的。
if docker exec base-nginx nginx -t >/dev/null 2>&1; then
  docker exec base-nginx nginx -s reload
  echo "   服务: csw-collector=$(systemctl is-active csw-collector) nginx=reloaded"
else
  rm -f "/opt/docker/nginx/conf.d/$DOMAIN.conf"
  echo "   ✗ nginx 校验不过，已把 $DOMAIN.conf 删掉（其余站点没受影响）" >&2
  docker exec base-nginx nginx -t 2>&1 | tail -5 >&2
  exit 1
fi

# Hermes 那边一根头发都不该动，确认一下
echo "   Hermes 主网关: $(systemctl --user is-active hermes-gateway.service 2>/dev/null || echo '读不到（不是 root 的用户级单元，正常）')"
REMOTE

log "5/6 检查"
# ⚠️ **下面这些检查没有一个位置是完全可信的**，踩过两次才弄明白：
#
#   - 从**本机**发：开发机挂着代理（DNS 被劫持到 198.18.x.x），
#     代理给的响应与服务器上真实的响应能差出一个状态码——曾经报过一次假 403。
#   - 从**服务器上**发：它访问不了自己的公网域名（发夹弯 NAT，云厂商常见），
#     一律 000——曾经据此误判成「全挂了」。
#
# 所以这里从本机发，把它当**参照而不是结论**；真要确认安全边界（尤其 8090
# 有没有对公网敞着），用一条第三方线路：手机流量、另一台机器、或在线端口扫描。

# 第一次启动要建 LanceDB 的表，比平时慢，所以等得久一点
printf '   healthz（内网）: %s\n' "$("${SSH[@]}" 'curl -fsS --retry 45 --retry-connrefused --retry-delay 2 --max-time 10 http://127.0.0.1:8090/healthz' || echo '✗ 起不来，上去看 journalctl -u csw-collector -n 50')"
printf '   前端:            HTTP %s\n' "$(curl -s -o /dev/null -w '%{http_code}' "https://$DOMAIN/")"
printf '   接口未登录:      HTTP %s（401 即正常——读接口也要会话）\n' "$(curl -s -o /dev/null -w '%{http_code}' "https://$DOMAIN/api/rounds")"
printf '   /metrics 公网:   HTTP %s（403 即正常）\n' "$(curl -s -o /dev/null -w '%{http_code}' "https://$DOMAIN/metrics")"

# 8090 直连必须打不通。nginx 的 allow/deny 只管经过它的流量，
# 端口本身若对公网开着，绕过 nginx 就什么都读得到。
#
# ⚠️ **这一条是从本机发的，而本机可能有代理**——代理拦下请求的话，这里会显示
# 「打不通」，而实际上它对公网敞着。也就是说**这条检查给得出假的安全结论**。
# 真要确认，用一条不经代理的线路（手机流量、另一台机器）再试一次：
#     curl -m5 -o /dev/null -w '%{http_code}\n' http://<公网IP>:8090/metrics
HOSTIP="${HOST#*@}"
RAW="$(curl -s -m 5 -o /dev/null -w '%{http_code}' "http://$HOSTIP:8090/metrics" || echo 000)"
if [ "$RAW" = "200" ]; then
  printf '\n\033[1;31m✗ 8090 从公网直接打通了（HTTP 200）。\n'
  printf '  /metrics 与 /healthz 没有鉴权，现在等于公开。\n'
  printf '  处置：给这台机的安全组或 firewalld 关掉 8090 的入站，只留 443。\033[0m\n'
else
  printf '   8090 公网直连:   %s（打不通即正常，但见上面那条注意）\n' "$RAW"
fi

log "6/6 完成 → https://$DOMAIN"
if [ "$PREVIEW" = "1" ]; then
cat <<NEXT

   **预览模式**，碰不到生产：运行面断开（接不到单）、定时全关。
   collector.env 只在首次生成占位、已有的绝不覆盖——远端要是已经填了真密钥，
   它们原样留着；库也只在首次且带 --seed 时放样例，已有的库原样留着。

   打开 https://$DOMAIN ，用**你自己在 tasks.aworld.ltd 后台的账号**登录。
   工作台这边不存口令：它把登录转发给引擎，引擎的 token 只留在服务端会话里。

   看不到 Van 模式那一页是对的 —— 它要一个映射成 van 的 viewer 账号，这次没建。

   走查完、确认页面没问题之后，正式部署是同一条命令去掉 --preview，
   那时候要先备份挪走这份预览配置，并按切换清单逐项来。

NEXT
else
cat <<'NEXT'

   接下来不在本脚本里的几步（都要单独取得同意）：
   1. 填 /opt/csw-collector/collector.env 的密钥，systemctl restart csw-collector
   2. 用**同一行 collector agent** 再签一枚运行面 token（新建 agent 接不到单：
      任务在触发时就钉死到 ActiveAgentByRole 返回的那一行）
   3. 夜间首次建知识库（约两小时）
   4. 放行验证：影子轮跑一遍，结果写引擎副本，不碰生产引擎、不群播报
   5. 切换清单：intake_sources 的 xhs/web 置 required=0 → wfctl activate daily_news@9
      → 重跑 gen_stage_docs.py → 停 hermes-gateway-collector.service
NEXT
fi
