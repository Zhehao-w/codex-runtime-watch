#!/bin/sh
# Codex Runtime Watch privileged wake helper.
# Fixed commands only: no Codex access, network, auth, eval, or user code.
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
PMSET=/usr/bin/pmset
DATE=/bin/date
FIXED_OWNER=dev.codexruntimewatch.wake-helper
DYNAMIC_OWNER=dev.codexruntimewatch.wake-helper.dynamic

[ "$#" -eq 2 ] || { echo "Usage: wake-helper {run|remove|dry-run} marker" >&2; exit 2; }
ACTION=$1
MARKER=$2
case "$ACTION" in run|remove|dry-run) ;; *) exit 2 ;; esac
case "$MARKER" in
  /Users/*/Library/Application\ Support/dev.codexruntimewatch.app/window-wake-enabled) ;;
  *) echo "Unexpected wake marker path" >&2; exit 2 ;;
esac
[ "$ACTION" = dry-run ] || [ "$(/usr/bin/id -u)" = 0 ] || exit 2
NEXT_FILE=$(/usr/bin/dirname "$MARKER")/window-next-reset

# OS output uses either two- or four-digit year depending on macOS release.
# Match exact owner tags; never touch macOS- or other app-owned events.
fixed_events() {
  "$PMSET" -g sched | /usr/bin/sed -nE \
    "s#.*wake at ([0-9]{2}/[0-9]{2}/[0-9]{2,4} [0-9]{2}:[0-9]{2}:[0-9]{2}) by 'dev[.]codexruntimewatch[.]wake-helper'.*#\1#p"
}
dynamic_events() {
  "$PMSET" -g sched | /usr/bin/sed -nE \
    "s#.*wake at ([0-9]{2}/[0-9]{2}/[0-9]{2,4} [0-9]{2}:[0-9]{2}:[0-9]{2}) by 'dev[.]codexruntimewatch[.]wake-helper[.]dynamic'.*#\1#p"
}
# pmset schedule cancel expects the same mm/dd/yy clock time as scheduling.
to_short_date() {
  case "$1" in
    ??/??/????\ *) "$DATE" -j -f "%m/%d/%Y %H:%M:%S" "$1" "+%m/%d/%y %H:%M:%S" ;;
    *) printf '%s\n' "$1" ;;
  esac
}
cancel_events() {
  owner=$1
  events=$2
  printf '%s\n' "$events" | while IFS= read -r stamp; do
    [ -n "$stamp" ] || continue
    short=$(to_short_date "$stamp")
    "$PMSET" schedule cancel wake "$short" "$owner"
  done
}
remove_owned() {
  old_fixed=$(fixed_events)
  old_dynamic=$(dynamic_events)
  cancel_events "$FIXED_OWNER" "$old_fixed"
  cancel_events "$DYNAMIC_OWNER" "$old_dynamic"
}

# Fail closed if listing scheduled power events is unavailable.
"$PMSET" -g sched >/dev/null
if [ "$ACTION" = remove ] || [ ! -f "$MARKER" ]; then
  [ "$ACTION" = dry-run ] || remove_owned
  exit 0
fi
if ! "$PMSET" -g batt | /usr/bin/grep -q "AC Power"; then
  [ "$ACTION" = dry-run ] || remove_owned
  exit 0
fi

now=$("$DATE" +%s)
current_fixed=$(fixed_events)
# Fixed daily anchor/fallback wakes. Preserve all other users' wake events.
for day in 0 1; do
  for hour in 06 11 16 21; do
    stamp=$("$DATE" -v+"$day"d -v"$hour"H -v58M -v00S "+%m/%d/%y %H:%M:%S")
    epoch=$("$DATE" -j -f "%m/%d/%y %H:%M:%S" "$stamp" +%s)
    [ "$epoch" -gt "$((now + 45))" ] || continue
    # The system may show the year as YYYY; normalize for comparisons.
    exists=0
    printf '%s\n' "$current_fixed" | while IFS= read -r item; do
      [ -n "$item" ] || continue
      [ "$(to_short_date "$item")" != "$stamp" ] || exit 22
    done || exists=1
    [ "$exists" -eq 0 ] || continue
    if [ "$ACTION" = dry-run ]; then
      printf 'would schedule fixed wake %s\n' "$stamp"
    else
      "$PMSET" schedule wake "$stamp" "$FIXED_OWNER"
    fi
  done
done

# A user-owned plain decimal epoch timestamp is *data*, not an instruction.
# Root only accepts a local 07:00-22:30 reset within the next six hours.
target=""
if [ -f "$NEXT_FILE" ] && [ ! -L "$NEXT_FILE" ]; then
  target=$(/usr/bin/head -c 32 "$NEXT_FILE" | /usr/bin/tr -d '\n')
fi
case "$target" in
  *[!0-9]*|"") target="" ;;
esac
if [ -n "$target" ] && [ "$(/bin/expr "$target" : '[0-9]*$')" -ne 10 ]; then target=""; fi
wake_stamp=""
wake_epoch=0
if [ -n "$target" ] && [ "$target" -gt "$((now + 45))" ] && [ "$target" -le "$((now + 21600))" ]; then
  hm=$("$DATE" -r "$target" +%H%M)
  if [ "$hm" -ge 700 ] && [ "$hm" -le 2230 ]; then
    wake_epoch=$((target - 90))
    if [ "$wake_epoch" -gt "$((now + 45))" ]; then
      wake_stamp=$("$DATE" -r "$wake_epoch" "+%m/%d/%y %H:%M:%S")
    fi
  fi
fi

# Replace *only our dynamic* event when Codex reports a changed reset.
old_dynamic=$(dynamic_events)
desired_exists=0
printf '%s\n' "$old_dynamic" | while IFS= read -r item; do
  [ -n "$item" ] || continue
  [ -n "$wake_stamp" ] && [ "$(to_short_date "$item")" = "$wake_stamp" ] && exit 22
done || desired_exists=1
if [ "$ACTION" != dry-run ]; then
  # If desired exists, leave it; otherwise cancel stale dynamic wakes.
  if [ "$desired_exists" -eq 0 ]; then
    cancel_events "$DYNAMIC_OWNER" "$old_dynamic"
  fi
  if [ "$desired_exists" -eq 0 ] && [ -n "$wake_stamp" ]; then
    "$PMSET" schedule wake "$wake_stamp" "$DYNAMIC_OWNER"
  fi
elif [ -n "$wake_stamp" ]; then
  printf 'would maintain dynamic wake %s (reset %s)\n' "$wake_stamp" "$target"
fi

# Bounded protection between scheduled wake and the actual reset.
# Root never launches Codex; a user-owned process holds its own assertion
# throughout any actual rate-limit RPC + model request.
fixed_time=$("$DATE" +%H%M)
hold=0
case "$fixed_time" in
  0658|0659|0700|0701|1158|1159|1200|1201|1658|1659|1700|1701|2158|2159|2200|2201)
    hold=1 ;;
esac
if [ -n "$target" ] && [ "$target" -ge "$now" ] && [ "$target" -le "$((now + 180))" ]; then
  hold=1
fi
if [ "$hold" -eq 1 ] && [ "$ACTION" != dry-run ]; then
  # Keep this launchd job alive while its assertion is active; a detached
  # background child could be terminated when launchd ends the job.
  /usr/bin/caffeinate -i -t 240 </dev/null >/dev/null 2>&1
fi
