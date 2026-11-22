#!/bin/sh
set -eu

scripts=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
base=/var/lib/webosbrew
state=$base/arc-restore
hook=$base/init.d/96-arc-restore
legacy_hook=$base/init.d/95-restore-sound-out
worker=$scripts/watch.sh

if [ "$(id -u)" != 0 ]; then
    echo 'ARC Restore needs the Homebrew Channel root service.' >&2
    exit 1
fi

running() {
    [ -r "$state/pid" ] || return 1
    read -r pid < "$state/pid"
    case "$pid" in ''|*[!0-9]*|0|1) return 1 ;; esac
    [ -r "/proc/$pid/cmdline" ] &&
        tr '\000' '\n' < "/proc/$pid/cmdline" | grep -Fxq "$worker"
}

show_status() {
    if running; then
        printf 'state=running\n'
        [ ! -r "$state/status" ] || cat "$state/status"
    elif [ -x "$hook" ]; then
        printf 'state=stopped\n'
    elif [ -d "$state" ]; then
        printf 'state=disabled\n'
    else
        printf 'state=uninstalled\n'
    fi
    if [ -x "$legacy_hook" ]; then
        printf 'legacy=active\n'
    fi
}

start_worker() {
    if running; then return; fi
    rm -f "$state/status"
    /bin/sh "$worker" "$state" "$hook" </dev/null >/dev/null 2>&1 9>&- &
    pid=$!
    printf '%s\n' "$pid" > "$state/pid"
    sleep 1
}

case "${1:-status}" in
    status) show_status ;;
    enable|boot|disable)
        if [ "$1" = disable ] && [ ! -d "$state" ]; then
            show_status
            exit 0
        fi
        umask 077
        mkdir -p "$state"
        exec 9>"$state/lock"
        flock -x 9
        case "$1" in
            enable)
                if [ -x "$legacy_hook" ] &&
                    grep -Fq 'sh /var/lib/webosbrew/restore-sound-out.sh' "$legacy_hook"; then
                    chmod a-x "$legacy_hook"
                fi
                pkill -f '^(/bin/)?sh /var/lib/webosbrew/restore-sound-out[.]sh$' 2>/dev/null || true
                mkdir -p "$base/init.d"
                ln -sfn "$scripts/boot.sh" "$hook"
                start_worker
                ;;
            boot)
                [ ! -x "$hook" ] || start_worker
                exit 0
                ;;
            disable)
                rm -f "$hook"
                if running; then kill "$pid" 2>/dev/null || true; fi
                rm -f "$state/pid" "$state/status"
                ;;
        esac
        show_status
        ;;
    *) exit 2 ;;
esac
