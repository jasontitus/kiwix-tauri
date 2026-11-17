#!/bin/bash
# usage: run.sh <noinit|init>  — one fresh probe process per run.
# Prints the probe result and the requests the test tracker received.
A=${ADB:-adb}; here=$(dirname "$0"); log=${TRACKER_LOG:?set TRACKER_LOG to the tracker log file}
$A shell am force-stop org.kiwix.probe; $A logcat -c; : > "$log"
$A shell am start -n org.kiwix.probe/.MainActivity --es mode "$1" >/dev/null
timeout 900 bash -c "until $A logcat -d -s KiwixProbe:I | grep -q DONE; do sleep 5; done" || echo "(probe did not finish)"
echo "== live"; $A logcat -d -s KiwixProbeLive:I | sed 's/^.*KiwixProbeLive: //' | grep -v '^-----'
echo "== result"; $A logcat -d -s KiwixProbe:I | sed 's/^.*KiwixProbe: //' | grep -v '^-----'
echo "== tracker log"; cat "$log"
