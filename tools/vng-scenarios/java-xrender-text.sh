# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
#
# Issue #137 (kaaduu): Java AWT/Swing text is invisible, and
# `-Dsun.java2d.xrender=false` cures it. Java2D's XRender pipeline paints
# every glyph through `XRSolidSrcPict` — a 1x1 pixmap picture with
# `repeat=Normal` — and the glyph path accepted only SolidFill and Gradient
# sources, so every text draw was discarded.
#
# No window manager on purpose. The probe is a single Swing toplevel that
# centres itself, and a WM would add its own text (titlebar, panel) to the
# scanout — which is cairo/pango text through CreateSolidFill, i.e. the path
# that always worked. Keeping the screen to just the probe means anything
# non-background in the dump came from Java.
#
# YS_XRENDER selects the pipeline, and is the whole experiment: `true` is the
# broken-before/fixed-after case, `false` is the in-run control that must look
# the same either way (it takes the core-X11 text path, which this change does
# not touch). Run it both ways and compare.
set -u
: "${YS_XRENDER:=true}"
# kaaduu's probe, compiled to a gitignored directory (gist
# 0d4ee5b733a623c64c37e948263fcc1a). The guest shares the host rootfs, so the
# host's build is right there.
: "${YS_PROBE_CP:=${YSERVER_REPO:?}/target/diag137}"
# YS_JAVA_AA picks the text antialiasing, which selects the GLYPH format and
# is a second, independent axis from the source-picture kind that #137 is
# about. `gasp`/`on` give grayscale AA and Java uploads A8 glyphs; `lcd` gives
# subpixel AA and Java uploads ARGB32 component-alpha glyphs, which
# `render_composite_glyphs` reduces to their alpha channel alone. There is no
# settings daemon in this guest, so the desktop default here is NOT what a
# MATE/GNOME session gives you -- forcing it is the only way to reach the LCD
# path from a bare guest.
: "${YS_JAVA_AA:=gasp}"

# The JVM sometimes dies with SIGILL in C1-compiled code in this guest
# (hs_err in `Module.ensureNativeAccess`) — a JIT/guest-CPU interaction,
# nothing to do with the X server. It has to be retried rather than tolerated:
# a run whose probe never mapped leaves a bare backdrop, and on the PRE-FIX
# side of an A/B that is easy to misread as "the bug". The window-mapped
# marker below is what separates the two.
attempt=0
while [ "$attempt" -lt 3 ]; do
    attempt=$((attempt + 1))
    if [ ! -r "$YS_PROBE_CP/XRenderTextProbe.class" ]; then
        echo "no XRenderTextProbe.class under $YS_PROBE_CP" > java-missing.txt
        break
    fi
    java -Dsun.java2d.xrender="$YS_XRENDER" \
        -Dawt.useSystemAAFontSettings="$YS_JAVA_AA" -cp "$YS_PROBE_CP" \
        XRenderTextProbe > "java-$attempt.log" 2>&1 &
    java_pid=$!
    # The JVM starting, laying out Swing and mapping its first frame is much
    # slower than an X client; 12s is what it takes here with room to spare.
    sleep 12
    # The probe's toplevel is 900x620 and it is the only client, so its
    # presence in the tree is the whole test of "the run is valid at all".
    if xwininfo -root -tree 2>/dev/null | grep -q 'Java XRender text probe'; then
        echo "attempt $attempt: mapped" > window-mapped.txt
        cp "java-$attempt.log" java.log 2>/dev/null || true
        break
    fi
    echo "attempt $attempt: probe never mapped" >> window-missing.txt
    kill -9 "$java_pid" 2>/dev/null || true
    rm -f hs_err_pid*.log
    sleep 2
done

# Where the probe actually landed, so a region measurement in the dump is
# checked against a number rather than eyeballed.
xwininfo -root -tree > tree.txt 2>&1 || true
xdotool getdisplaygeometry > geometry.txt 2>&1 || true
{
    echo "sun.java2d.xrender=$YS_XRENDER"
    echo "awt.useSystemAAFontSettings=$YS_JAVA_AA"
    java -version 2>&1
} > java-config.txt
# Also capture through the X protocol: it is the only route under
# `--server xorg` (there the root window IS the framebuffer), which is how
# the Xorg control for the glyph-format question gets its pixels.
import -window root screen.png 2>&1 | head -3 > import.log || true
sleep 1
