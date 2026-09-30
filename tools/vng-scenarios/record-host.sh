#!/usr/bin/env bash
# Host half of the issue #180 vng A/B (see record.sh): boots the guest on
# yserver or Xorg, waits for the in-guest recorder, then presses real keys on
# the guest's PS/2 keyboard through the QEMU monitor, one held past the
# auto-repeat delay.
#   tools/vng-scenarios/record-host.sh [name] [vng-shot args...]   # e.g. --server xorg
# Artifacts: target/vng/<name>/{record,physical}.log
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
name=${1:-record}
shift || true
out=${VNG_OUT:-$repo/target/vng}/$name
rm -rf "$out"
"$repo/tools/vng-shot.sh" --dump none --hold 20 --name "$name" "$@" \
    --scenario "$repo/tools/vng-scenarios/record.sh" &
shot=$!
for _ in $(seq 1 600); do
    [ -e "$out/LISTENING" ] && break
    kill -0 "$shot" 2>/dev/null || { wait "$shot"; exit 1; }
    sleep 0.5
done
[ -e "$out/LISTENING" ] || { echo "record-host: guest recorder never started" >&2; exit 1; }
mon=$out/monitor.sock
send() { "$repo/tools/qemu-monitor.py" "$mon" "$@"; sleep 1; }
send "sendkey a"
send "sendkey b 1500"
wait "$shot"
cat "$out/physical.log"
