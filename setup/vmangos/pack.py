#!/usr/bin/env python3
"""Rebuilds the new-file hunks of kartcraft.patch from setup/vmangos/src/: the Kartcraft
sources live there as plain files and the patch carries them. Run after editing them, or
courses/courses.json:

    python3 setup/vmangos/pack.py           # regenerate KartcraftCourses.h, then the patch
    python3 setup/vmangos/pack.py --check   # exit 1 if either is stale

src/KartcraftCourses.h is generated here from courses/courses.json (docs/courses.md),
src/KartcraftShowcase.h from showcase/showcase.json (the showcase runs' starts), and so is
sql/kartcraft.sql, the world-database rows of the Race Marshals: one spawn per distinct
"marshal" spot in courses.json (written there by `course-gen marshals`). From
deliveries/deliveries.json, src/KartcraftDeliveries.h and sql/kartcraft_deliveries.sql: the
courier routes, every town to every other on its continent, a repeatable quest each. From
roads/roads.json (the GPS roads, driven by the bot on the real collision), sql/kartcraft_caches.sql:
the Astral and Wumpa Caches, spots along the roads in pools of one live cache a square. A file in src/ the
patch does not carry yet is added as a new file beside Kartcraft.h. The other hunks (the hooks
into vmangos's own files) are kept as they are.
"""
import hashlib
import json
import math
import os
import re
import textwrap
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PATCH = os.path.join(HERE, 'kartcraft.patch')
COURSES_JSON = os.path.join(HERE, '..', '..', 'courses', 'courses.json')
COURSES_H = os.path.join(HERE, 'src', 'KartcraftCourses.h')
SHOWCASE_JSON = os.path.join(HERE, '..', '..', 'showcase', 'showcase.json')
SHOWCASE_H = os.path.join(HERE, 'src', 'KartcraftShowcase.h')
MARSHAL_SQL = os.path.join(HERE, 'sql', 'kartcraft.sql')
DELIVERIES_JSON = os.path.join(HERE, '..', '..', 'deliveries', 'deliveries.json')
PARS_JSON = os.path.join(HERE, '..', '..', 'deliveries', 'pars.json')
DELIVERIES_H = os.path.join(HERE, 'src', 'KartcraftDeliveries.h')
DELIVERIES_SQL = os.path.join(HERE, 'sql', 'kartcraft_deliveries.sql')
ROADS_JSON = os.path.join(HERE, '..', '..', 'roads', 'roads.json')
CACHES_SQL = os.path.join(HERE, 'sql', 'kartcraft_caches.sql')
# The caches (Kartcraft.cpp "Caches"): the Astral Cache, rare, Ahn'Qiraj's Scarab Coffer (display
# 6502, seen nowhere else in the open world), and the Wumpa Cache, common, the Titan chest of
# Gordok Tribute (1387); neither the plain chest.
CACHE_ASTRAL, CACHE_WUMPA = 190200, 190201
CACHE_SCRIPT = 'go_kartcraft_cache'
# Spawn guids and pool entries reserved for them (the world's gameobject guids end near 4.0 M, its
# pool entries near 44 k).
CACHE_GUIDS = (4100000, 4109999)
CACHE_POOLS = (60000, 61999)
# Spots at least this far apart (yards) along the roads; one cache live at a time in each square
# this wide (yards); a spot this far (yards) from any course's start or marshal stays clear of it.
CACHE_SPACING = 250.0
CACHE_SQUARE = 3000.0
CACHE_CLEAR = 80.0
# One spot in this many is an Astral Cache.
CACHE_ASTRAL_SHARE = 7
# How much further (yards) an Astral Cache is seen than the world's 250, its beam with it.
CACHE_ASTRAL_SEEN = 250
# A claimed cache comes back somewhere in its square between these (s).
CACHE_RESPAWN = (120, 300)
GAME_DIR = 'src/game/'
KEY = re.compile(r'^[a-z0-9_-]{1,32}$')
CRATE_KINDS = 'FQW'


def hunk(path, text):
    lines = text.split('\n')
    if lines and lines[-1] == '':
        lines.pop()
    data = text.encode()
    blob = hashlib.sha1(b'blob %d\0' % len(data) + data).hexdigest()[:7]
    head = (f'diff --git a/{path} b/{path}\nnew file mode 100644\nindex 0000000..{blob}\n'
            f'--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{len(lines)} @@\n')
    return head + ''.join(f'+{l}\n' for l in lines)


def fail(message):
    sys.exit(f'courses/courses.json: {message}')


def num(v, what):
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        fail(f'{what} is not a number')
    v = float(v)
    if v != v or abs(v) > 1e6:
        fail(f'{what} is out of range')
    return v


def cfloat(v):
    """A C++ float literal: `-8930.f`, `12.5f`."""
    if v == int(v):
        return f'{int(v)}.f'
    return f'{v!r}f'


def cstring(s, what):
    if not isinstance(s, str) or not s or not s.isascii() or any(c in s for c in '\t\n\r"\\'):
        fail(f'{what} must be a non-empty ASCII string without tabs, quotes or backslashes')
    return f'"{s}"'


def load_courses():
    with open(COURSES_JSON, encoding='utf-8') as f:
        return json.load(f)


def marshal_of(c, key):
    """A course's Race Marshal spot (x, y, z, o), which `course-gen marshals` writes."""
    m = c.get('marshal')
    if not isinstance(m, dict):
        fail(f'course {key}: no "marshal" (run: course-gen marshals)')
    return tuple(num(m.get(f), f'course {key}: marshal {f}') for f in 'xyzo')


