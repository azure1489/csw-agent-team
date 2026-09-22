#!/usr/bin/env bash
# 本机把整套起起来，专为**在浏览器里走查十一页**。
#
#   ./dev/walkthrough.sh              # 起：引擎后台 + 采集服务 + 前端，并造一轮样例数据
#   ./dev/walkthrough.sh --password '你自己定的口令'
#   Ctrl-C 全停
#
# 起的都是**本机临时的**：引擎库在 /tmp/csw-demo/e.db，采集库在 /tmp/csw-demo/cdata。
# 不碰生产、不连 csw、不调模型（密钥填的是占位符，知识库那几页会如实回 503）。
#
# 为什么要造样例数据：空库上走查等于没查——每页都是「还没有数据」，
# 看不出表格对不对齐、四档配色分不分得开、长正文会不会撑破卡片。
set -uo pipefail
# 提醒：**变量后面紧跟中文标点时一定要写 ${VAR}**。裸 $VAR 后面跟「（」「，」这类
# 全角字符时，bash 会把它们当成变量名的一部分，`set -u` 下直接崩在一句 echo 上。
# 这坑在看门狗脚本里踩过一次，这里又踩了一次。

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
DEMO="${DEMO_DIR:-/tmp/csw-demo}"
PASSWORD=""
USER="walkthrough"
VAN_USER="van-demo"

while [ $# -gt 0 ]; do
  case "$1" in
    --password) PASSWORD="${2:-}"; shift 2 ;;
    --user) USER="${2:-}"; shift 2 ;;
    *) echo "不认识的参数：$1"; exit 2 ;;
  esac
done

log() { printf '\n\033[1;36m── %s\033[0m\n' "$*"; }
PIDS=()
cleanup() {
  printf '\n\033[1;33m停掉后台进程…\033[0m\n'
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done
  wait 2>/dev/null
  echo "都停了。临时数据还在 ${DEMO}，要清就 rm -rf ${DEMO}"
}
trap cleanup EXIT INT TERM

mkdir -p "$DEMO"

log "1/5 构建（引擎 Go + 采集服务 Rust）"
(cd "$REPO/csw-task/csw-task-svc" && make build >/dev/null) || { echo "引擎构建失败"; exit 1; }
if [ ! -x "$HERE/target/debug/csw-collector" ]; then
  echo "   采集服务是第一次编（或刚 cargo clean 过）——lance 那套依赖很大，"
  echo "   **要几分钟**，中间没输出是正常的，别以为卡住了。"
fi
(cd "$HERE" && cargo build -p csw-collector) || { echo "采集服务构建失败"; exit 1; }

log "2/5 引擎后台（临时库 $DEMO/e.db）"
# CSW_DB_PATH 一定要 export：adminctl 认不出的参数会被当成默认行为，
# 它会在 csw-task-svc/data/ 下**另建一个库**跑迁移，而你以为动的是临时库
export CSW_DB_PATH="$DEMO/e.db"
ADMINCTL="$REPO/csw-task/csw-task-svc/bin/adminctl"
"$ADMINCTL" migrate up >/dev/null
[ -n "$PASSWORD" ] || PASSWORD="$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 16)"
if "$ADMINCTL" user create "$USER" --role superadmin --password "$PASSWORD" >/dev/null 2>&1; then
  echo "   建了后台账号 ${USER}（superadmin，看得到十页）"
else
  # 已经建过了（这个临时库是留着的，重跑脚本不会重建）
  echo "   账号 $USER 已存在，口令用你上次那个"
  PASSWORD=""
fi
# **Van 模式那一页要单独一个账号**：引擎里没有 van 这个角色，
# 它是「配置里列出的 viewer 用户名」映射来的。不建这个就走查不到那一页。
if "$ADMINCTL" user create "$VAN_USER" --role viewer --password "$PASSWORD" >/dev/null 2>&1; then
  echo "   建了后台账号 ${VAN_USER}（viewer → 映射成 van，登录后直接进 Van 那一页）"
fi
CSW_JWT_SECRET=localdemo CSW_ADMIN_ADDR=127.0.0.1:18081 \
  "$REPO/csw-task/csw-task-svc/bin/adminsrv" > "$DEMO/adminsrv.log" 2>&1 &
PIDS+=($!)

# van_usernames 只有配置文件能配（env 没有这一项），所以这里生成一份临时的
cat > "$DEMO/collector.toml" <<TOML
data_dir = "$DEMO/cdata"
listen = "127.0.0.1:8090"

[engine]
base_url = "http://127.0.0.1:18080/api/v1"
admin_url = "http://127.0.0.1:18081"

[web]
# 本机走的是 http，不是 https——Secure 的 cookie 浏览器不会回传，登录会一直转圈
secure_cookie = false
van_usernames = ["$VAN_USER"]
TOML

log "3/5 造一轮样例数据"
(cd "$HERE" && cargo run -q -p csw-collector-core --example seed_demo -- "$DEMO/cdata/collector.db")

log "4/5 采集服务（8090，Vite 的代理指着它）"
RUST_LOG=info \
CSW_COLLECTOR_CONFIG="$DEMO/collector.toml" \
CSW_ENGINE_TOKEN=placeholder CSW_API_KEY=placeholder SUB2API_API_KEY=placeholder \
  "$HERE/target/debug/csw-collector" serve > "$DEMO/collector.log" 2>&1 &
PIDS+=($!)

for i in $(seq 1 40); do
  curl -s -m1 http://127.0.0.1:8090/healthz >/dev/null 2>&1 && break
  sleep 1
done
curl -s -m2 http://127.0.0.1:8090/healthz >/dev/null 2>&1 ||
  { echo "   采集服务没起来，看 $DEMO/collector.log"; exit 1; }
echo "   healthz 通了"

log "5/5 前端"
cat <<INFO

  打开 http://localhost:5174  （不是 127.0.0.1——Vite 只绑 localhost）

    看十页        账号 $USER
    看 Van 模式   账号 $VAN_USER
    口令（两个一样） ${PASSWORD:-（你上次定的）}

  Van 那个登录后会直接跳到她那一页，看不到运行与设置——那正是要走查的。

  日志：$DEMO/collector.log · $DEMO/adminsrv.log
  Ctrl-C 全停。

INFO
cd "$REPO/csw-task/csw-collector-web"
[ -d node_modules ] || pnpm install
npx vite --port 5174 --strictPort
