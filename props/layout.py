"""Lays out props/props.json: the hub's stunt park and the Skyhop course over the Shimmering Flats.

    python3 props/layout.py props/props.json
"""

import json
import math
import sys

GROUND = -58.8
props, pads, gates = [], [], []


def ramp(x, y, z, o, length, width, height, colour):
    props.append({'map': 1, 'kind': 'ramp', 'x': round(x, 2), 'y': round(y, 2), 'z': round(z, 2),
                  'o': round(o, 4), 'length': length, 'width': width, 'height': height, 'colour': colour})


def box(x, y, z, o, length, width, height, colour):
    props.append({'map': 1, 'kind': 'box', 'x': round(x, 2), 'y': round(y, 2), 'z': round(z, 2),
                  'o': round(o, 4), 'length': length, 'width': width, 'height': height, 'colour': colour})


def pad(x, y, z, o):
    pads.append({'map': 1, 'x': round(x, 2), 'y': round(y, 2), 'z': round(z, 2), 'o': round(o, 4)})


ORANGE, GOLD, BLUE, GREEN, STONE = [0.95, 0.45, 0.1], [0.95, 0.75, 0.2], [0.2, 0.45, 0.95], [0.2, 0.85, 0.35], [0.6, 0.58, 0.55]

# ── The stunt park, y -4100: kickers, a table-top, a big launch ──
for i, x in enumerate([-6165, -6140, -6115]):
    ramp(x, -4100, GROUND, 0, 10, 8, 2.5 + i * 0.5, ORANGE)
pad(-6185, -4100, GROUND, 0)
ramp(-6160, -4130, GROUND, 0, 12, 10, 4, ORANGE)
box(-6146, -4130, GROUND, 0, 16, 10, 4, STONE)
ramp(-6132, -4130, GROUND, math.pi, 12, 10, 4, ORANGE)
pad(-6180, -4130, GROUND, 0)
ramp(-6150, -4070, GROUND, 0, 18, 10, 6, ORANGE)
pad(-6172, -4070, GROUND, 0)

# ── Skyhop: a launch ramp up onto floating platforms, gaps the hop must clear at speed ──
start = (-6178.0, -4030.0)
o = 0.0
fwd = lambda o: (math.cos(o), math.sin(o))
x, y = start
gates.append({'x': x, 'y': y, 'z': GROUND, 'r': 8})
# The ramp up: 32 long, 7 up (13 degrees, a climb and not a launcher).
lx, ly = x + 12 + 16, y
ramp(lx, ly, GROUND, o, 32, 10, 7, ORANGE)
top = GROUND + 7
edge = (lx + 16, ly)  # the ramp's high edge
# (length, width, gap, rise, lateral shift, turn before it, pad on it)
PLATFORMS = [
    # The first half: gaps of 9 to 13 yards (a hop at full speed clears 15), two gentle turns.
    (30, 10, 0, 0.0, 0, 0, False),
    (14, 8, 9, 0.0, 2, 0, False),
    (12, 7, 10, 0.0, -3, 0, False),
    (14, 7, 11, 0.0, 3, 0, False),
    (16, 9, 9, 0.0, 0, 0.25, True),
    (10, 6, 11, 0.0, 0, 0, False),
    (12, 6, 12, 0.0, -2, 0, False),
    (16, 9, 10, 0.0, 0, -0.35, True),
    (10, 6, 12, 0.0, 0, 0, False),
    (12, 6, 13, 0.0, 0, 0, False),
    # Halfway: a rest, its pad's boost for the first long gap.
    (22, 12, 11, 0.0, 0, 0, True),
    # The second half: narrower, the gaps at a hop's full reach, the line weaving.
    (12, 8, 12.5, 0.0, 0, 0, False),
    (10, 6, 12, 0.0, 1.5, 0, False),
    (10, 6, 12, 0.0, -1.5, 0, False),
    (16, 8, 11, 0.0, 0, 0.2, True),
    (10, 6, 12.5, 0.0, 0, 0, False),
    (10, 6, 12, 0.0, 1.5, 0, False),
    (16, 8, 12, 0.0, 0, -0.2, True),
    (8, 6, 12.5, 0.0, -1.5, 0, False),
    (20, 16, 12, 0.0, 0, 0, False),
]
cx, cy = edge
lowest = top
for n, (length, width, gap, rise, lat, turn, has_pad) in enumerate(PLATFORMS):
    o += turn
    fx, fy = fwd(o)
    rx, ry = -fy, fx  # to the left
    cx, cy = cx + fx * gap, cy + fy * gap  # the platform's near edge
    top += rise
    mx, my = cx + fx * length / 2 + rx * lat, cy + fy * length / 2 + ry * lat
    goal = n == len(PLATFORMS) - 1
    box(mx, my, top - 1.0, o, length, width, 1.0, GREEN if goal else (GOLD if n % 2 else BLUE))
    if has_pad:
        # At the platform's start: its boost burns on to the edge, and the hop is off the pad.
        pad(mx - fx * (length / 2 - 4), my - fy * (length / 2 - 4), top, o)
    gates.append({'x': round(mx, 2), 'y': round(my, 2), 'z': round(top, 2), 'r': round(max(length, width) / 2 + 2, 1)})
    cx, cy = cx + fx * length + rx * lat, cy + fy * length + ry * lat

out = {
    '_comment': 'World of Kartcraft props (kart/props.rs): shapes the kart drives on that the world has not, '
                'drawn by every client and in the kart collision; turbo pads; and the Skyhop challenge. '
                'WoW coordinates; a prop stands on (x, y, z), its length along o; a ramp rises along it.',
    'props': props,
    'pads': pads,
    'challenges': [{
        'key': 'skyhop',
        'name': 'Skyhop',
        'map': 1,
        'gates': gates,
        'floor': round(lowest - 3.0, 2),
    }],
}
json.dump(out, open(sys.argv[1], 'w', encoding='utf-8', newline='\n'), indent=1)
print(len(props), 'props', len(pads), 'pads', len(gates), 'gates; goal', gates[-1])
