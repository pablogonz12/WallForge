#!/usr/bin/env bash
# Reproduce the numbers in the README's Performance table.
#
#   tools/bench.sh                      # idle baseline + every video and up to 3 scenes
#   tools/bench.sh 1315534440 3244466773  # only these Workshop ids
#
# Needs the renderer built (render/target/release/wallforge-render), a Wayland session and
# /sys/class/drm/card*/device/gpu_busy_percent (amdgpu). Override with:
#   WF_OUTPUT=DP-1 WF_SECONDS=12 WF_ROUNDS=20 tools/bench.sh
set -u
cd "$(dirname "$0")/.." || exit 1

RENDER=${WF_RENDER:-render/target/release/wallforge-render}
OUTPUT=${WF_OUTPUT:-$(niri msg outputs 2>/dev/null | sed -n 's/^Output ".*" (\([A-Z0-9-]*\)).*/\1/p' | head -1)}
SECONDS_PER=${WF_SECONDS:-10}
ROUNDS=${WF_ROUNDS:-16}
WS=${WF_WS:-$HOME/.local/share/Steam/steamapps/workshop/content/431960}
GPU=$(ls /sys/class/drm/card*/device/gpu_busy_percent 2>/dev/null | head -1)

if [ ! -x "$RENDER" ]; then echo "renderer not built: $RENDER" >&2; exit 1; fi
if [ -z "$OUTPUT" ]; then echo "no output found; set WF_OUTPUT" >&2; exit 1; fi
if [ ! -d "$WS" ]; then echo "no Workshop library at $WS" >&2; exit 1; fi

cpu_ticks() { awk '{print $14+$15}' "/proc/$1/stat" 2>/dev/null || echo 0; }

sample() { # sample <pid> -> "gpu% cpu% rssMB"; CPU is over the sampling window, as % of one core
    local pid=$1 gpu=0 rss=0 i
    local tck; tck=$(getconf CLK_TCK)
    local t0 cpu0; t0=$(date +%s%N); cpu0=$(cpu_ticks "$pid")
    for i in $(seq 1 "$ROUNDS"); do
        [ -n "$GPU" ] && gpu=$((gpu + $(cat "$GPU")))
        rss=$((rss + $(ps -o rss= -p "$pid" 2>/dev/null || echo 0)))
        sleep 0.5
    done
    local t1 cpu1; t1=$(date +%s%N); cpu1=$(cpu_ticks "$pid")
    awk -v g="$((gpu / ROUNDS))" -v c0="$cpu0" -v c1="$cpu1" -v t0="$t0" -v t1="$t1" \
        -v tck="$tck" -v r="$((rss / ROUNDS / 1024))" \
        'BEGIN{dt=(t1-t0)/1e9; printf "%3d%% %5.1f%% %5d MB", g, (dt>0 ? (c1-c0)/tck/dt*100 : 0), r}'
}

printf '%-34s %8s %7s %9s  %s\n' wallpaper gpu cpu rss memory

idle_gpu=0
for _ in $(seq 1 "$ROUNDS"); do [ -n "$GPU" ] && idle_gpu=$((idle_gpu + $(cat "$GPU"))); sleep 0.5; done
printf '%-34s %7d%% %7s %9s  %s\n' "(idle desktop)" "$((idle_gpu / ROUNDS))" "-" "-" "-"

ids=("$@")
if [ ${#ids[@]} -eq 0 ]; then
    scenes=0
    for d in "$WS"/*/; do
        id=$(basename "$d")
        case $(sed -n 's/.*"type"[[:space:]]*:[[:space:]]*"\([a-z]*\)".*/\1/p' "$d/project.json" 2>/dev/null | head -1) in
            video) ids+=("$id") ;;
            scene) [ $scenes -lt 3 ] && { ids+=("$id"); scenes=$((scenes + 1)); } ;;
        esac
    done
fi

for id in "${ids[@]}"; do
    [ -d "$WS/$id" ] || { echo "$id: not installed" >&2; continue; }
    log=$(mktemp)
    "$RENDER" --output "$OUTPUT" --bg "$WS/$id" --layer bottom --duration $((SECONDS_PER + 4)) >"$log" 2>&1 &
    pid=$!
    sleep 2
    stats=$(sample "$pid")
    wait "$pid" 2>/dev/null
    mem=$(grep -oE 'texture\+fbo memory [0-9.]+ MiB' "$log" | tail -1 | sed 's/texture+fbo memory //')
    [ -n "$mem" ] || mem=$(grep -oE '[0-9.]+ MiB' "$log" | tail -1)
    printf '%-34s %s  %s\n' "$id" "$stats" "${mem:--}"
    rm -f "$log"
done