def courses_header(data):
    """KartcraftCourses.h: the course table, from courses/courses.json."""
    courses = data.get('courses') if isinstance(data, dict) else None
    if not isinstance(courses, list) or not courses:
        fail('"courses" must be a non-empty list')
    if len(courses) > 1000:
        fail('at most 1000 courses')
    out = [
        '// Generated by setup/vmangos/pack.py from courses/courses.json (docs/courses.md): edit',
        '// that file and rerun pack.py. Included by Kartcraft.cpp inside namespace Kartcraft,',
        '// after Course and CrateSpot.',
        '',
        '    static std::vector<Course> const COURSES = {',
    ]
    keys = set()
    for n, c in enumerate(courses):
        if not isinstance(c, dict):
            fail(f'course {n} is not an object')
        key = c.get('key')
        if not isinstance(key, str) or not KEY.match(key) or key == 'any':
            fail(f'course {n}: key must be 1-32 of a-z 0-9 _ - and not "any"')
        if key in keys:
            fail(f'course {key}: key used twice')
        keys.add(key)
        name = cstring(c.get('name'), f'course {key}: name')
        # The server's zone is only the Race Finder's group: a series ("Grand Prix") in its stead.
        zone = c.get('series') or c.get('zone', '')
        zone_c = cstring(zone, f'course {key}: zone') if zone else '""'
        cmap = c.get('map')
        laps = c.get('laps')
        if not isinstance(cmap, int) or isinstance(cmap, bool) or not 0 <= cmap < 2**32:
            fail(f'course {key}: map must be a map id')
        if not isinstance(laps, int) or isinstance(laps, bool) or not 0 <= laps <= 255:
            fail(f'course {key}: laps must be 0 (a sprint) to 255')
        points = c.get('checkpoints')
        if not isinstance(points, list) or len(points) < 2:
            fail(f'course {key}: at least two checkpoints')
        xy, gates = [], []
        for i, p in enumerate(points):
            if not isinstance(p, dict):
                fail(f'course {key}: checkpoint {i} is not an object')
            x = num(p.get('x'), f'course {key}: checkpoint {i} x')
            y = num(p.get('y'), f'course {key}: checkpoint {i} y')
            if 'z' in p:
                num(p['z'], f'course {key}: checkpoint {i} z')
            gate = num(p.get('gate', 0), f'course {key}: checkpoint {i} gate')
            if gate < 0:
                fail(f'course {key}: checkpoint {i} gate is negative')
            xy.append(f'{{ {cfloat(x)}, {cfloat(y)} }}')
            gates.append(gate)
        crates = []
        for i, s in enumerate(c.get('crates', [])):
            if not isinstance(s, dict) or s.get('kind') not in list(CRATE_KINDS):
                fail(f'course {key}: crate {i} needs a kind of F, Q or W')
            x = num(s.get('x'), f'course {key}: crate {i} x')
            y = num(s.get('y'), f'course {key}: crate {i} y')
            z = cfloat(num(s['z'], f'course {key}: crate {i} z')) if 'z' in s else 'INVALID_HEIGHT_VALUE'
            crates.append(f"{{ '{s['kind']}', {cfloat(x)}, {cfloat(y)}, {z} }}")

        doors = c.get('doors', [])
        if not isinstance(doors, list) or not all(
                isinstance(d, int) and not isinstance(d, bool) and 0 < d < 2**32 for d in doors):
            fail(f'course {key}: doors must be a list of gameobject entries')

        def rows(items, per):
            return ',\n            '.join(', '.join(items[i:i + per]) for i in range(0, len(items), per))

        note = c.get('note')
        title = f'{c["name"]}' + (f' ({c.get("zone")})' if c.get("zone") else '') + (f': {note}' if note else '')
        out += textwrap.wrap(title, 100, initial_indent='        // ', subsequent_indent='        // ')
        out.append(f'        {{ "{key}", {name}, {zone_c}, {cmap}, {laps},')
        out.append(f'          {{ {rows(xy, 4)} }},')
        if any(gates):
            out.append(f'          {{ {rows([cfloat(g) for g in gates], 8)} }},')
        else:
            out.append('          {},')
        out.append(f'          {{ {rows(crates, 3)} }},' if crates else '          {},')
        mx, my, mz, _ = marshal_of(c, key)
        doors_c = '{ ' + ', '.join(f'{d}u' for d in doors) + ' }' if doors else '{}'
        # A dungeon course's start height (Course::z): the start's z, else its marshal's beside it;
        # and its scale (Course::scale), the karts' and racers' size on it.
        scale = num(c.get('scale', 1), f'course {key}: scale')
        if not 0.2 <= scale <= 3:
            fail(f'course {key}: scale must be 0.2 to 3')
        sz = 0
        if cmap > 1:
            sz = num(points[0]['z'], f'course {key}: checkpoint 0 z') if 'z' in points[0] else mz
        # A flight trial's course (Course::flight): flown, never hosted, queued for or a challenge.
        flight = c.get('flight', False)
        if not isinstance(flight, bool):
            fail(f'course {key}: flight must be true or false')
        tail = ''
        if flight:
            tail = f', {cfloat(sz)}, {cfloat(scale)}, true'
        elif cmap > 1 or scale != 1:
            tail = f', {cfloat(sz)}' + (f', {cfloat(scale)}' if scale != 1 else '')
        out.append(f'          {{ {cfloat(mx)}, {cfloat(my)} }}, {doors_c}{tail} }},')
    out.append('    };')
    return '\n'.join(out) + '\n'


# The Race Marshal's ids, in the world database's custom ranges (docs/courses.md).
MARSHAL_ENTRY = 190000
MARSHAL_TEXT = 190000          # its greeting; MARSHAL_TEXT + 1 answers "How do races work?",
                               # MARSHAL_TEXT + 2 heads the course records, MARSHAL_TEXT + 3 the
                               # time trial leaderboard, MARSHAL_TEXT + 4 hosting's ground types
