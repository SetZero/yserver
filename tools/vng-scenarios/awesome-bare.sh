# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
#
# Deliberately runs awesome with NO client windows. That is the condition under
# which a missed root repaint stays visible: with clients open their paints and
# their coverage of the root repair or hide it continuously, which is why this
# was never seen on a working desktop. The stock config paints a wallpaper
# across every screen, so the root is the drawable that straddles both outputs.
#
# Then sweep the cursor: on a dual-head layout the cursor is the only thing
# generating damage, so whatever it repaints marks exactly what was stale.
set -u
# jos's REAL config by default — it is what reproduced, and the guest shares
# the host rootfs so it is right there. It differs from the stock example in
# ways that matter here: `beautiful.wallpaper` is a JPEG and
# `gears.wallpaper.maximized(wallpaper, s, true)` is called PER SCREEN, so a
# dual-head start paints the shared root-sized pixmap twice, once per screen,
# clearing the root each time. That is a very different damage sequence from
# one straddling paint. YS_RC overrides.
: "${YS_RC:=/home/jos/.config/awesome/rc.lua}"
[ -r "$YS_RC" ] || YS_RC=/etc/xdg/awesome/rc.lua
# Use the config's own wallpaper when it is reachable. A NON-UNIFORM root is
# mandatory, not cosmetic: a uniformly black root matches a black scanout
# however the ack logic behaves, and two runs were wasted on exactly that
# vacuous comparison before the image became visible. If the configured image
# is missing, substitute a deliberately noisy one rather than proceeding with a
# blank root.
cp "$YS_RC" rc.lua
configured=$(sed -n 's/^beautiful.wallpaper *= *"\(.*\)".*/\1/p' rc.lua | head -1)
if [ -n "$configured" ] && [ -r "$configured" ]; then
    wallpaper=$configured
else
    magick -size 36x25 xc: +noise Random -scale 2304x800 wallpaper.png 2> magick.log
    wallpaper=$PWD/wallpaper.png
    sed -i "s|^beautiful.wallpaper .*=.*|beautiful.wallpaper = \"$wallpaper\"|" rc.lua
fi
{ echo "rc.lua from: $YS_RC"; echo "wallpaper: $wallpaper"; } > rc-used.txt
awesome -c "$PWD/rc.lua" > awesome.log 2>&1 &
sleep 8

xwininfo -root -tree > tree.txt 2>&1 || true
xdotool getdisplaygeometry > geometry.txt 2>&1 || true

# Repaint the WHOLE root, once, well after both outputs have composed at least
# one frame. That is the straddling paint under suspicion: one drawable, one
# paint, two outputs that must each present it. Doing it here rather than at
# startup is the point — a paint that lands before the first compose is carried
# by the initial full redraw and proves nothing.
# awesome's own wallpaper does not paint in the guest (the default theme's
# `gears.wallpaper` silently does nothing here and the root stays black), and a
# uniform root makes the whole comparison vacuous — a black scanout matches a
# black root however the ack logic behaves. So paint one the way a wallpaper
# setter does: background pixmap over the whole root, plus a clear.
if [ "${YS_REPAINT_ROOT:-0}" = 1 ]; then
    src=$(dirname "$0")/root-wallpaper-client.c
    [ -r "$src" ] || src=${YSERVER_REPO:?}/tools/vng-scenarios/root-wallpaper-client.c
    cc -O1 -o root-wallpaper-client "$src" -lX11 > cc-root.log 2>&1 \
        && { ./root-wallpaper-client > repaint.log 2>&1 & sleep 4; } \
        || cat cc-root.log >&2
fi

# YS_TRIGGER re-runs the wallpaper paint WHILE both outputs are live and
# composing, which start-up cannot: at start-up the first full redraw carries
# it and proves nothing.
#   hup    - SIGHUP makes awesome reload its config, which re-runs
#            set_wallpaper() for every screen.
#   xrandr - a layout change, which is what `screen.connect_signal
#            ("property::geometry", set_wallpaper)` is actually wired to. jos's
#            root went 4240x1440 -> 5120x1440 between sessions, so a layout
#            change did happen at some point before the symptom appeared.
case "${YS_TRIGGER:-none}" in
    hup)
        pkill -HUP -x awesome && sleep 6
        ;;
    xrandr)
        xrandr > xrandr-before.txt 2>&1 || true
        xrandr --output Virtual-2 --off > xrandr.log 2>&1 || true
        sleep 3
        xrandr --output Virtual-2 --auto --right-of Virtual-1 >> xrandr.log 2>&1 || true
        sleep 5
        xrandr > xrandr-after.txt 2>&1 || true
        ;;
esac

# Diagonal sweep across the left output, then a second pass lower down, so the
# trail is unmistakable against a stale background.
for i in $(seq 0 40); do
    xdotool mousemove $((80 + i * 26)) $((120 + i * 14))
    sleep 0.03
done
for i in $(seq 0 40); do
    xdotool mousemove $((1100 - i * 24)) $((520 + i * 5))
    sleep 0.03
done
sleep 2
