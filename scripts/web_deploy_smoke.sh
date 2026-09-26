#!/usr/bin/env bash
# 用独立 Compose 项目演练控制台启动、升级、前端回滚和上一版组合回滚。
# 清理只删除本次新建的项目、卷和镜像标签。
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
id="asterlane-smoke-$$"
project="$id"
workdir=$(mktemp -d "${TMPDIR:-/tmp}/${id}.XXXXXX")
gw_prev="${id}-gateway-prev"
gw_next="${id}-gateway-next"
web_a="${id}-web-a"
web_b="${id}-web-b"
compose_file="$workdir/compose.yaml"
token_file="$workdir/admin-token"
config_file="$workdir/gateway.yaml"
auth_file="$workdir/curl-auth"
gw_port=""
web_port=""
prev_dist=""

cleanup() {
  status=$?
  case "$project" in
    asterlane-smoke-*) ;;
    *)
      printf '%s\n' "拒绝清理意外项目名" >&2
      exit 1
      ;;
  esac
  if [[ "$status" -ne 0 && -f "$compose_file" ]]; then
    TOKEN_FILE="$token_file" docker compose -p "$project" -f "$compose_file" logs --tail 80 2>&1 \
      | TOKEN_FILE="$token_file" python3 -c 'import os, pathlib, sys
token = ""
path = os.environ.get("TOKEN_FILE", "")
if path and os.path.exists(path):
    token = pathlib.Path(path).read_text().strip()
data = sys.stdin.read()
sys.stderr.write(data.replace(token, "[redacted]") if token else data)' || true
  fi
  if [[ -f "$compose_file" ]]; then
    docker compose -p "$project" -f "$compose_file" down -v --remove-orphans >/dev/null 2>&1 || true
  fi
  while IFS= read -r volume; do
    [[ -z "$volume" ]] && continue
    case "$volume" in
      "${project}"_*) docker volume rm "$volume" >/dev/null 2>&1 || true ;;
    esac
  done < <(docker volume ls -q 2>/dev/null || true)
  docker rmi "$gw_next" "$gw_prev" "$web_a" "$web_b" >/dev/null 2>&1 || true
  rm -rf "$workdir"
  exit "$status"
}
trap cleanup EXIT

docker info >/dev/null
if docker compose ls -q | grep -qx "$project"; then
  printf '%s\n' "测试项目名已存在，停止" >&2
  exit 1
fi

env -u ASTERLANE_ADMIN_TOKEN docker compose -f "$root/compose.yaml" config >/dev/null
python3 - "$root/compose.yaml" <<'PY'
import pathlib, sys
text = pathlib.Path(sys.argv[1]).read_text()
web = text.split("\n  web:", 1)
if len(web) != 2:
    sys.exit("compose.yaml 缺少 web 服务")
if "ADMIN_TOKEN" in web[1] or "admin-token" in web[1]:
    sys.exit("web 服务不能持有 admin token")
for needle in (
    "127.0.0.1:${ASTERLANE_GATEWAY_PORT:-3721}:3000",
    "127.0.0.1:${ASTERLANE_WEB_PORT:-3722}:8080",
    "ASTERLANE_GATEWAY_UPSTREAM: gateway:3000",
    "gateway-data:",
):
    if needle not in text:
        sys.exit(f"compose.yaml 缺少 {needle}")
PY

umask 077
openssl rand -hex 24 > "$token_file"
printf 'header = "Authorization: Bearer %s"\n' "$(tr -d '\n' < "$token_file")" > "$auth_file"
cat > "$config_file" <<'EOF'
admin:
  keys:
    - id: smoke-ops
      token_ref: secret://file//run/secrets/admin_token
api_resources: []
proxy_keys: []
EOF

free_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}
gw_port=$(free_port)
web_port=$(free_port)
if [[ "$gw_port" == "$web_port" ]]; then
  web_port=$(free_port)
fi

commit=$(git -C "$root" rev-parse HEAD)
if [[ ! "$commit" =~ ^[0-9a-f]{40}$ ]]; then
  commit=unknown
fi

if [[ -n "${ASTERLANE_SMOKE_GATEWAY_IMAGE:-}" ]]; then
  docker image inspect "$ASTERLANE_SMOKE_GATEWAY_IMAGE" >/dev/null
  docker tag "$ASTERLANE_SMOKE_GATEWAY_IMAGE" "$gw_prev"
else
  docker build --build-arg "GIT_COMMIT=${commit}" --label "asterlane.smoke=${id}" -t "$gw_prev" "$root"