MARSHAL_GUIDS = (400000, 400999)
MARSHAL_SCRIPT = 'npc_kartcraft_marshal'
# The hub's Quartermaster (Kartcraft.cpp "Hub"): at the Mirage Raceway, between its pit crews, in
# the Goblin Pit Boss's look (display 7191); its menu opens the garage, the shop, the tracks and
# the Race Finder.
HUB_ENTRY = 190011
# The Caverns of Time's sealing door, CavernDoor01 (stock gameobject 179588), taken out.
COT_DOOR = 179588
# The most racers a dungeon course's copy holds: a race's whole field.
RACE_COPY_PLAYERS = 16
HUB_GUID = 401000
HUB_TEXT = 190020
HUB_SCRIPT = 'npc_kartcraft_hub'
HUB_SPOT = (1, -6206.0, -3896.0, -58.67, 3.1)
HUB_GREETING = ("The Mirage Raceway: every racer's meeting place. Pick your kart and its look, spend "
                "your tokens, or find a race from here.")
# NO_XP | NO_LOOT | UNKILLABLE | IMMUNE_TO_PC | IMMUNE_TO_NPC | AMPHIBIOUS | IGNORE_COMBAT
# (CreatureDefines.h): nobody fights the marshal.
MARSHAL_STATIC_FLAGS = 0x2 | 0x4 | 0x8 | 0x20 | 0x40 | 0x80000 | 0x2000000
GREETING = ('Racers, start your engines! Every course that starts here is yours to host: pick one, '
            'gather your friends, and when the host says go, everyone lines up on the grid.')
HELP = ('Host a race on a course that starts here, or join one that is waiting. Anyone can join '
        'until the host starts it; then every racer is put on the grid behind the line for the '
        'countdown.$B$BPass every checkpoint in order. A circuit runs its laps back through the '
        'start; a sprint ends at its last checkpoint. Break crates along the way for Wumpa fruit '
        'and weapons.$B$BEvery finish pays experience and Karting skill, the more the better your '
        'place. Far from a start? The Race Finder queues you for any course and takes you there.'
        '$B$BA time trial is you alone against the clock: no crates, no weapons, and the ghost of '
        'your best run to chase. Its times go on their own leaderboard.$B$BThe host picks whether '
        'ground types count: with them on, grass, dirt, snow, mud and water slow every racer alike.')
RECORDS = ('The fastest times on the courses that start here, and the record holders. Beat one '
           'and your name goes up on the board.')
TRIALS = ('The fastest time trials on the courses that start here: one racer, no weapons, only the '
          'clock. Set a time and see where you stand.')
HOST = ('Ground types on or off? On, grass, dirt, snow and mud slow every kart off the road, and water '
        'too, as they did on the old tracks; off, the whole course drives like the road. Every racer '
        'in your race drives by your choice, whatever their own settings say.')


def sql_str(text):
    return "'" + text.replace('\\', '\\\\').replace("'", "''") + "'"


def sql_num(v):
    return str(int(v)) if v == int(v) else repr(v)


