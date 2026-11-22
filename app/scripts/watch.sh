#!/bin/sh

state=$1
hook=$2
audio=luna://com.webos.service.audio
settings=luna://com.webos.settingsservice
previous=
bluetooth_seen=0
attempts=0
detail=Waiting
live=
configured=
last_status=

query() {
    luna-send -w 2500 -n 1 "$1" "$2" 2>/dev/null
}

output_value() {
    printf '%s\n' "$1" |
        sed -n 's/.*"soundOutput"[[:space:]]*:[[:space:]]*"\([A-Za-z0-9_-][A-Za-z0-9_-]*\)".*/\1/p'
}

is_bluetooth() {
    case "$1" in
        bt_soundbar|tv_speaker_bluetooth|bluetooth_speaker*|*bluetooth*) return 0 ;;
        *) return 1 ;;
    esac
}

is_enabled() {
    [ -x "$hook" ] && [ -r "$0" ]
}

publish() {
    status=$(printf 'route=%s\nconfigured=%s\nprevious=%s\ndetail=%s' "$live" "$configured" "$previous" "$detail")
    [ "$status" != "$last_status" ] || return 0
    printf '%s\n' "$status" > "$state/status.$$" && mv -f "$state/status.$$" "$state/status"
    last_status=$status
}

trap 'rm -f "$state/status.$$"; exit 0' INT TERM

while is_enabled; do
    configured=$(output_value "$(query "$settings/getSystemSettings" '{"category":"sound","keys":["soundOutput"]}')")
    live=$(output_value "$(query "$audio/getSoundOutput" '{}')")
    output=${live:-$configured}

    if is_bluetooth "$configured" || is_bluetooth "$live"; then
        if [ "$bluetooth_seen" -eq 0 ]; then attempts=0; fi
        bluetooth_seen=1
        detail=Bluetooth
    elif [ "$output" = tv_speaker ] && [ "$bluetooth_seen" -eq 1 ] &&
        [ -n "$previous" ] && [ "$previous" != tv_speaker ] &&
        { [ -z "$configured" ] || [ "$configured" = tv_speaker ] || [ "$configured" = "$previous" ]; }; then
        if [ "$attempts" -lt 3 ]; then
            is_enabled || break
            attempts=$((attempts + 1))
            response=$(query "$settings/setSystemSettings"                 "{\"category\":\"sound\",\"settings\":{\"soundOutput\":\"$previous\"}}")
            if printf '%s\n' "$response" | grep -Eq '"returnValue"[[:space:]]*:[[:space:]]*true'; then
                detail=CheckingRestore
            else
                detail=Retrying
            fi
        else
            bluetooth_seen=0
            detail=RestoreFailed
        fi
    elif [ -n "$output" ]; then
        if [ "$bluetooth_seen" -eq 1 ]; then
            if [ "$attempts" -gt 0 ] && [ "$output" = "$previous" ]; then
                detail=Restored
            else
                detail=Waiting
            fi
        fi
        bluetooth_seen=0
        previous=$output
    fi
    publish
    sleep 2
done