fi
docker build -t "$gw_next" - <<EOF
FROM ${gw_prev}
LABEL asterlane.smoke=${id}
LABEL asterlane.smoke-role=next
EOF
docker build -f "$root/web/Dockerfile" --build-arg ASSET_LABEL=smoke-a --build-arg "GIT_COMMIT=${commit}" \
  --label "asterlane.smoke=${id}" -t "$web_a" "$root/web"
docker build -f "$root/web/Dockerfile" --build-arg ASSET_LABEL=smoke-b --build-arg "GIT_COMMIT=${commit}" \
  --label "asterlane.smoke=${id}" -t "$web_b" "$root/web"

docker run --rm --entrypoint sh "$gw_prev" -c \
  'if command -v node >/dev/null || command -v bun >/dev/null || command -v nginx >/dev/null; then exit 1; fi; test -x /usr/local/bin/asterlane'
docker run --rm --entrypoint cat "$gw_prev" /etc/asterlane-build-info | grep -q "git_commit=${commit}"
docker run --rm --entrypoint sh "$web_a" -c \
  'if command -v node >/dev/null || command -v bun >/dev/null || command -v cargo >/dev/null; then exit 1; fi; test -s /usr/share/nginx/html/index.html'
docker run --rm --entrypoint cat "$web_a" /usr/share/nginx/html/build-info.json \
  | python3 -c 'import json,sys; info=json.load(sys.stdin); assert info["node"]=="22.23.1"; assert info["bun"]=="1.4.2"; assert info["vp"]=="1.0.0-rc.0"; assert info["nginx"]=="1.28.1"'
docker run --rm --entrypoint cat "$web_a" /usr/local/share/asterlane-web/retain.md | grep -q "发布周期"
docker run --rm --entrypoint cat "$web_a" /usr/local/share/asterlane-web/retain.md | grep -q "service worker"
python3 - "$web_a" "$token_file" <<'PY'
import json, pathlib, subprocess, sys
image, token_path = sys.argv[1:]
token = pathlib.Path(token_path).read_text().strip()
raw = subprocess.check_output(["docker", "image", "inspect", image], text=True)
if token and token in raw:
    sys.exit("前端镜像元数据含有测试 token")
info = json.loads(raw)[0]
env = info["Config"].get("Env") or []
if any(item.startswith("ASTERLANE_ADMIN_TOKEN=") for item in env):
    sys.exit("前端镜像环境含有 ASTERLANE_ADMIN_TOKEN")
PY

cid=$(docker create "$web_a")
docker cp "$cid:/usr/share/nginx/html" "$workdir/prev-src"
docker rm "$cid" >/dev/null
if [[ -f "$workdir/prev-src/index.html" ]]; then
  prev_dist="$workdir/prev-src"
elif [[ -f "$workdir/prev-src/html/index.html" ]]; then
  prev_dist="$workdir/prev-src/html"
else
  printf '%s\n' "无法提取上一版静态文件" >&2
  exit 1
fi

write_compose() {
  local gateway_image="$1" web_image="$2" previous="$3"
  {
    cat <<EOF
services:
  gateway:
    image: ${gateway_image}
    restart: "no"
    ports:
      - "127.0.0.1:${gw_port}:3000"
    volumes:
      - ${config_file}:/etc/asterlane/config.yaml:ro
      - smoke-data:/data
    secrets:
      - admin_token
    command:
      - serve
      - --config=/etc/asterlane/config.yaml
      - --database-url=sqlite:/data/asterlane.db?mode=rwc
      - --bind=0.0.0.0:3000
  web:
    image: ${web_image}
    restart: "no"
    depends_on:
      gateway:
        condition: service_healthy
    ports:
      - "127.0.0.1:${web_port}:8080"
    environment:
      ASTERLANE_GATEWAY_UPSTREAM: gateway:3000
EOF
    if [[ -n "$previous" ]]; then
      printf '    volumes:\n      - %s:/previous-dist:ro\n' "$previous"
    fi
    cat <<EOF
secrets:
  admin_token:
    file: ${token_file}
volumes:
  smoke-data:
EOF
  } > "$compose_file"
}

wait_healthy() {
  local deadline=$((SECONDS + 180)) gw web
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    gw=$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}' "${project}-gateway-1" 2>/dev/null || echo missing)
    web=$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}' "${project}-web-1" 2>/dev/null || echo missing)
    if [[ "$gw" == healthy && "$web" == healthy ]]; then
      return 0
    fi
    sleep 2
  done
  docker compose -p "$project" -f "$compose_file" ps >&2 || true
  return 1
}

