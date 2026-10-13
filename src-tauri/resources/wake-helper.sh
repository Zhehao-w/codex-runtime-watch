#!/bin/sh
# Restricted wake-event maintainer for Codex Runtime Watch.
# Installed root:wheel under /Library/PrivilegedHelperTools by explicit admin approval.
# NEVER reads credentials, runs Codex, or executes user-supplied shell code.
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
PMSET=/usr/bin/pmset
DATE=/bin/date
OWNER=dev.codexruntimewatch.wake-helper
ACTION=${1:-}
MARKER=${2:-}

case "$ACTION" in
  run|remove|dry-run) ;;
  *) echo "Usage: wake-helper.sh {run|remove|dry-run} marker-file" >&2; exit 2 ;;
esac
case "$MARKER" in
  /Users/*/Library/Application\ Support/dev.codexruntimewatch.app/window-wake-enabled) ;;
  *) echo "Refusing an unexpected marker path" >&2; exit 2 ;;
esac
if [ "$ACTION" != dry-run ] && [ "$(/usr/bin/id -u)" != 0 ]; then
  echo "Wake helper must run as root" >&2
  exit 2
fi

# Match only wake events created with our unique owner name. Other users'
# schedules and repeating rules are NEVER modified.
owned_events() {
  "$PMSET" -g sched | /usr/bin/sed -nE \
    "s/.*wake at ([0-9]{2}\/[0-9]{2}\/[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}) by '$OWNER'.*/\1/p"
}
remove_owned() {
  owned_events | while IFS= read -r stamp; do
    [ -n "$stamp" ] || continue
    "$PMSET" schedule cancel wake "$stamp" "$OWNER" || true
  done
}
if [ "$ACTION" = remove ] || [ ! -f "$MARKER" ]; then
  if [ "$ACTION" != dry-run ]; then remove_owned; fi
  exit 0
fi

# Only schedule while plugged in. Previously armed wakes might still fire
# after unplugging during sleep; the unprivileged tick separately checks AC.
if ! "$PMSET" -g batt | /usr/bin/grep -q "AC Power"; then
  if [ "$ACTION" != dry-run ]; then remove_owned; fi
  exit 0
fi

# Fail closed if the system cannot report scheduled events. Never create
# duplicate events when the schedule list is unavailable.
"$PMSET" -g sched >/dev/null

now=$("$DATE" +%s)
for offset in 0 1; do
  for hour in 06 11 16 21; do
    stamp=$("$DATE" -v+"$offset"d -v"${hour}"H -v58M -v00S "+%m/%d/%y %H:%M:%S")
    epoch=$("$DATE" -j -f "%m/%d/%y %H:%M:%S" "$stamp" +%s 2>/dev/null || echo 0)
    [ "$epoch" -gt "$((now + 45))" ] || continue
    if owned_events | /usr/bin/grep -Fxq "$stamp"; then
      continue
    fi
    if [ "$ACTION" = dry-run ]; then
      printf 'would-schedule wake %s owner=%s\n' "$stamp" "$OWNER"
    else
      "$PMSET" schedule wake "$stamp" "$OWNER"
    fi
  done
done

# Prevent idle sleep for at most 3 minutes near each wake point; user-requested
# Sleep is still permitted, and the assertion stops automatically.
minute=$("$DATE" +%H%M)
case "$minute" in
  0658|0659|0700|0701|1158|1159|1200|1201|1658|1659|1700|1701|2158|2159|2200|2201)
    if [ "$ACTION" != dry-run ]; then
      /usr/bin/caffeinate -i -t 180 >/dev/null 2>&1 &
    fi
    ;;
esac
