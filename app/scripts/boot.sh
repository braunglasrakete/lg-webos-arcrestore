#!/bin/sh
boot_script=$(readlink -f "$0") || exit 1
exec /bin/sh "${boot_script%/*}/control.sh" boot