fetch() {
  curl -sS -o "$workdir/body" -D "$workdir/headers" -w '%{http_code}' "$@"
}

header_has() {
  local name="$1" needle="$2"
  awk -v name="$name" -v needle="$needle" '
    BEGIN { found = 0 }
    tolower($0) ~ "^" tolower(name) ":" && index(tolower($0), tolower(needle)) { found = 1 }
    END { exit !found }
  ' "$workdir/headers"
}

not_spa() {
  ! grep -q 'id="root"' "$workdir/body"
  ! grep -q '星径控制台' "$workdir/body"
}

asset_path() {
  python3 - "$1" <<'PY'
import pathlib, re, sys
html = pathlib.Path(sys.argv[1]).read_text()
match = re.search(r'/assets/[^"]+\.js', html)
if match is None:
    sys.exit("index 里没有脚本")
print(match.group(0))
PY
}

write_compose "$gw_prev" "$web_a" ""
docker compose -p "$project" -f "$compose_file" up -d --remove-orphans --force-recreate
wait_healthy
code=$(fetch "http://127.0.0.1:${web_port}/")
[[ "$code" == 200 ]]
header_has content-type text/html
header_has cache-control no-cache
header_has content-security-policy "default-src 'self'"
header_has x-frame-options DENY
header_has referrer-policy no-referrer
cp "$workdir/body" "$workdir/index-a.html"
script_a=$(asset_path "$workdir/index-a.html")
code=$(fetch "http://127.0.0.1:${web_port}${script_a}")
[[ "$code" == 200 ]]
header_has cache-control immutable
cp "$workdir/body" "$workdir/script-a.js"
code=$(fetch "http://127.0.0.1:${web_port}/usage")
[[ "$code" == 200 ]]
grep -q 'id="root"' "$workdir/body"
code=$(fetch "http://127.0.0.1:${web_port}/assets/missing-aaaaaaaa.js")
[[ "$code" == 404 ]]
not_spa
code=$(fetch "http://127.0.0.1:${web_port}/admin/ui")
[[ "$code" == 308 ]]
python3 - "$workdir/headers" <<'PY'
import pathlib, sys
location = ""
for line in pathlib.Path(sys.argv[1]).read_text().splitlines():
    if line.lower().startswith("location:"):
        location = line.split(":", 1)[1].strip()
path = location.split("?", 1)[0]
if path.startswith("http://") or path.startswith("https://"):
    path = "/" + path.split("/", 3)[-1]
if path != "/":
    sys.exit(f"unexpected location {location}")
PY
code=$(fetch "http://127.0.0.1:${web_port}/admin/ui/")
[[ "$code" == 308 ]]
code=$(fetch "http://127.0.0.1:${web_port}/admin/ui/core.js")
[[ "$code" == 404 ]]
not_spa
code=$(fetch "http://127.0.0.1:${web_port}/admin/health")
[[ "$code" == 401 ]]
header_has content-type application/json
header_has cache-control no-store
grep -q admin.unauthorized "$workdir/body"
not_spa
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/not-a-real-route")
[[ "$code" == 404 ]]
header_has cache-control no-store
not_spa
printf '%s\n' '{"id":"smoke-res","domain":"search","provider":"example","base_url":"https://example.invalid","description":"smoke"}' > "$workdir/resource.json"
code=$(fetch -K "$auth_file" -H 'content-type: application/json' --data-binary @"$workdir/resource.json" \
  -X POST "http://127.0.0.1:${web_port}/admin/resources")
[[ "$code" == 200 ]]
grep -q smoke-res "$workdir/body"
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/usage?group_by=tool")
[[ "$code" == 200 ]]
python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["group_by"]=="tool"' "$workdir/body"
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/config/export")
[[ "$code" == 200 ]]
header_has content-type yaml
grep -q smoke-res "$workdir/body"
python3 - "$workdir/body" "$token_file" <<'PY'
import pathlib, sys
body = pathlib.Path(sys.argv[1]).read_text()
token = pathlib.Path(sys.argv[2]).read_text().strip()
if token in body:
    sys.exit("导出的 YAML 含有 admin token")
