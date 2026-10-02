# Sourced as guest root by vng-shot: kill the server (a sent SEGV does not stop yserver).
# shellcheck shell=sh disable=SC2154
( sleep 3; kill -KILL "$server" ) &
