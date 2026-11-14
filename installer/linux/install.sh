#!/bin/sh
# Installs Paneltarium for the current user (no root needed):
#
#   ./install.sh               programs to ~/.local/bin, menu entries, icon
#   ./install.sh --uninstall   removes them again (keeps your profiles in
#                              ~/.config/paneltarium)
#
# PREFIX=/usr/local sudo -E ./install.sh installs for every user instead.
#
# Needs at run time: GTK 3 and libayatana-appindicator3 (tray icon), and one
# of: GNOME, KDE Plasma (kscreen-doctor), a wlroots compositor (wlr-randr)
# or an X11 session (xrandr). On GNOME the tray icon also needs the
# "AppIndicator and KStatusNotifierItem Support" extension.
set -eu

APP_ID=io.github.jordan_dev_stuff.Paneltarium
PREFIX=${PREFIX:-"$HOME/.local"}
BINDIR="$PREFIX/bin"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor/256x256/apps"
HERE=$(cd "$(dirname "$0")" && pwd)
PROGRAMS="paneltarium paneltarium-app paneltarium-manager"

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$APPS" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" || true
}

if [ "${1:-}" = "--uninstall" ]; then
    pkill -x paneltarium-app 2>/dev/null || true
    for p in $PROGRAMS; do rm -f "$BINDIR/$p"; done
    rm -f "$APPS/$APP_ID.desktop" "$APPS/$APP_ID.Manager.desktop" "$ICONS/$APP_ID.png"
    # The "start when I sign in" entry, if it was switched on.
    rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/paneltarium.desktop"
    refresh
    echo "Paneltarium removed. Your profiles are still in ${XDG_CONFIG_HOME:-$HOME/.config}/paneltarium."
    exit 0
fi

# Upgrading: stop the running copy so its file can be replaced.
running=0
if pgrep -x paneltarium-app >/dev/null 2>&1; then
    running=1
    pkill -x paneltarium-app || true
    sleep 1
fi

mkdir -p "$BINDIR" "$APPS" "$ICONS"
for p in $PROGRAMS; do
    install -m 755 "$HERE/bin/$p" "$BINDIR/$p"
done
for f in "$APP_ID.desktop" "$APP_ID.Manager.desktop"; do
    sed "s|@BINDIR@|$BINDIR|g" "$HERE/share/applications/$f" >"$APPS/$f"
    chmod 644 "$APPS/$f"
done
install -m 644 "$HERE/share/icons/$APP_ID.png" "$ICONS/$APP_ID.png"
refresh

echo "Paneltarium installed to $BINDIR."
case ":$PATH:" in
    *":$BINDIR:"*) ;;
    *) echo "Note: $BINDIR isn't on your PATH; the menu entries work anyway." ;;
esac
if [ "$running" = 1 ]; then
    nohup "$BINDIR/paneltarium-app" >/dev/null 2>&1 &
    echo "Restarted the running Paneltarium."
else
    echo "Start it from your app menu (\"Paneltarium\") or run: paneltarium-app"
fi
