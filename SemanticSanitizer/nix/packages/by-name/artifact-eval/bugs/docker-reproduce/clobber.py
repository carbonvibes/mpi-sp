#!/usr/bin/env python3

from __future__ import annotations

import datetime as dt
import os
import pathlib
import time

STACKDUMP_DIR = pathlib.Path(os.environ.get("STACKDUMP_DIR", "/tmp"))
TARGET = pathlib.Path(os.environ.get("TARGET", "/tmp/semsan-docker-tmp/example"))
PAST_SLOP_SECONDS = 2
FUTURE_WINDOW_SECONDS = 100
TICK_SECONDS = 1


def go_rfc3339_no_colons(ts: dt.datetime) -> str:
    value = ts.isoformat(timespec="seconds")
    if value.endswith("+00:00"):
        value = value[:-6] + "Z"
    return value.replace(":", "")


def stackdump_name(ts: dt.datetime) -> str:
    return f"goroutine-stacks-{go_rfc3339_no_colons(ts)}.log"


def desired_links(now: dt.datetime) -> set[pathlib.Path]:
    return {
        STACKDUMP_DIR / stackdump_name(now + dt.timedelta(seconds=offset))
        for offset in range(-PAST_SLOP_SECONDS, FUTURE_WINDOW_SECONDS + 1)
    }


def remove_if_ours(path: pathlib.Path) -> None:
    try:
        if path.is_symlink() and pathlib.Path(os.readlink(path)) == TARGET:
            path.unlink()
    except FileNotFoundError:
        pass


def main() -> int:
    managed: set[pathlib.Path] = set()
    print(f"maintaining {FUTURE_WINDOW_SECONDS}s rolling window in {STACKDUMP_DIR}")
    print(f"target: {TARGET}")

    try:
        while True:
            wanted = desired_links(dt.datetime.now().astimezone())

            for old in managed - wanted:
                remove_if_ours(old)

            created = 0
            for link in wanted:
                if link.exists() or link.is_symlink():
                    remove_if_ours(link)
                    if link.exists() or link.is_symlink():
                        try:
                            link.unlink()
                        except FileNotFoundError:
                            pass
                        except PermissionError:
                            continue
                try:
                    os.symlink(TARGET, link)
                    created += 1
                except FileExistsError:
                    pass

            managed = wanted
            print(f"active={len(managed)} created={created}", flush=True)
            time.sleep(TICK_SECONDS)
    except KeyboardInterrupt:
        print("\ncleaning up")
        for link in managed:
            remove_if_ours(link)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
