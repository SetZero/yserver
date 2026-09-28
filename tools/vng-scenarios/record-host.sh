#!/usr/bin/env bash
# Host half of the issue #180 vng A/B (see record.sh): boots the guest on
# yserver or Xorg, waits for the in-guest recorder, then presses real keys on
# the guest's PS/2 keyboard through the QEMU monitor, one held past the
# auto-repeat delay.
#   tools/vng-scenarios/record-host.sh yserver|xorg
# Artifacts: target/vng/record-<server>/{record,physical}.log (Xorg: physical only)
set -euo pipefail
server=${1:?usage: record-host.sh yserver|xorg}
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
name=record-$server
out=$repo/target/vng/$name
rm -rf "$out"
env=()
[ "$server" = xorg ] && env=(--env RECORD_PHYSICAL_ONLY=1)
"$repo/tools/vng-shot.sh" --server "$server" --dump none --hold 20 --name "$name" \
    ${env+"${env[@]}"} --scenario "$repo/tools/vng-scenarios/record.sh" &
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