def marshal_sql(data):
    """sql/kartcraft.sql: the Race Marshal, its texts and one spawn per distinct marshal spot."""
    spots = []
    for c in data['courses']:
        spot = (c['map'],) + marshal_of(c, c['key'])
        if spot not in spots:
            spots.append(spot)
    lo, hi = MARSHAL_GUIDS
    if len(spots) > hi - lo + 1:
        fail(f'{len(spots)} marshals, more than the {hi - lo + 1} spawn guids reserved')
    t0, t1, t2, t3, t4 = (MARSHAL_TEXT + i for i in range(5))
    out = [
        '-- Generated by setup/vmangos/pack.py from courses/courses.json (docs/courses.md): edit that',
        '-- file and rerun pack.py. The Race Marshals of World of Kartcraft, for the world database',
        '-- (mangos). vmangos-deploy runs it from storage/database/custom-sql/ on every database start,',
        '-- so it replaces its own rows, and only those.',
        '',
        'START TRANSACTION;',
        '',
        f'DELETE FROM `creature` WHERE `guid` BETWEEN {lo} AND {hi} OR `id` = {MARSHAL_ENTRY};',
        f'DELETE FROM `creature_template` WHERE `entry` = {MARSHAL_ENTRY};',
        f'DELETE FROM `npc_text` WHERE `ID` IN ({t0}, {t1}, {t2}, {t3}, {t4});',
        f'DELETE FROM `broadcast_text` WHERE `entry` IN ({t0}, {t1}, {t2}, {t3}, {t4});',
        '',
        'INSERT INTO `broadcast_text` (`entry`, `male_text`, `female_text`) VALUES',
        f'({t0}, {sql_str(GREETING)}, {sql_str(GREETING)}),',
        f'({t1}, {sql_str(HELP)}, {sql_str(HELP)}),',
        f'({t2}, {sql_str(RECORDS)}, {sql_str(RECORDS)}),',
        f'({t3}, {sql_str(TRIALS)}, {sql_str(TRIALS)}),',
        f'({t4}, {sql_str(HOST)}, {sql_str(HOST)});',
        '',
        'INSERT INTO `npc_text` (`ID`, `BroadcastTextID0`, `Probability0`) VALUES',
        f'({t0}, {t0}, 1),',
        f'({t1}, {t1}, 1),',
        f'({t2}, {t2}, 1),',
        f'({t3}, {t3}, 1),',
        f'({t4}, {t4}, 1);',
        '',
        '-- Daisy\'s goblin with the starting flag (display 2591), friendly to all (faction 35), a',
        f'-- gossip NPC (npc_flags 1) whose menu is the core\'s script {MARSHAL_SCRIPT}.',
        'INSERT INTO `creature_template` (`entry`, `patch`, `name`, `subname`, `level_min`, `level_max`,',
        '  `faction`, `npc_flags`, `display_id1`, `display_scale1`, `display_probability1`,',
        '  `display_total_probability`, `type`, `unit_class`, `inhabit_type`, `civilian`,',
        '  `static_flags1`, `flags_extra`, `script_name`) VALUES',
        f"({MARSHAL_ENTRY}, 0, 'Race Marshal', 'World of Kartcraft', 60, 60, 35, 1, 2591, 1, 100, 100, 7, 1, 3, 1,",
        f"  {MARSHAL_STATIC_FLAGS}, 2, '{MARSHAL_SCRIPT}');",
        '',
        '-- One beside each distinct course start, from courses.json\'s "marshal" spots.',
        'INSERT INTO `creature` (`guid`, `id`, `map`, `position_x`, `position_y`, `position_z`,',
        '  `orientation`, `spawntimesecsmin`, `spawntimesecsmax`, `wander_distance`, `movement_type`) VALUES',
    ]
    rows = [f'({lo + i}, {MARSHAL_ENTRY}, {m}, {sql_num(x)}, {sql_num(y)}, {sql_num(z)}, {sql_num(o)}, 300, 300, 0, 0)'
            for i, (m, x, y, z, o) in enumerate(spots)]
    out.append(',\n'.join(rows) + ';')
    m, x, y, z, o = HUB_SPOT
    out += [
        '',
        "-- The hub's Quartermaster at the Mirage Raceway (Kartcraft.cpp \"Hub\"): a gossip NPC whose",
        f'-- menu is the core\'s script {HUB_SCRIPT}.',
        f'DELETE FROM `creature` WHERE `guid` = {HUB_GUID} OR `id` = {HUB_ENTRY};',
        f'DELETE FROM `creature_template` WHERE `entry` = {HUB_ENTRY};',
        f'DELETE FROM `npc_text` WHERE `ID` = {HUB_TEXT};',
        f'DELETE FROM `broadcast_text` WHERE `entry` = {HUB_TEXT};',
        'INSERT INTO `broadcast_text` (`entry`, `male_text`, `female_text`) VALUES',
        f'({HUB_TEXT}, {sql_str(HUB_GREETING)}, {sql_str(HUB_GREETING)});',
        'INSERT INTO `npc_text` (`ID`, `BroadcastTextID0`, `Probability0`) VALUES',
        f'({HUB_TEXT}, {HUB_TEXT}, 1);',
        'INSERT INTO `creature_template` (`entry`, `patch`, `name`, `subname`, `level_min`, `level_max`,',
        '  `faction`, `npc_flags`, `display_id1`, `display_scale1`, `display_probability1`,',
        '  `display_total_probability`, `type`, `unit_class`, `inhabit_type`, `civilian`,',
        '  `static_flags1`, `flags_extra`, `script_name`) VALUES',
        f"({HUB_ENTRY}, 0, 'Kartcraft Quartermaster', 'World of Kartcraft', 60, 60, 35, 1, 7191, 1, 100, 100, 7, 1, 3, 1,",
        f"  {MARSHAL_STATIC_FLAGS}, 2, '{HUB_SCRIPT}');",
        'INSERT INTO `creature` (`guid`, `id`, `map`, `position_x`, `position_y`, `position_z`,',
        '  `orientation`, `spawntimesecsmin`, `spawntimesecsmax`, `wander_distance`, `movement_type`) VALUES',
        f'({HUB_GUID}, {HUB_ENTRY}, {m}, {sql_num(x)}, {sql_num(y)}, {sql_num(z)}, {sql_num(o)}, 300, 300, 0, 0);',
        '',
        "-- The Caverns of Time opened for racing (World of Kartcraft's deviation): the stock spawn",
        '-- of CavernDoor01 (179588) that seals the cavern is not spawned; the client leaves out the',
        '-- canyon plug and the cavern door (benilla-adt OPENED_WMOS, benilla-assets',
        '-- OPENED_DOODADS).',
        f'DELETE FROM `gameobject` WHERE `id` = {COT_DOOR} AND `map` = 1;',
    ]
    # A dungeon course's race takes a whole field into one copy of the dungeon (Kartcraft.cpp
    # "Race copies"), more than a 5- or 10-player dungeon's cap.
    dungeons = sorted({c['map'] for c in data['courses'] if c.get('map', 0) > 1})
    if dungeons:
        out += [
            '',
            "-- The dungeons courses are raced in hold a race's whole field (World of Kartcraft's",
            '-- deviation: Kartcraft.cpp "Race copies").',
            f'UPDATE `map_template` SET `player_limit` = GREATEST(`player_limit`, {RACE_COPY_PLAYERS}) '
            f'WHERE `entry` IN ({", ".join(str(m) for m in dungeons)});',
        ]
    out += ['', 'COMMIT;']
    return '\n'.join(out) + '\n'


