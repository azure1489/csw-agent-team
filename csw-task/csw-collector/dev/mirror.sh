#!/usr/bin/env bash
# 用**生产数据的镜像**在本机起工作台，在浏览器里看生产页面长什么样。
#
#   ./dev/mirror.sh                 # 拉一份生产库 + 向量库，起本机引擎后台 + 工作台 + 前端
#   ./dev/mirror.sh --no-fetch      # 用上次拉下来的镜像，不再拉
#   Ctrl-C 全停
#
# 为什么不直接登生产页面：工作台登录走引擎后台账号，公网登录页上不输密码（浏览器操作的规矩）。
# 刊译台那边同理：接口检查走 verify-bot（scripts/prod-api.sh），页面看本机镜像。
#
# 碰不到生产的几处（与 deploy-collector.sh --preview 同一套）：
#   1. 运行面地址指向 127.0.0.1:1，连不通：不接单、不心跳、outbox 发不出去。
#   2. 定时任务时刻全填 99:99：预取轮、知识库同步、备份永不触发。
#   3. 密钥是占位符；登录走**本机临时引擎后台**（临时账号、随机口令），不碰生产引擎。
#   4. 数据是 sqlite .backup 出来的副本（向量库是目录拷贝），本机怎么写都不回生产。
# 唯一连到生产的是向量服务：ssh -L 把本机 18022 转到 agent 主机 127.0.0.1:8022，
# 文字检索算查询向量、以图搜图要用它。**它占生产 GPU**——别在正式轮（北京 05:25–06:30）里点以图搜图。
# 杂志回填在本机镜像里也会跑（库里有没算完的就会算），同样占生产 GPU；生产那边算完了，镜像里也就没活。
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
MIRROR="${MIRROR_DIR:-/tmp/csw-mirror}"
HOST="${MIRROR_HOST:-root@8.138.23.218}"
PORT="${MIRROR_PORT:-9318}"
USER="mirror"
FETCH=1
[ "${1:-}" = "--no-fetch" ] && FETCH=0

log() { printf '\n\033[1;36m── %s\033[0m\n' "$*"; }
PIDS=()
cleanup() {
  printf '\n\033[1;33m停掉后台进程…\033[0m\n'
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done
  wait 2>/dev/null
  echo "都停了。镜像还在 ${MIRROR}（约 1G），要清就 rm -rf ${MIRROR}"
}
trap cleanup EXIT INT TERM
SSH=(ssh -p "$PORT" -o LogLevel=ERROR "$HOST")
mkdir -p "$MIRROR/cdata"

if [ "$FETCH" = 1 ]; then
  log "1/5 拉生产镜像（库用 .backup，向量库整目录；不拉图片缓存）"
  avail=$(df -k "$MIRROR" | awk 'NR==2{print $4}')
  [ "$avail" -gt 4000000 ] || { echo "本机剩余空间不到 4G，先腾空间（10-08 曾经满盘）"; exit 1; }
  "${SSH[@]}" 'rm -f /tmp/collector-mirror.db; /root/miniconda3/bin/sqlite3 /opt/csw-collector/data/collector.db ".backup /tmp/collector-mirror.db"' \
    || { echo "生产库备份失败"; exit 1; }
  scp -q -P "$PORT" -o LogLevel=ERROR "$HOST:/tmp/collector-mirror.db" "$MIRROR/cdata/collector.db" || exit 1
  "${SSH[@]}" 'rm -f /tmp/collector-mirror.db'
  rm -rf "$MIRROR/cdata/lance"
  "${SSH[@]}" 'tar -C /opt/csw-collector/data -cf - lance' | tar -C "$MIRROR/cdata" -xf - || exit 1
  echo "   库 $(du -sh "$MIRROR/cdata/collector.db" | cut -f1)，向量库 $(du -sh "$MIRROR/cdata/lance" | cut -f1)"
fi
[ -f "$MIRROR/cdata/collector.db" ] || { echo "没有镜像，去掉 --no-fetch 再跑"; exit 1; }

log "2/5 构建（引擎后台 Go + 工作台 Rust，用当前工作区的代码）"
(cd "$REPO/csw-task/csw-task-svc" && make build >/dev/null) || { echo "引擎构建失败"; exit 1; }
(cd "$HERE" && cargo build -p csw-collector) || { echo "工作台构建失败"; exit 1; }

log "3/5 本机临时引擎后台（只用来登录）"
export CSW_DB_PATH="$MIRROR/e.db"
ADMINCTL="$REPO/csw-task/csw-task-svc/bin/adminctl"
"$ADMINCTL" migrate up >/dev/null
if [ ! -f "$MIRROR/pw" ]; then
  LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 16 >"$MIRROR/pw"
  chmod 600 "$MIRROR/pw"
  "$ADMINCTL" user create "$USER" --role superadmin --password "$(cat "$MIRROR/pw")" >/dev/null
fi
CSW_JWT_SECRET=localmirror CSW_ADMIN_ADDR=127.0.0.1:18081 \
  "$REPO/csw-task/csw-task-svc/bin/adminsrv" >"$MIRROR/adminsrv.log" 2>&1 &
PIDS+=($!)

log "4/5 向量服务隧道（本机 18022 → agent 主机 8022）+ 工作台（8090）"
"${SSH[@]}" -N -L 18022:127.0.0.1:8022 &
PIDS+=($!)
cat >"$MIRROR/collector.toml" <<TOML
# 本机镜像：只看页面。说明见 dev/mirror.sh 开头
data_dir = "$MIRROR/cdata"
listen = "127.0.0.1:8090"

[engine]
base_url = "http://127.0.0.1:1/api/v1"
admin_url = "http://127.0.0.1:18081"

[vector]
base_url = "http://127.0.0.1:18022"

[schedule]
prefetch_at_utc = "99:99"
kb_sync_at_utc = ["99:99"]
backup_at_utc = "99:99"

[web]
secure_cookie = false
van_usernames = []

[features]
xhs_collector = false
web_collector = false
TOML
RUST_LOG=info CSW_COLLECTOR_CONFIG="$MIRROR/collector.toml" \
  CSW_ENGINE_TOKEN=placeholder CSW_API_KEY=placeholder SUB2API_API_KEY=placeholder \
  "$HERE/target/debug/csw-collector" serve >"$MIRROR/collector.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 60); do curl -s -m1 http://127.0.0.1:8090/healthz >/dev/null 2>&1 && break; sleep 1; done
curl -s -m2 http://127.0.0.1:8090/healthz >/dev/null 2>&1 || { echo "工作台没起来，看 $MIRROR/collector.log"; exit 1; }

log "5/5 前端（http://localhost:5174）"
cat <<INFO

  打开 http://localhost:5174 ，账号 $USER，口令在 $MIRROR/pw（本机临时账号）。
  日志：$MIRROR/collector.log · $MIRROR/adminsrv.log
  Ctrl-C 全停。

INFO
cd "$REPO/csw-task/csw-collector-web"
[ -d node_modules ] || pnpm install
npx vite --port 5174 --strictPort
