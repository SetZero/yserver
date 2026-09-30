# Never finishes; the runner must time it out or be interrupted.
# shellcheck shell=sh
touch STARTED
sleep 100000
echo pass > RESULT