def showcase_header():
    """The showcase runs' table: each run's key, title, note, map and start (WoW x, y, z and the
    facing the kart's heading gives, radians)."""
    with open(SHOWCASE_JSON, encoding='utf-8') as f:
        data = json.load(f)
    out = [
        '// Generated by setup/vmangos/pack.py from showcase/showcase.json (`course-gen showcase`):',
        '// rerun pack.py after authoring. Included by Kartcraft.cpp inside namespace Kartcraft,',
        '// after ShowRun.',
        '',
        '    static std::vector<ShowRun> const SHOW_RUNS = {',
    ]
    for r in data.get('runs', []):
        name = r.get('name')
        if not isinstance(name, str) or not KEY.match(name):
            sys.exit(f'showcase/showcase.json: a run name {name!r} is not a key')
        if r.get('map') not in (0, 1):
            sys.exit(f'showcase/showcase.json: {name}: map must be 0 or 1')
        start = r.get('start')
        if not isinstance(start, list) or len(start) != 3:
            sys.exit(f'showcase/showcase.json: {name}: start is not [x, y, z]')
        x, y, z = (num(v, f'{name} start') for v in start)
        heading = r.get('heading')
        if not isinstance(heading, int):
            sys.exit(f'showcase/showcase.json: {name}: heading is not a whole number')
        # The kart's heading (4096ths) as WoW's facing: kartcraft::yaw_of, Bevy's yaw being WoW's.
        o = round((heading * 2 * math.pi / 4096 + math.pi) % (2 * math.pi), 4)
        out.append(f'        {{ {cstring(name, "a run name")}, {cstring(r.get("title"), name + " title")}, '
                   f'{cstring(r.get("note"), name + " note")},')
        out.append(f'          {r["map"]}, {cfloat(x)}, {cfloat(y)}, {cfloat(z)}, {cfloat(o)} }},')
    out.append('    };')
    return '\n'.join(out) + '\n'


# ── Courier deliveries ───────────────────────────────────────────────────────────────────────────
# deliveries/deliveries.json's towns make the routes: every town to every other on its continent
# that one faction can use (a faction's town to its own faction's and the neutral ones, which
# then only that faction takes), quest DELIVERY_QUEST + MAX_TOWNS x the from-town's place + the
# to-town's, so a town added at the end leaves every route's quest as it was. Par: the route's
# way by the GPS roads at the speed the CTR-physics bot drove them, from deliveries/pars.json
# (`course-gen gps` writes it); a pair it lacks, the straight line, half again for the roads, at
# 32 yards a second (the Balanced kart's top is 42), up to the next 30 s, at least 2 minutes.
DELIVERY_QUEST = 190200
DELIVERY_TOKEN = 190100
DELIVERY_PACKAGE = 190101
DELIVERY_PAY = 10
DELIVERY_BONUS = 5
MAX_TOWNS = 64
# A faction's races (RequiredRaces), and its team in KartcraftDeliveries.h (0 either).
FACTIONS = {'alliance': (77, 1), 'horde': (178, 2)}
# What a route's crate holds, as Courier Jobs lists it (the bags' item is the package either way):
# one of its faction's goods, or the neutral towns', by its quest.
CARGO = {
    None: ['Goblin Rocket Fuel', 'Gnomish Gears', 'Gadgetzan Water', 'Thorium Bars', 'Bilgewater Rum',
           'Venture Co. Blueprints', 'Steamwheedle Contraband', 'Mithril Casings', 'Swiftness Potions',
           'Nitro Canisters', 'Fireworks', 'Arcane Crystals', 'Seaforium Charges', 'Mechanical Squirrels'],
    'alliance': ['Stormwind Steel', 'Dwarven Stout', 'Dalaran Sharp Cheese', 'Ironforge Ingots',
                 'Moonberry Juice', 'Westfall Stew', 'Gnomish Spare Parts', 'Holy Candles', 'Thandol Rivets',
                 'Mountain Rams\' Feed', 'Silverleaf Bundles', 'Elwynn Lumber'],
    'horde': ['Orcish Axes', 'Mulgore Spice Bread', 'Thunder Ale', 'Tauren Totems', 'Kodo Hides',
              'Troll Voodoo Charms', 'Durotar Cactus Apples', 'Horde Banners', 'Razor Hill Fish',
              'Forsaken Tonics', 'Wolf Saddles', 'Bloodhoof Tallow'],
}


def load_deliveries():
    with open(DELIVERIES_JSON, encoding='utf-8') as f:
        data = json.load(f)
    towns = data.get('towns') if isinstance(data, dict) else None
    if not isinstance(towns, list) or len(towns) < 2 or len(towns) > MAX_TOWNS:
        sys.exit(f'deliveries/deliveries.json: "towns" must list 2 to {MAX_TOWNS} towns')
    keys = set()
    for i, t in enumerate(towns):
        what = f'deliveries/deliveries.json: town {i}'
        if not isinstance(t, dict) or not KEY.match(str(t.get('key', ''))) or t['key'] in keys:
            sys.exit(f'{what}: a unique key of a-z 0-9 _ -')
        keys.add(t['key'])
        for field in ('map', 'zone', 'npc'):
            if not isinstance(t.get(field), int) or isinstance(t.get(field), bool) or t[field] < 0:
                sys.exit(f'{what}: {field} must be a whole number')
        for field in ('x', 'y', 'z', 'o'):
            if isinstance(t.get(field), bool) or not isinstance(t.get(field), (int, float)):
                sys.exit(f'{what}: {field} must be a number')
        if t.get('faction', 'neutral') not in ('neutral',) + tuple(FACTIONS):
            sys.exit(f'{what}: faction must be alliance, horde or none')
        for field in ('name', 'npc_name'):
            v = t.get(field)
            if not isinstance(v, str) or not v or not v.isascii() or any(c in v for c in '\t\n\r"\\'):
                sys.exit(f'{what}: {field} must be ASCII without tabs, quotes or backslashes')
    return towns


def load_pars():
    try:
        with open(PARS_JSON, encoding='utf-8') as f:
            pars = json.load(f).get('pars', {})
    except FileNotFoundError:
        return {}
    if not all(isinstance(v, int) and not isinstance(v, bool) and 60 <= v <= 36000 for v in pars.values()):
        sys.exit('deliveries/pars.json: every par must be whole seconds, 60 to 36000')
    return pars


