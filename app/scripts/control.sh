#!/bin/sh
set -eu

scripts=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
base=/var/lib/webosbrew
state=$base/arc-restore
hook=$base/init.d/96-arc-restore
legacy_hook=$base/init.d/95-restore-sound-out
shell_worker=$scripts/watch.sh
case "$(uname -m)" in
    aarch64|arm64) native=$scripts/../bin/arc-restore-watcher-aarch64 ;;
    *) native=$scripts/../bin/arc-restore-watcher-armv7 ;;
esac
native_copy=$state/arc-restore-watcher

if [ "$(id -u)" != 0 ]; then
    echo 'ARC Restore needs the Homebrew Channel root service.' >&2
    exit 1
fi

running() {
    [ -r "$state/pid" ] || return 1
    read -r pid < "$state/pid"
    case "$pid" in ''|*[!0-9]*|0|1) return 1 ;; esac
    [ -r "/proc/$pid/cmdline" ] &&
        tr '\000' '\n' < "/proc/$pid/cmdline" |
        grep -Fxq -e "$shell_worker" -e "$native_copy" -e "$(app_native)"
}

# Where the packaged binary for this CPU is, by its real path.
app_native() {
    if [ -f "$native" ]; then
        printf '%s/%s\n' "$(CDPATH='' cd -- "$(dirname -- "$native")" && pwd -P)" "${native##*/}"
    else
        printf '%s\n' "$shell_worker"
    fi
}

# Prints a native watcher that runs on this TV. Packaging can drop the
# execute bit and some app mounts refuse chmod, so a copy in the state
# directory is the fallback.
native_binary() {
    [ -f "$native" ] || return 1
    path=$(app_native)
    [ -x "$path" ] || chmod 755 "$path" 2>/dev/null || true
    if [ -x "$path" ] && "$path" --version >/dev/null 2>&1; then
        printf '%s\n' "$path"
        return 0
    fi
    cmp -s "$path" "$native_copy" 2>/dev/null || cp -f "$path" "$native_copy" 2>/dev/null || return 1
    chmod 700 "$native_copy" 2>/dev/null || true
    "$native_copy" --version >/dev/null 2>&1 || return 1
    printf '%s\n' "$native_copy"
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
    if [ -f "$state/shell" ]; then
        bin=
    else
        bin=$(native_binary) || bin=
    fi
    if [ -n "$bin" ]; then
        "$bin" "$state" "$hook" </dev/null >/dev/null 2>&1 9>&- &
    else
        /bin/sh "$shell_worker" "$state" "$hook" </dev/null >/dev/null 2>&1 9>&- &
    fi
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
                rm -f "$state/pid" "$state/status" "$native_copy"
                ;;
        esac
        show_status
        ;;
    *) exit 2 ;;
esac