PY
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/security-events?kind=admin_audit")
[[ "$code" == 200 ]]
grep -q smoke-res "$workdir/body"
code=$(fetch "http://127.0.0.1:${gw_port}/healthz")
[[ "$code" == 200 ]]
python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["status"]=="ok"' "$workdir/body"
code=$(fetch "http://127.0.0.1:${web_port}/healthz")
[[ "$code" == 404 ]]
! grep -q '"status"' "$workdir/body"
code=$(fetch "http://127.0.0.1:${gw_port}/admin/ui")
[[ "$code" == 404 ]]
not_spa
code=$(fetch "http://127.0.0.1:${gw_port}/")
[[ "$code" == 404 ]]
if awk 'BEGIN{IGNORECASE=1} /^location:/ {found=1} END{exit !found}' "$workdir/headers"; then
  printf '%s\n' "网关根路径不应再重定向到旧控制台" >&2
  exit 1
fi
code=$(fetch -K "$auth_file" "http://127.0.0.1:${gw_port}/admin/health")
[[ "$code" == 200 ]]
code=$(fetch "http://127.0.0.1:${web_port}/mcp")
[[ "$code" == 404 ]]
not_spa
code=$(fetch "http://127.0.0.1:${web_port}/v1/tools")
[[ "$code" == 404 ]]
not_spa
python3 - "$token_file" "${project}-web-1" <<'PY'
import json, pathlib, subprocess, sys
token = pathlib.Path(sys.argv[1]).read_text().strip()
raw = subprocess.check_output(["docker", "inspect", sys.argv[2]], text=True)
if token and token in raw:
    sys.exit("前端容器含有测试 token")
env = json.loads(raw)[0]["Config"].get("Env") or []
if any(item.startswith("ASTERLANE_ADMIN_TOKEN=") for item in env):
    sys.exit("前端容器环境含有 ASTERLANE_ADMIN_TOKEN")
PY

write_compose "$gw_next" "$web_b" "$prev_dist"
docker compose -p "$project" -f "$compose_file" up -d --remove-orphans --force-recreate
wait_healthy
code=$(fetch "http://127.0.0.1:${web_port}/")
[[ "$code" == 200 ]]
cp "$workdir/body" "$workdir/index-b.html"
! cmp -s "$workdir/index-a.html" "$workdir/index-b.html"
script_b=$(asset_path "$workdir/index-b.html")
[[ "$script_a" != "$script_b" ]]
code=$(fetch "http://127.0.0.1:${web_port}${script_a}")
[[ "$code" == 200 ]]
cmp -s "$workdir/script-a.js" "$workdir/body"
code=$(fetch "http://127.0.0.1:${web_port}${script_b}")
[[ "$code" == 200 ]]
header_has cache-control immutable
docker logs "${project}-web-1" > "$workdir/web.log" 2>&1 || true
grep -q 'retain: assets/' "$workdir/web.log"
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/resources")
[[ "$code" == 200 ]]
grep -q smoke-res "$workdir/body"

write_compose "$gw_next" "$web_a" ""
docker compose -p "$project" -f "$compose_file" up -d --remove-orphans --no-deps --force-recreate web
wait_healthy
code=$(fetch "http://127.0.0.1:${web_port}/")
[[ "$code" == 200 ]]
cmp -s "$workdir/body" "$workdir/index-a.html"
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/resources")
grep -q smoke-res "$workdir/body"

write_compose "$gw_prev" "$web_a" ""
docker compose -p "$project" -f "$compose_file" up -d --remove-orphans --force-recreate
wait_healthy
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/resources")
grep -q smoke-res "$workdir/body"
code=$(fetch -K "$auth_file" -X DELETE "http://127.0.0.1:${web_port}/admin/resources/smoke-res")
[[ "$code" == 200 ]]
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/resources")
! grep -q smoke-res "$workdir/body"

docker compose -p "$project" -f "$compose_file" stop gateway
sleep 2
[[ "$(docker inspect -f '{{.State.Status}}' "${project}-gateway-1")" == exited ]]
[[ "$(docker inspect -f '{{.State.Health.Status}}' "${project}-web-1")" == healthy ]]
code=$(fetch "http://127.0.0.1:${web_port}/")
[[ "$code" == 200 ]]
grep -q 'id="root"' "$workdir/body"
code=$(fetch -K "$auth_file" "http://127.0.0.1:${web_port}/admin/health")
[[ "$code" == 503 ]]
header_has content-type application/json
header_has cache-control no-store
grep -q gateway.unavailable "$workdir/body"
not_spa
code=$(fetch "http://127.0.0.1:${gw_port}/healthz" || true)
[[ "$code" != 200 ]]

printf '%s\n' "web-deploy-smoke ok"