def delivery_routes(towns):
    pars = load_pars()
    routes = []
    for i, a in enumerate(towns):
        for j, b in enumerate(towns):
            if i == j or a['map'] != b['map']:
                continue
            fa, fb = a.get('faction'), b.get('faction')
            if fa and fb and fa != fb:
                continue
            faction = fa or fb
            distance = math.hypot(a['x'] - b['x'], a['y'] - b['y'])
            par = pars.get(f"{a['key']}>{b['key']}") or max(120, int(math.ceil(distance * 1.5 / 32 / 30)) * 30)
            races, team = FACTIONS.get(faction, (0, 0))
            quest = DELIVERY_QUEST + MAX_TOWNS * i + j
            goods = CARGO[faction]
            routes.append({'quest': quest, 'from': a, 'to': b, 'par': par, 'races': races, 'team': team,
                           'distance': int(round(distance)), 'cargo': goods[quest % len(goods)]})
    return routes


def clock(seconds):
    return f'{seconds // 60}:{seconds % 60:02d}'


def deliveries_header(routes, towns):
    """KartcraftDeliveries.h: the routes, for Kartcraft.cpp's offers, clock and bonus."""
    out = [
        '// Generated by setup/vmangos/pack.py from deliveries/deliveries.json: edit that file and',
        '// rerun pack.py. Included by Kartcraft.cpp inside namespace Kartcraft, after Delivery.',
        '',
        '    static Delivery const DELIVERIES[] = {',
    ]
    for r in routes:
        a, b = r['from'], r['to']
        out.append(f'        {{ {r["quest"]}, "{a["name"]}", "{b["name"]}", {a["npc"]}, {b["npc"]}, {r["par"]}, '
                   f'{r["team"]}, "{a["npc_name"]}", "{b["npc_name"]}", {a["map"]}, {cfloat(a["x"])}, '
                   f'{cfloat(a["y"])}, {cfloat(a["z"])}, {cfloat(a["o"])}, {cfloat(b["x"])}, {cfloat(b["y"])}, '
                   f'{r["distance"]}, "{r["cargo"]}" }},')
    out.append('    };')
    # The Marathon Delivery's stops: the towns no faction holds, where either faction's courier
    # may call, at their couriers' places.
    out += ['', '    static MarathonTown const MARATHON_TOWNS[] = {']
    for t in towns:
        if not t.get('faction'):
            out.append(f'        {{ "{t["name"]}", {t["map"]}, {cfloat(t["x"])}, {cfloat(t["y"])}, '
                       f'{cfloat(t["z"])}, {cfloat(t["o"])} }},')
    out.append('    };')
    return '\n'.join(out) + '\n'


def deliveries_sql(towns, routes):
    """sql/kartcraft_deliveries.sql: the token, the package, a quest per route and who gives and
    takes it."""
    last = DELIVERY_QUEST + MAX_TOWNS * MAX_TOWNS
    out = [
        '-- Generated by setup/vmangos/pack.py from deliveries/deliveries.json: edit that file and',
        '-- rerun pack.py. World of Kartcraft\'s courier deliveries, for the world database (mangos):',
        '-- the token they pay, the package they carry, and a repeatable quest per route, from a',
        '-- town\'s courier (creature_questrelation) to another\'s (creature_involvedrelation).',
        '-- Kartcraft.cpp offers some of them a day (KartcraftDeliveries.h) and pays the bonus within',
        '-- par. vmangos-deploy runs this from storage/database/custom-sql/ on every database start,',
        '-- so it replaces its own rows, and only those.',
        '',
        'START TRANSACTION;',
        '',
        f'DELETE FROM `item_template` WHERE `entry` IN ({DELIVERY_TOKEN}, {DELIVERY_PACKAGE});',
        f'DELETE FROM `quest_template` WHERE `entry` BETWEEN {DELIVERY_QUEST} AND {last};',
        f'DELETE FROM `creature_questrelation` WHERE `quest` BETWEEN {DELIVERY_QUEST} AND {last};',
        f'DELETE FROM `creature_involvedrelation` WHERE `quest` BETWEEN {DELIVERY_QUEST} AND {last};',
        '',
        '-- The token (the Darkmoon Faire ticket\'s look), Kartcraft\'s currency (a delivery pays it,',
        '-- the skin shop takes it), bound to who earns it, 250 a stack. The package (the sealed',
        '-- crate\'s look), a quest item, one at a time.',
        'INSERT INTO `item_template` (`entry`, `patch`, `class`, `subclass`, `name`, `description`,',
        '  `display_id`, `quality`, `flags`, `buy_price`, `sell_price`, `bonding`, `max_count`, `stackable`) VALUES',
        f"({DELIVERY_TOKEN}, 0, 15, 0, 'Kartcraft Token', 'Kartcraft''s currency, earned on the road. Spend it in the Skin Shop: /kartshop.', 31745, 2, 0, 0, 0, 1, 0, 250),",
        f"({DELIVERY_PACKAGE}, 0, 12, 0, 'Courier''s Package', 'By kart only: no flights, no hearthstones, no portals.', 8928, 1, 0, 0, 0, 4, 1, 1);",
        '',
        f'-- The routes: the package handed over on accepting and taken back on delivering, {DELIVERY_PAY} tokens',
        '-- paid. Repeatable (SpecialFlags 1): the server allows each once a day. A faction\'s routes take',
        '-- only its races (RequiredRaces: Alliance 77, Horde 178; 0 anyone).',
        'INSERT INTO `quest_template` (`entry`, `patch`, `Method`, `ZoneOrSort`, `MinLevel`, `QuestLevel`,',
        '  `RequiredRaces`, `SpecialFlags`, `SrcItemId`, `SrcItemCount`, `Title`, `Details`, `Objectives`,',
        '  `OfferRewardText`, `RequestItemsText`, `ReqItemId1`, `ReqItemCount1`, `RewItemId1`, `RewItemCount1`) VALUES',
    ]
    rows = []
    for r in routes:
        a, b = r['from'], r['to']
        details = (f"This crate has to reach {b['npc_name']} in {b['name']}, and it goes by kart: no flight "
                   f"paths, no hearthstones, no portals while you carry it.$B$BGet it there within "
                   f"{clock(r['par'])} and there's a bonus in it for you.")
        rows.append(
            f"({r['quest']}, 0, 2, {a['zone']}, 1, 60, {r['races']}, 1, {DELIVERY_PACKAGE}, 1, "
            f"{sql_str('Express Delivery: ' + b['name'])}, {sql_str(details)}, "
            f"{sql_str('Deliver the Courier' + chr(39) + 's Package to ' + b['npc_name'] + ' in ' + b['name'] + '.')}, "
            f"{sql_str('Right on time. Here' + chr(39) + 's your pay, courier.')}, "
            f"{sql_str('You have something for me?')}, {DELIVERY_PACKAGE}, 1, {DELIVERY_TOKEN}, {DELIVERY_PAY})")
    out.append(',\n'.join(rows) + ';')
    out += ['', 'INSERT INTO `creature_questrelation` (`id`, `quest`) VALUES']
    out.append(',\n'.join(f"({r['from']['npc']}, {r['quest']})" for r in routes) + ';')
    out += ['', 'INSERT INTO `creature_involvedrelation` (`id`, `quest`) VALUES']
    out.append(',\n'.join(f"({r['to']['npc']}, {r['quest']})" for r in routes) + ';')
    out += ['', 'COMMIT;']
    return '\n'.join(out) + '\n'


