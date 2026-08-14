# The resource watch units

`ygg-panic.timer` + `ygg-panic.service` run
`.agents/skills/yggterm-agent-fleet/ygg-panic.py tick` hourly, on the machine
that holds the seat. They are checked in so a successor can install them in one
line rather than rediscover them:

```sh
cp .agents/systemd/ygg-panic.* ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now ygg-panic.timer
systemctl --user list-timers ygg-panic.timer   # ⛔ CONFIRM `NEXT` IS NOT EMPTY
```

⛔ **Read the timer back, do not trust `enable`.** The first version used
`OnUnitActiveSec=` and systemd reported it `enabled` and `active (running)` with
`Trigger: n/a` — no next elapse at all. A watchdog in that state reports healthy
and never fires, which is worse than no watchdog. `OnCalendar=` always yields a
next run, and `NextElapseUSecRealtime` is the field that proves it.

⚠ It fires at **:17**, deliberately off the hour, so it does not pile onto every
other job that picked `:00`.
