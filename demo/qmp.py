#!/usr/bin/env python3
"""Drives the pointer of the omavm VM through QEMU's QMP socket (the VM has a
usb-tablet, so positions are absolute). Coordinates are logical pixels of the
guest screen, 1920x1080 for 3840x2160 at scale 2 (override with QMP_W/QMP_H).

    demo/qmp.py move X Y
    demo/qmp.py click X Y
    demo/qmp.py drag X0 Y0 X1 Y1 [hold]    # hold: leave the button down
    demo/qmp.py release
"""

import json
import os
import socket
import sys
import time

W = int(os.environ.get("QMP_W", 1920))
H = int(os.environ.get("QMP_H", 1080))

sock = socket.socket(socket.AF_UNIX)
sock.connect(os.path.join(os.environ["XDG_RUNTIME_DIR"], "omavm-qmp.sock"))
f = sock.makefile("rw")
f.readline()


def cmd(c):
    f.write(json.dumps(c) + "\n")
    f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            return r


def ev(events):
    return cmd({"execute": "input-send-event", "arguments": {"events": events}})


def move(x, y):
    ev([{"type": "abs", "data": {"axis": "x", "value": int(float(x) * 32767 / (W - 1))}},
        {"type": "abs", "data": {"axis": "y", "value": int(float(y) * 32767 / (H - 1))}}])


def btn(down):
    ev([{"type": "btn", "data": {"down": down, "button": "left"}}])


cmd({"execute": "qmp_capabilities"})
action, args = sys.argv[1], sys.argv[2:]
if action == "move":
    move(*args[:2])
elif action == "click":
    move(*args[:2])
    time.sleep(0.25)
    btn(True)
    time.sleep(0.08)
    btn(False)
elif action == "drag":
    x0, y0, x1, y1 = (float(a) for a in args[:4])
    move(x0, y0)
    time.sleep(0.4)
    btn(True)
    time.sleep(0.3)
    for i in range(1, 51):
        t = i / 50
        # ease in and out, so it looks like a hand
        e = t * t * (3 - 2 * t)
        move(x0 + (x1 - x0) * e, y0 + (y1 - y0) * e)
        time.sleep(0.025)
    time.sleep(0.6)
    if "hold" not in args[4:]:
        btn(False)
elif action == "release":
    btn(False)