def caches_sql(data):
    """sql/kartcraft_caches.sql: the caches' templates, a spot every CACHE_SPACING along the GPS
    roads (each map's points, in a fixed pseudo-random order, the first of any cluster kept), and
    a pool per CACHE_SQUARE of them, one live at a time."""
    with open(ROADS_JSON, encoding='utf-8') as f:
        roads = json.load(f)
    starts = []
    for c in data['courses']:
        cps = c.get('checkpoints') or []
        if cps:
            starts.append((c['map'], cps[0]['x'], cps[0]['y']))
        if c.get('marshal'):
            starts.append((c['map'], c['marshal']['x'], c['marshal']['y']))
    spots = []
    for m in roads['maps']:
        p = m['points']
        pts = [(p[i], p[i + 1], p[i + 2]) for i in range(0, len(p), 3)]
        order = sorted(range(len(pts)), key=lambda i: hashlib.sha256(
            f"{m['map']}:{pts[i][0]:.1f}:{pts[i][1]:.1f}".encode()).hexdigest())
        cell = CACHE_SPACING
        grid = {}
        for i in order:
            x, y, z = pts[i]
            gx, gy = int(math.floor(x / cell)), int(math.floor(y / cell))
            near = any(math.hypot(x - qx, y - qy) < CACHE_SPACING
                       for dx in (-1, 0, 1) for dy in (-1, 0, 1)
                       for (qx, qy) in grid.get((gx + dx, gy + dy), []))
            if near or any(sm == m['map'] and math.hypot(x - sx, y - sy) < CACHE_CLEAR for sm, sx, sy in starts):
                continue
            grid.setdefault((gx, gy), []).append((x, y))
            spots.append((m['map'], x, y, z))
    lo, hi = CACHE_GUIDS
    if len(spots) > hi - lo + 1:
        fail(f'{len(spots)} cache spots, more than the {hi - lo + 1} guids reserved')
    squares = {}
    for s in spots:
        squares.setdefault((s[0], int(math.floor(s[1] / CACHE_SQUARE)), int(math.floor(s[2] / CACHE_SQUARE))), [])
    keys = sorted(squares)
    plo, phi = CACHE_POOLS
    if len(keys) > phi - plo + 1:
        fail(f'{len(keys)} cache squares, more than the {phi - plo + 1} pool entries reserved')
    pool_of = {k: plo + i for i, k in enumerate(keys)}
    out = [
        '-- Generated by setup/vmangos/pack.py from roads/roads.json: edit that file (course-gen gps) or',
        '-- pack.py, and rerun it. The Astral and Wumpa Caches of World of Kartcraft (Kartcraft.cpp',
        '-- "Caches"), for the world database (mangos): chests on spots along the drivable roads, one',
        '-- live at a time in each square, claimed by driving through (or opening) one. vmangos-deploy',
        '-- runs it from storage/database/custom-sql/ on every database start, so it replaces its own',
        '-- rows, and only those.',
        '',
        'START TRANSACTION;',
        '',
        f'DELETE FROM `pool_gameobject` WHERE `guid` BETWEEN {lo} AND {hi};',
        f'DELETE FROM `pool_template` WHERE `entry` BETWEEN {plo} AND {phi};',
        f'DELETE FROM `gameobject` WHERE `guid` BETWEEN {lo} AND {hi} OR `id` IN ({CACHE_ASTRAL}, {CACHE_WUMPA});',
        f'DELETE FROM `gameobject_template` WHERE `entry` IN ({CACHE_ASTRAL}, {CACHE_WUMPA});',
        '',
        '-- Chests (type 3) with no lock and no loot of their own, consumed when claimed (data3), the',
        f'-- claim being the core\'s script {CACHE_SCRIPT}.',
        'INSERT INTO `gameobject_template` (`entry`, `patch`, `type`, `displayId`, `name`, `size`,',
        '  `data0`, `data1`, `data3`, `script_name`) VALUES',
        f"({CACHE_ASTRAL}, 0, 3, 6502, 'Astral Cache', 0.8, 0, 0, 1, '{CACHE_SCRIPT}'),",
        f"({CACHE_WUMPA}, 0, 3, 1387, 'Wumpa Cache', 0.5, 0, 0, 1, '{CACHE_SCRIPT}');",
        '',
        f'-- A pool a {int(CACHE_SQUARE)}-yard square, one cache live in it at a time.',
        'INSERT INTO `pool_template` (`entry`, `max_limit`, `description`) VALUES',
        ',\n'.join(f"({pool_of[k]}, 1, 'Kartcraft caches, map {k[0]} square {k[1]},{k[2]}')" for k in keys) + ';',
        '',
        f'-- A spot every {int(CACHE_SPACING)} yards along the roads; one in {CACHE_ASTRAL_SHARE} an Astral Cache.',
        'INSERT INTO `gameobject` (`guid`, `id`, `map`, `position_x`, `position_y`, `position_z`,',
        '  `orientation`, `rotation0`, `rotation1`, `rotation2`, `rotation3`, `spawntimesecsmin`,',
        '  `spawntimesecsmax`, `animprogress`, `state`, `visibility_mod`) VALUES',
    ]
    rows, pools = [], []
    for i, (m, x, y, z) in enumerate(spots):
        guid = lo + i
        astral = int(hashlib.sha256(f'{m}:{x}:{y}'.encode()).hexdigest(), 16) % CACHE_ASTRAL_SHARE == 0
        entry = CACHE_ASTRAL if astral else CACHE_WUMPA
        rows.append(f'({guid}, {entry}, {m}, {sql_num(round(x, 1))}, {sql_num(round(y, 1))}, '
                    f'{sql_num(round(z, 1))}, 0, 0, 0, 0, 1, {CACHE_RESPAWN[0]}, {CACHE_RESPAWN[1]}, 100, 1, '
                    f'{CACHE_ASTRAL_SEEN if astral else 0})')
        square = (m, int(math.floor(x / CACHE_SQUARE)), int(math.floor(y / CACHE_SQUARE)))
        pools.append(f"({guid}, {pool_of[square]}, 0, 'Kartcraft cache')")
    out.append(',\n'.join(rows) + ';')
    out += ['', 'INSERT INTO `pool_gameobject` (`guid`, `pool_entry`, `chance`, `description`) VALUES',
            ',\n'.join(pools) + ';', '', 'COMMIT;']
    return '\n'.join(out) + '\n'


