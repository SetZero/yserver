#!/usr/bin/env bash
# Host half of pixel-oracle.sh: dumps every output at each snapshot, then
# checks them with pixel-oracle-check.py; exits 1 on any wrong pixel.
#   tools/vng-scenarios/pixel-oracle-host.sh [name] [vng-shot args...]
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd -- "$here/../.." && pwd)
name=${1:-pixel-oracle}
shift || true
out=${VNG_OUT:-$repo/target/vng}/$name
rm -rf "$out"
"$repo/tools/vng-shot.sh" --outputs 2 --dump none --name "$name" --settle 0 --timeout 900 "$@" \
    --scenario "$here/pixel-oracle.sh" > /dev/null &
shot=$!
mon=$out/monitor.sock
n=0
while :; do
    for _ in $(seq 1 600); do
        [ -e "$out/READY-$n" ] || [ -e "$out/STEPS-DONE" ] && break
        kill -0 "$shot" 2>/dev/null || break
        sleep 0.5
    done
    [ -e "$out/READY-$n" ] || break
    outs=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["requested"]))' "$out/snap-$n.json")
    "$repo/tools/qemu-monitor.py" "$mon" "sendkey ctrl-alt-ret"
    for _ in $(seq 1 40); do
        [ "$(compgen -G "$out/yserver-scanout-*-out*" | wc -l || true)" -ge "$outs" ] && break
        sleep 0.25
    done
    sleep 0.5
    for f in "$out"/yserver-scanout-*-out*; do
        [ -e "$f" ] || continue
        o=${f##*-out}; o=${o%%-*}
        ext=${f##*.}
        mv "$f" "$out/scanout-$n-out$o.$ext"
    done
    touch "$out/DONE-$n"
    n=$((n + 1))
done
wait "$shot"
[ "$n" -gt 0 ] || { echo "pixel-oracle-host: no snapshot" >&2; exit 1; }
python3 "$here/pixel-oracle-check.py" "$out" | tee "$out/oracle-check.txt"