def refresh(path, text, name, check, stale):
    # A checkout may have turned the line ends to CRLF; the file is compared as made.
    old = open(path, newline='').read().replace('\r\n', '\n') if os.path.isfile(path) else None
    if text != old:
        stale.append(name)
        if not check:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, 'w', newline='\n').write(text)


def main():
    check = '--check' in sys.argv
    stale = []
    data = load_courses()
    header = courses_header(data)
    refresh(COURSES_H, header, 'setup/vmangos/src/KartcraftCourses.h', check, stale)
    refresh(MARSHAL_SQL, marshal_sql(data), 'setup/vmangos/sql/kartcraft.sql', check, stale)
    show = showcase_header()
    refresh(SHOWCASE_H, show, 'setup/vmangos/src/KartcraftShowcase.h', check, stale)
    towns = load_deliveries()
    routes = delivery_routes(towns)
    deliveries = deliveries_header(routes, towns)
    refresh(DELIVERIES_H, deliveries, 'setup/vmangos/src/KartcraftDeliveries.h', check, stale)
    refresh(DELIVERIES_SQL, deliveries_sql(towns, routes), 'setup/vmangos/sql/kartcraft_deliveries.sql',
            check, stale)
    refresh(CACHES_SQL, caches_sql(data), 'setup/vmangos/sql/kartcraft_caches.sql', check, stale)
    made = {COURSES_H: header, SHOWCASE_H: show, DELIVERIES_H: deliveries}

    patch = open(PATCH, newline='').read()
    sections = re.split(r'(?m)^(?=diff --git )', patch)
    out = []
    carried = set()
    for s in sections:
        m = re.match(r'diff --git a/(\S+) b/\S+', s)
        local = m and os.path.join(HERE, 'src', m.group(1).split('/')[-1])
        if m and '\nnew file mode' in s.split('@@')[0] and os.path.isfile(local):
            carried.add(os.path.basename(local))
            text = made.get(local) or open(local, newline='').read()
            out.append(hunk(m.group(1), text))
        else:
            out.append(s)
    # A source the patch does not carry yet goes in after Kartcraft.h's hunk.
    for name in sorted(os.listdir(os.path.join(HERE, 'src'))):
        if name in carried:
            continue
        local = os.path.join(HERE, 'src', name)
        text = made.get(local) or open(local, newline='').read()
        at = next(i for i, s in enumerate(out) if s.startswith(f'diff --git a/{GAME_DIR}Kartcraft.h '))
        out.insert(at + 1, hunk(GAME_DIR + name, text))
    packed = ''.join(out)
    if packed != patch:
        stale.append('setup/vmangos/kartcraft.patch')
        if not check:
            open(PATCH, 'w', newline='\n').write(packed)
    if check and stale:
        sys.exit('stale (run python3 setup/vmangos/pack.py): ' + ', '.join(stale))


if __name__ == '__main__':
    main()
