#include "Kartcraft.h"
#include "Database/DBCStores.h"
#include "Chat.h"
#include "Player.h"
#include "SpellMgr.h"
#include "Timer.h"
#include "ObjectAccessor.h"
#include "WorldSession.h"
#include "World.h"

#include <cmath>
#include <string>
#include <vector>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <set>
#include <unordered_map>
#include "GridMap.h"

extern SkillRaceClassInfoMap SkillRaceClassInfoBySkill;

namespace Kartcraft
{
    // SpellIcon.dbc
    constexpr uint32 ICON_MECHASTRIDER = 1240;
    constexpr uint32 ICON_BOOTS        = 230;
    constexpr uint32 ICON_FLARE        = 136;
    constexpr uint32 ICON_SPRINT       = 516;
    constexpr uint32 ICON_MOONGLOW     = 46;

    // A skill tier of one 300 step, so the skill keeps its own max rather than following level.
    constexpr uint32 SKILL_TIER_300 = 181;
    constexpr uint32 SKILL_CATEGORY_SECONDARY = 9;

    // Lines closer together than this are not counted (ms).
    constexpr uint32 LINE_INTERVAL = 1000;
    // A reported line is clamped to this many points.
    constexpr float MAX_LINE_POINTS = 100000.f;

    struct Unlock { uint32 skill; uint32 spell; };
    constexpr Unlock UNLOCKS[] = {
        {   1, SPELL_SUMMON_KART },
        {  75, SPELL_WUMPA_JUICE },
        { 150, SPELL_TURBO_BOOST },
        { 225, SPELL_SUPER_ENGINE },
        { 300, SPELL_AKU_AKU },
    };

    static char s_skillName[] = "Karting";
    static char s_empty[] = "";
    static SkillLineEntry s_skillLine;
    static SkillRaceClassInfoEntry s_raceClass;

    static std::unordered_map<uint32, uint32> s_lastLine; // player guid low -> ms
    static std::unordered_map<uint32, uint32> s_lastState; // player guid low -> ms

    // Kart state is relayed to players this close (yards), at most this often per driver (ms).
    // As far as Kartcraft's servers show the world (Visibility.Distance.Continents = 250), so a
    // kart in sight is a kart that moves.
    constexpr float STATE_RANGE = 250.f;
    constexpr uint32 STATE_INTERVAL = 50;
    // A state line longer than this is not a kart's.
    constexpr size_t MAX_STATE_LENGTH = 120;

    void LoadSkillLine()
    {
        s_skillLine.id = SKILL_KARTING;
        s_skillLine.categoryId = SKILL_CATEGORY_SECONDARY;
        for (auto& name : s_skillLine.name)
            name = s_empty;
        s_skillLine.name[0] = s_skillName;
        s_skillLine.spellIcon = ICON_MECHASTRIDER;
        if (sSkillLineStore.LookupEntry(SKILL_KARTING) || !sSkillLineStore.InsertEntry(SKILL_KARTING, &s_skillLine))
        {
            sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "Kartcraft: skill line %u is taken or out of range; karting is off.", SKILL_KARTING);
            return;
        }

        s_raceClass.skillId = SKILL_KARTING;
        s_raceClass.raceMask = 0;
        s_raceClass.classMask = 0;
        s_raceClass.flags = 0;
        s_raceClass.reqLevel = 0;
        s_raceClass.skillTierId = SKILL_TIER_300;
        SkillRaceClassInfoBySkill.insert(SkillRaceClassInfoMap::value_type(SKILL_KARTING, &s_raceClass));
    }

    static SpellEntry* AddSpell(uint32 id, char const* name, uint32 icon, uint32 castTimeIndex, uint32 durationIndex, uint32 cooldown, bool aura)
    {
        SpellEntry* spell = sSpellMgr.AddCustomSpell(id);
        spell->Custom |= SPELL_CUSTOM_POSITIVE; // a buff we may cancel, not a debuff
        spell->SpellName[0] = name;
        spell->SpellIconID = icon;
        spell->activeIconID = icon;
        spell->CastingTimeIndex = castTimeIndex;
        spell->DurationIndex = durationIndex;
        spell->RecoveryTime = cooldown;
        spell->StartRecoveryCategory = 133; // the global cooldown
        spell->StartRecoveryTime = 1500;
        spell->rangeIndex = 1;              // self
        spell->baseLevel = 1;
        spell->spellLevel = 1;
        spell->InterruptFlags = castTimeIndex > 1 ? 0x0F : 0;
        spell->Effect[EFFECT_INDEX_0] = aura ? SPELL_EFFECT_APPLY_AURA : SPELL_EFFECT_DUMMY;
        spell->EffectApplyAuraName[EFFECT_INDEX_0] = aura ? SPELL_AURA_DUMMY : 0;
        spell->EffectImplicitTargetA[EFFECT_INDEX_0] = TARGET_UNIT_CASTER;
        spell->EffectDieSides[EFFECT_INDEX_0] = 1;
        spell->DmgMultiplier[EFFECT_INDEX_0] = 1.f;
        return spell;
    }

    // Stock mechanostrider mounts (red, green, blue, unpainted): the kart borrows one's creature, so
    // the server mounts us on it and every client sees a rider. Kartcraft clients draw the kart in
    // its place; a stock client shows the mechanostrider.
    constexpr uint32 MECHANOSTRIDER_SPELLS[] = { 10873, 17453, 10969, 17454 };

    static int32 MountCreature()
    {
        for (uint32 id : MECHANOSTRIDER_SPELLS)
            if (SpellEntry const* mount = sSpellMgr.GetSpellEntry(id))
                for (uint32 i = 0; i < MAX_EFFECT_INDEX; ++i)
                    if (mount->EffectApplyAuraName[i] == SPELL_AURA_MOUNTED && mount->EffectMiscValue[i])
                        return mount->EffectMiscValue[i];
        return 0;
    }

    void LoadSpells()
    {
        // SpellCastTimes.dbc: 1 instant, 16 1.5 s. SpellDuration.dbc: 21 until cancelled, 2 30 s, 18 20 s, 36 1 s.
        SpellEntry* kart = AddSpell(SPELL_SUMMON_KART, "Summon Kart", ICON_MECHASTRIDER, 16, 21, 0, true);
        if (int32 creature = MountCreature())
        {
            kart->EffectApplyAuraName[EFFECT_INDEX_0] = SPELL_AURA_MOUNTED;
            kart->EffectMiscValue[EFFECT_INDEX_0] = creature;
        }
        else
            sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "Kartcraft: no mechanostrider mount found; other players will not see karts.");
        // The abilities are used from the kart, which is a mount: casting must not dismount.
        for (SpellEntry* ability : {
                 AddSpell(SPELL_WUMPA_JUICE, "Wumpa Juice", ICON_BOOTS, 1, 2, 2 * MINUTE * IN_MILLISECONDS, true),
                 AddSpell(SPELL_TURBO_BOOST, "Turbo Boost", ICON_FLARE, 1, 36, 30 * IN_MILLISECONDS, true),
                 AddSpell(SPELL_SUPER_ENGINE, "Super Engine", ICON_SPRINT, 1, 2, 3 * MINUTE * IN_MILLISECONDS, true),
                 AddSpell(SPELL_AKU_AKU, "Aku Aku", ICON_MOONGLOW, 1, 18, 5 * MINUTE * IN_MILLISECONDS, true) })
            ability->Attributes |= SPELL_ATTR_ALLOW_WHILE_MOUNTED;
    }

    static void LearnUnlocked(Player* player)
    {
        uint32 skill = player->GetSkillValueBase(SKILL_KARTING);
        for (auto const& unlock : UNLOCKS)
            if (skill >= unlock.skill && !player->HasSpell(unlock.spell))
                player->LearnSpell(unlock.spell, false);
    }

    void OnLogin(Player* player)
    {
        if (!sSkillLineStore.LookupEntry(SKILL_KARTING))
            return;
        if (!player->HasSkill(SKILL_KARTING))
            player->SetSkill(SKILL_KARTING, 1, SKILL_MAX);
        LearnUnlocked(player);
    }

    // Points a line needs to raise the skill at `skill`.
    static float PointsNeeded(uint32 skill)
    {
        return 50.f + 5.f * skill;
    }

    static void LandLine(Player* player, float points)
    {
        if (!player->HasSkill(SKILL_KARTING) || !player->HasAura(SPELL_SUMMON_KART))
            return;

        uint32 now = WorldTimer::getMSTime();
        uint32& last = s_lastLine[player->GetGUIDLow()];
        if (last && WorldTimer::getMSTimeDiff(last, now) < LINE_INTERVAL)
            return;
        last = now;

        points = std::min(points, MAX_LINE_POINTS);
        uint32 skill = player->GetSkillValueBase(SKILL_KARTING);
        float ratio = points / PointsNeeded(skill);

        // A line worth the skill raises it by one, a bigger one by up to five.
        uint32 gain = ratio >= 8.f ? 5 : ratio >= 4.f ? 3 : ratio >= 2.f ? 2 : ratio >= 1.f ? 1 : 0;
        if (gain && skill < SKILL_MAX)
        {
            player->SetSkill(SKILL_KARTING, std::min(skill + gain, SKILL_MAX), SKILL_MAX);
            LearnUnlocked(player);
        }

        // Character XP: a same-level kill for a line worth the skill, up to four for a big one.
        uint32 kill = 45 + 5 * player->GetLevel();
        uint32 xp = uint32(kill * std::min(ratio, 4.f));
        if (xp)
            player->GiveXP(xp, nullptr);
    }

    // A driver's kart state (steer, drift, turbo, engine), relayed to the players around them as
    // the same addon line, so their clients can draw and hear this kart as it is driven.
    static void RelayState(Player* player, std::string const& message)
    {
        if (!player->HasAura(SPELL_SUMMON_KART) || message.size() > MAX_STATE_LENGTH)
            return;
        uint32 now = WorldTimer::getMSTime();
        uint32& last = s_lastState[player->GetGUIDLow()];
        if (last && WorldTimer::getMSTimeDiff(last, now) < STATE_INTERVAL)
            return;
        last = now;
        WorldPacket data;
        ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, message.c_str(), LANG_ADDON, CHAT_TAG_NONE,
            player->GetObjectGuid(), player->GetName());
        player->SendMessageToSetInRange(&data, STATE_RANGE, false);
    }

    // ── Races ──────────────────────────────────────────────────────────────────────────────────
    //
    // A driver at a course's start hosts a race ("R\tH\t<course>"); drivers near it are told
    // ("R\tO") and join ("R\tJ\t<id>"); the host starts it ("R\tS\t<id>") and every racer gets the
    // course's checkpoints ("R\tC") and a countdown ("R\tG"). Each checkpoint a client reports
    // ("R\tK\t<id>\t<index>") is checked against the racer's real position and the course's order;
    // progress ("R\tP") and finishes ("R\tF") go to every racer, and a finish pays experience and
    // Karting skill by place. The courses are courses/courses.json (KartcraftCourses.h); clients
    // learn a course from "R\tC".

    // A crate or fruit: 'F' fruit crate, 'Q' weapon crate, 'W' fruit; z at or below
    // INVALID_HEIGHT for the ground's.
    struct CrateSpot { char kind; float x, y, z; };

    struct Course
    {
        char const* key;
        char const* name;
        // The zone it is in, as the Race Finder groups courses; may be empty.
        char const* zone;
        uint32 map;
        // 0: a sprint from the first checkpoint to the last; else laps back through the first.
        uint8 laps;
        std::vector<std::pair<float, float>> points; // WoW x, y
        // Per checkpoint, how near (yards) a racer must pass it, 0 for CHECKPOINT_RADIUS; may be empty.
        std::vector<float> gates;
        // The crate layout; empty for the one built from the checkpoints (Layout).
        std::vector<CrateSpot> crates;
    };

    // The courses: courses/courses.json, made into this table by setup/vmangos/pack.py.
#include "KartcraftCourses.h"

    // How near a checkpoint (yards, across the ground) a racer must pass, how near the start a
    // driver must be to host or join, how long a countdown runs (ms), and how long a race may
    // last before it is dropped (ms).
    constexpr float CHECKPOINT_RADIUS = 45.f;
    constexpr float START_RADIUS = 60.f;
    constexpr uint32 COUNTDOWN = 4000;
    constexpr uint32 RACE_LIFETIME = 20 * MINUTE * IN_MILLISECONDS;
    constexpr size_t MAX_RACERS = 8;

    struct Racer
    {
        ObjectGuid guid;
        std::string name;
        uint8 lap = 0;
        uint8 checkpoint = 0;
        bool finished = false;
        uint8 place = 0; // finishing place, 1 = first
        // Weapons: the item held (ITEM_NONE for none) and its shots left, when the crate that gave
        // it was taken (ms), the Wumpa count, and until when a fired mask protects (ms, 0 none).
        uint8 held = 15;
        uint8 shots = 0;
        uint32 claimed = 0;
        uint8 wumpa = 0;
        uint32 maskUntil = 0;
    };

    // A weapon crate ('Q'), a fruit crate ('F') or a single Wumpa fruit ('W') on a race's course.
    struct Crate
    {
        char kind;
        float x, y, z;
        bool up = true;
        uint32 respawnAt = 0; // ms; 0 while up, or never (a single fruit)
    };

    // A relayed shot: the item, when it was relayed (ms), whether it has hit anyone, and whether
    // it was dropped behind (TNT, Nitro, beaker).
    struct Shot
    {
        uint8 item;
        uint32 at;
        bool hit = false;
        bool dropped = false;
    };

    struct Race
    {
        Course const* course = nullptr;
        ObjectGuid host;
        uint32 created = 0;
        uint32 start = 0; // 0 until started
        uint8 finishers = 0;
        std::vector<Racer> racers;
        // Weapons, live from the start: the crates, the shots relayed in the last SHOT_LIFETIME
        // keyed by shooter low guid << 16 | shot, and the race's MixRNG state.
        std::vector<Crate> crates;
        std::unordered_map<uint64, Shot> shots;
        uint16 rng = 0;
    };

    static std::unordered_map<uint32, Race> s_races;
    static uint32 s_nextRace = 1;

    // Party addon lines run on vmangos's single AsyncPacket thread, but a LANG_ADDON line sent as
    // say, emote or yell runs on its map's thread: every Kartcraft line takes this lock.
    static std::mutex s_mutex;

    static void SendLine(Player* to, std::string const& text)
    {
        WorldPacket data;
        ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, text.c_str(), LANG_ADDON, CHAT_TAG_NONE,
            to->GetObjectGuid(), to->GetName());
        to->GetSession()->SendPacket(&data);
    }

    static void SendRace(Race const& race, std::string const& text)
    {
        for (Racer const& r : race.racers)
            if (Player* p = ObjectAccessor::FindPlayer(r.guid))
                SendLine(p, text);
    }

    static float DistanceTo(Player const* player, std::pair<float, float> const& point)
    {
        float dx = player->GetPositionX() - point.first;
        float dy = player->GetPositionY() - point.second;
        return std::sqrt(dx * dx + dy * dy);
    }

    // How near checkpoint `index` a racer must pass: its gate, at least CHECKPOINT_RADIUS.
    static float GateRadius(Course const& c, uint32 index)
    {
        return index < c.gates.size() ? std::max(CHECKPOINT_RADIUS, c.gates[index]) : CHECKPOINT_RADIUS;
    }

    static Course const* FindCourse(std::string const& key)
    {
        for (Course const& c : COURSES)
            if (key == c.key)
                return &c;
        return nullptr;
    }

    static std::string Roster(Race const& race)
    {
        std::string names;
        for (Racer const& r : race.racers)
            names += (names.empty() ? "" : ",") + r.name;
        return names;
    }

    static void Reward(Player* player, uint32 place, uint32 racers)
    {
        // A win against a field pays most: experience as three same-level kills, Karting +3.
        uint32 worth = racers > place ? racers - place + 1 : 1;
        worth = std::min<uint32>(worth, 3);
        uint32 kill = 45 + 5 * player->GetLevel();
        player->GiveXP(kill * worth, nullptr);
        if (player->HasSkill(SKILL_KARTING))
        {
            uint32 skill = player->GetSkillValueBase(SKILL_KARTING);
            if (skill < SKILL_MAX)
            {
                player->SetSkill(SKILL_KARTING, std::min(skill + worth, SKILL_MAX), SKILL_MAX);
                LearnUnlocked(player);
            }
        }
    }

    // ── Weapons ────────────────────────────────────────────────────────────────────────────────
    //
    // docs/kartcraft-weapons-protocol.md. The server owns the crates, the roulette, what each
    // racer holds and their Wumpa; clients simulate shots and the victim reports a hit. Item ids
    // and rules are CTR's (docs/ctr-weapons-spec.md).

    enum Item : uint8
    {
        ITEM_TURBO = 0, ITEM_BOMB = 1, ITEM_MISSILE = 2, ITEM_TNT = 3, ITEM_BEAKER = 4, ITEM_SPRING = 5,
        ITEM_SHIELD = 6, ITEM_MASK = 7, ITEM_CLOCK = 8, ITEM_WARP = 9, ITEM_BOMB_3 = 10,
        ITEM_MISSILE_3 = 11, ITEM_NONE = 15,
    };

    // A crate comes back 30 + 16 frames after it breaks (RB_Crate.c:48-75, 125).
    constexpr uint32 CRATE_RESPAWN = 1533;
    // How near a crate (yards, across the ground) a racer must be to take it.
    constexpr float CRATE_RADIUS = 6.f;
    // Shots are accepted 20 frames into the roulette (the earliest Fire stop, VehPhysProc.c:833).
    constexpr uint32 ROLL_MIN = 600;
    // A shot can be reported as a hit this long after it was relayed (ms).
    constexpr uint32 SHOT_LIFETIME = 30 * IN_MILLISECONDS;
    // CTR's projectile pool, and the W lines one sender may send a second.
    constexpr size_t MAX_LIVE_SHOTS = 12;
    constexpr uint32 MAX_WEAPON_LINES = 20;
    // Wumpa: capped at 10, juiced at 10 (DRIVER_WUMPA_JUICED_COUNT).
    constexpr uint8 MAX_WUMPA = 10;
    // The mask protects 7680 ms, 11520 ms juiced (VehPickupItem.c:193-351).
    constexpr uint32 MASK_TIME = 7680;
    constexpr uint32 MASK_TIME_JUICED = 11520;
    // A whole addon line stays under this (docs/ctr-weapons-assets-and-net.md, B1).
    constexpr size_t MAX_LINE = 200;

    // Per segment between consecutive checkpoints: a fruit crate a quarter of the way, a row of
    // weapon crates across the path halfway (yards apart), and single fruit three quarters along.
    constexpr float CRATE_ROW_SPACING = 4.f;
    constexpr uint32 CRATE_ROW = 4;
    constexpr uint32 SEGMENT_FRUIT = 3;
    constexpr float FRUIT_SPACING = 4.f;

    // The roulette tables (zGlobal_DATA.c:8108-8174). Race4 samples only its first 19 entries.
    constexpr uint8 TABLE_RACE1[] = { 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 0, 6 };
    constexpr uint8 TABLE_RACE2[] = {
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 1, 10, 10, 10,
        0, 0, 0, 0, 0, 6, 6, 6, 2, 2, 2, 11, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 8 };
    constexpr uint8 TABLE_RACE3[] = { 4, 3, 1, 10, 10, 0, 0, 6, 2, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7, 8 };
    constexpr uint8 TABLE_RACE4[] = { 0, 6, 2, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8 };
    constexpr uint8 TABLE_BOSS[] = { 4, 3, 1, 10, 10, 0, 0, 6, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7, 8, 8 };

    enum ItemSet { SET_RACE1, SET_RACE2, SET_RACE3, SET_RACE4, SET_BOSS, SET_FALLBACK };

    // A course's crate layout, built once from its checkpoints and the ground.
    static std::unordered_map<Course const*, std::vector<CrateSpot>> s_layouts;

    struct LineRate { uint32 window = 0; uint32 count = 0; };
    static std::unordered_map<uint32, LineRate> s_weaponRate; // player guid low -> this second

    // MixRNG_Scramble: CTR's 16-bit LCG (MixRNG.c:3-10).
    static uint16 Scramble(Race& race)
    {
        race.rng = uint16((uint32(race.rng) * 0x6255 + 0x3619) & 0xFFFF);
        return race.rng;
    }

    static std::string Fixed(float value, int decimals)
    {
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%.*f", decimals, value);
        return buf;
    }

    static float GroundZ(TerrainInfo const* terrain, float x, float y, float fallback)
    {
        if (!terrain)
            return fallback;
        // The terrain first, then a vmap floor just above it (a bridge, a road over a model).
        float ground = terrain->GetHeightStatic(x, y, MAX_HEIGHT, false);
        if (ground <= INVALID_HEIGHT)
            return fallback;
        float floor = terrain->GetHeightStatic(x, y, ground + 2.f, true, 6.f);
        return floor > INVALID_HEIGHT ? floor : ground;
    }

    static std::vector<CrateSpot> const& Layout(Course const& course, TerrainInfo const* terrain, float fallbackZ)
    {
        auto cached = s_layouts.find(&course);
        if (cached != s_layouts.end())
            return cached->second;
        std::vector<CrateSpot> spots;
        bool grounded = true;
        auto place = [&](char kind, float x, float y)
        {
            float z = GroundZ(terrain, x, y, INVALID_HEIGHT_VALUE);
            if (z <= INVALID_HEIGHT)
            {
                grounded = false;
                z = fallbackZ;
            }
            spots.push_back({ kind, x, y, z });
        };
        // The course's own layout where it has one, else one built along its segments.
        for (CrateSpot const& spot : course.crates)
        {
            if (spot.z > INVALID_HEIGHT)
                spots.push_back(spot);
            else
                place(spot.kind, spot.x, spot.y);
        }
        size_t n = course.points.size();
        size_t segments = !course.crates.empty() ? 0 : course.laps ? n : n - 1;
        for (size_t i = 0; i < segments; ++i)
        {
            auto const& a = course.points[i];
            auto const& b = course.points[(i + 1) % n];
            float dx = b.first - a.first, dy = b.second - a.second;
            float length = std::sqrt(dx * dx + dy * dy);
            if (length < 1.f)
                continue;
            float ux = dx / length, uy = dy / length; // along the path
            float px = -uy, py = ux;                  // across it
            auto at = [&](float t, float across, float along)
            {
                return std::make_pair(a.first + dx * t + px * across + ux * along,
                    a.second + dy * t + py * across + uy * along);
            };
            auto fruitCrate = at(0.25f, 0.f, 0.f);
            place('F', fruitCrate.first, fruitCrate.second);
            for (uint32 k = 0; k < CRATE_ROW; ++k)
            {
                auto crate = at(0.5f, (k - (CRATE_ROW - 1) / 2.f) * CRATE_ROW_SPACING, 0.f);
                place('Q', crate.first, crate.second);
            }
            for (uint32 k = 0; k < SEGMENT_FRUIT; ++k)
            {
                auto fruit = at(0.75f, 0.f, (k - (SEGMENT_FRUIT - 1) / 2.f) * FRUIT_SPACING);
                place('W', fruit.first, fruit.second);
            }
        }
        if (!grounded)
        {
            // Some ground was missing: use the layout this once and look again next race.
            static std::vector<CrateSpot> s_once;
            s_once = std::move(spots);
            return s_once;
        }
        return s_layouts[&course] = std::move(spots);
    }

    static Racer* FindRacer(Race& race, uint32 guidLow)
    {
        for (Racer& r : race.racers)
            if (r.guid.GetCounter() == guidLow)
                return &r;
        return nullptr;
    }

    // Race order: finishers by place, then by checkpoints passed, then by distance to the next.
    static std::vector<Racer*> Standings(Race& race)
    {
        Course const& c = *race.course;
        uint32 n = uint32(c.points.size());
        struct Key { Racer* racer; uint32 progress; float remaining; };
        std::vector<Key> keys;
        for (Racer& r : race.racers)
        {
            float remaining = 1e9f;
            uint32 next = c.laps ? (r.checkpoint + 1) % n : std::min<uint32>(r.checkpoint + 1, n - 1);
            if (Player* p = ObjectAccessor::FindPlayer(r.guid))
                remaining = DistanceTo(p, c.points[next]);
            keys.push_back({ &r, uint32(r.lap) * n + r.checkpoint, remaining });
        }
        std::stable_sort(keys.begin(), keys.end(), [](Key const& a, Key const& b)
        {
            if (a.racer->finished != b.racer->finished)
                return a.racer->finished;
            if (a.racer->finished)
                return a.racer->place < b.racer->place;
            if (a.progress != b.progress)
                return a.progress > b.progress;
            return a.remaining < b.remaining;
        });
        std::vector<Racer*> order;
        for (Key const& k : keys)
            order.push_back(k.racer);
        return order;
    }

    static bool LiveShot(Shot const& shot, uint32 now)
    {
        return WorldTimer::getMSTimeDiff(shot.at, now) <= SHOT_LIFETIME;
    }

    static bool IsProjectile(uint8 item)
    {
        switch (item)
        {
            case ITEM_BOMB: case ITEM_MISSILE: case ITEM_TNT: case ITEM_BEAKER:
            case ITEM_WARP: case ITEM_BOMB_3: case ITEM_MISSILE_3:
                return true;
        }
        return false;
    }

    // A projectile still in CTR's pool: relayed recently and not ended by a single-target hit
    // (missile, warp orb) or by a hit on a dropped item.
    static bool InPool(Shot const& shot, uint32 now)
    {
        return IsProjectile(shot.item) && LiveShot(shot, now) && !shot.hit;
    }

    // VehPhysGeneral_SetHeldItem (VehPhysGeneral.c:1651-1962), race mode, no boss overrides.
    static uint8 Roll(Race& race, Racer& me, uint32 now)
    {
        std::vector<Racer*> order = Standings(race);
        uint32 rank = uint32(std::find(order.begin(), order.end(), &me) - order.begin());
        uint32 mode = uint32(race.racers.size());
        int set = SET_FALLBACK;
        switch (mode)
        {
            case 8: set = rank == 1 ? SET_RACE2 : int(rank / 2); break;
            case 6: set = rank == 0 ? SET_RACE1 : rank == 5 ? SET_RACE4 : int((rank - 1) / 2 + 1); break;
            case 5: set = rank >= 4 ? SET_RACE4 : int(rank); break;
            case 4: set = int(std::min<uint32>(rank, SET_RACE4)); break;
            case 3:
                if (rank == 0)
                    set = SET_RACE1;
                else if (rank == 1)
                    set = (Scramble(race) & 1) ? SET_RACE2 : SET_RACE3;
                else
                    set = SET_RACE4;
                break;
            case 2: set = rank == 0 ? SET_RACE1 : SET_BOSS; break;
        }
        // Race4 on the first lap is downgraded to Race3. A sprint is all one lap, so there the
        // catch-up items come once the first third of its checkpoints is behind (Kartcraft's own
        // rule: CTR has no sprints).
        bool firstLap = race.course->laps ? me.lap == 0
                                          : size_t(me.checkpoint) * 3 < race.course->points.size();
        if (set == SET_RACE4 && firstLap)
            set = SET_RACE3;

        uint32 rng = (Scramble(race) >> 3) % 200;
        uint8 item;
        switch (set)
        {
            case SET_RACE1: item = TABLE_RACE1[rng * 20 / 200]; break;
            case SET_RACE2: item = TABLE_RACE2[rng * 52 / 200]; break;
            case SET_RACE3: item = TABLE_RACE3[rng * 20 / 200]; break;
            case SET_RACE4: item = TABLE_RACE4[rng * 19 / 200]; break;
            case SET_BOSS: item = TABLE_BOSS[rng * 20 / 200]; break;
            default: item = uint8(Scramble(race) % 12); break;
        }

        if (item == ITEM_SPRING)
            item = ITEM_TURBO;
        // One warp orb at a time: another becomes Missile x3 while one is held or flying.
        if (item == ITEM_WARP)
        {
            bool held = false;
            for (Racer const& r : race.racers)
                held |= &r != &me && r.held == ITEM_WARP && r.shots;
            for (auto const& s : race.shots)
                held |= s.second.item == ITEM_WARP && InPool(s.second, now);
            if (held)
                item = ITEM_MISSILE_3;
        }
        // With 3 or more players at most 2 may hold Missile x3; a third gets Missile x1.
        if (item == ITEM_MISSILE_3 && race.racers.size() >= 3)
        {
            uint32 holders = 0;
            for (Racer const& r : race.racers)
                holders += &r != &me && r.held == ITEM_MISSILE_3 && r.shots;
            if (holders >= 2)
                item = ITEM_MISSILE;
        }
        return item;
    }

    static void SendLayout(Race const& race)
    {
        std::string const head = "KARTCRAFT\tW\tL\t";
        std::string line;
        size_t first = 0;
        auto flush = [&](size_t next)
        {
            if (!line.empty())
                SendRace(race, head + std::to_string(first) + "\t" + line);
            line.clear();
            first = next;
        };
        for (size_t i = 0; i < race.crates.size(); ++i)
        {
            Crate const& c = race.crates[i];
            std::string crate = std::string(1, c.kind) + "," + Fixed(c.x, 2) + "," + Fixed(c.y, 2) + "," + Fixed(c.z, 2);
            size_t length = head.size() + std::to_string(first).size() + 1 + line.size() + (line.empty() ? 0 : 1) + crate.size();
            if (!line.empty() && length >= MAX_LINE)
                flush(i);
            line += (line.empty() ? "" : ";") + crate;
        }
        flush(race.crates.size());
    }

    // At the start: the crates go up and every racer is told where they are.
    static void StartWeapons(Race& race, uint32 now)
    {
        race.crates.clear();
        race.shots.clear();
        race.rng = uint16(now ^ (now >> 16));
        // The course map's own terrain: a Race Finder race starts with no racer near the course.
        TerrainInfo const* terrain = sTerrainMgr.LoadTerrain(race.course->map);
        auto const& start = race.course->points[0];
        float startZ = GroundZ(terrain, start.first, start.second, 0.f);
        for (CrateSpot const& s : Layout(*race.course, terrain, startZ))
        {
            Crate c;
            c.kind = s.kind;
            c.x = s.x;
            c.y = s.y;
            c.z = s.z;
            race.crates.push_back(c);
        }
        for (Racer& r : race.racers)
        {
            r.held = ITEM_NONE;
            r.shots = 0;
            r.claimed = 0;
            r.wumpa = 0;
            r.maskUntil = 0;
        }
        SendLayout(race);
    }

    // A racer left: their shots go with them.
    static void DropShots(Race& race, uint32 guidLow)
    {
        for (auto s = race.shots.begin(); s != race.shots.end();)
            s = uint32(s->first >> 16) == guidLow ? race.shots.erase(s) : std::next(s);
    }

    // A racer leaves: their item, Wumpa, mask and shots go with them. If they hosted, the next
    // in the roster starts the race instead, as the clients already show (the first is the host).
    static void RemoveRacer(Race& race, std::vector<Racer>::iterator racer)
    {
        uint32 low = racer->guid.GetCounter();
        race.racers.erase(racer);
        DropShots(race, low);
        bool hostLeft = std::none_of(race.racers.begin(), race.racers.end(),
            [&race](Racer const& r) { return r.guid == race.host; });
        if (hostLeft && !race.racers.empty())
            race.host = race.racers.front().guid;
    }

    // Racers are out of the fight from the countdown to the finish: monsters and players can't
    // attack them, nor they anyone (Kartcraft's own rule: courses run through mob country). A
    // unit belongs to its map's thread, so the change goes through the world's messager, which
    // runs between map updates; the flags are checked there, so a racer who logs back in mid-race
    // is covered again.
    static std::set<ObjectGuid> s_shielded;
    static uint32 s_shieldedAt = 0;
    constexpr uint32 SHIELD_INTERVAL = 250;
    constexpr uint32 RACE_SHIELD_FLAGS = UNIT_FLAG_IMMUNE_TO_NPC | UNIT_FLAG_IMMUNE_TO_PLAYER;

    // Run after every Kartcraft line: a racer let go is freed at once; those racing are covered
    // again at most every SHIELD_INTERVAL.
    static void TickShields(uint32 now)
    {
        std::set<ObjectGuid> racing;
        for (auto const& entry : s_races)
            if (entry.second.start)
                for (Racer const& r : entry.second.racers)
                    if (!r.finished)
                        racing.insert(r.guid);
        std::vector<ObjectGuid> off;
        for (ObjectGuid const& guid : s_shielded)
            if (!racing.count(guid))
                off.push_back(guid);
        bool newcomer = std::any_of(racing.begin(), racing.end(),
            [](ObjectGuid const& guid) { return !s_shielded.count(guid); });
        s_shielded = racing;
        bool due = !racing.empty() && WorldTimer::getMSTimeDiff(s_shieldedAt, now) >= SHIELD_INTERVAL;
        if (off.empty() && !newcomer && !due)
            return;
        s_shieldedAt = now;
        sWorld.GetMessager().AddMessage([racing, off](World*)
        {
            for (ObjectGuid const& guid : racing)
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    if (!p->HasFlag(UNIT_FIELD_FLAGS, RACE_SHIELD_FLAGS))
                    {
                        p->SetFlag(UNIT_FIELD_FLAGS, RACE_SHIELD_FLAGS);
                        p->CombatStopWithPets(true);
                        p->GetHostileRefManager().deleteReferences();
                    }
            for (ObjectGuid const& guid : off)
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    p->RemoveFlag(UNIT_FIELD_FLAGS, RACE_SHIELD_FLAGS);
        });
    }

    // Crates due back come up; old shots are forgotten. Run on every Kartcraft line: a racer's
    // kart sends its state ten times a second, so a respawn is late by at most about 100 ms.
    static void TickWeapons(uint32 now)
    {
        for (auto& entry : s_races)
        {
            Race& race = entry.second;
            for (size_t i = 0; i < race.crates.size(); ++i)
            {
                Crate& c = race.crates[i];
                if (!c.up && c.respawnAt && int32(now - c.respawnAt) >= 0)
                {
                    c.up = true;
                    c.respawnAt = 0;
                    SendRace(race, "KARTCRAFT\tW\tU\t" + std::to_string(i));
                }
            }
            for (auto s = race.shots.begin(); s != race.shots.end();)
                s = LiveShot(s->second, now) ? std::next(s) : race.shots.erase(s);
        }
    }

    static bool ParseFloat(std::string const& text, float& out)
    {
        char* end = nullptr;
        out = std::strtof(text.c_str(), &end);
        return end != text.c_str() && *end == '\0' && std::isfinite(out);
    }

    static void HandleWeapon(Player* player, std::string const& body)
    {
        uint32 now = WorldTimer::getMSTime();
        LineRate& rate = s_weaponRate[player->GetGUIDLow()];
        if (!rate.window || WorldTimer::getMSTimeDiff(rate.window, now) >= IN_MILLISECONDS)
        {
            rate.window = now;
            rate.count = 0;
        }
        if (++rate.count > MAX_WEAPON_LINES || !player->HasAura(SPELL_SUMMON_KART))
            return;

        std::vector<std::string> f;
        for (size_t at = 0, tab; at <= body.size(); at = tab + 1)
        {
            tab = body.find('\t', at);
            if (tab == std::string::npos)
                tab = body.size();
            f.push_back(body.substr(at, tab - at));
        }
        if (f.size() < 3)
            return;

        // The sender's race: one that has passed GO with them in it.
        Race* race = nullptr;
        Racer* me = nullptr;
        for (auto& entry : s_races)
        {
            Race& r = entry.second;
            if (!r.start || int32(now - r.start) < 0 || r.crates.empty())
                continue;
            if (Racer* x = FindRacer(r, player->GetGUIDLow()))
            {
                race = &r;
                me = x;
            }
        }
        if (!race || player->GetMapId() != race->course->map)
            return;
        std::string const& verb = f[1];
        std::string const guid = std::to_string(player->GetGUIDLow());

        if (verb == "C" && !me->finished)
        {
            char* end = nullptr;
            unsigned long index = std::strtoul(f[2].c_str(), &end, 10);
            if (end == f[2].c_str() || *end || index >= race->crates.size())
                return;
            Crate& crate = race->crates[index];
            if (!crate.up || DistanceTo(player, { crate.x, crate.y }) > CRATE_RADIUS)
                return;
            crate.up = false;
            crate.respawnAt = crate.kind == 'W' ? 0 : now + CRATE_RESPAWN;
            SendRace(*race, "KARTCRAFT\tW\tX\t" + std::to_string(index) + "\t" + guid);
            if (crate.kind == 'Q')
            {
                // A racer still holding an item breaks the crate for nothing (RB_Crate.c:152-226).
                if (me->held != ITEM_NONE || me->shots)
                    return;
                uint8 item = Roll(*race, *me, now);
                me->held = item;
                me->shots = item == ITEM_BOMB_3 || item == ITEM_MISSILE_3 ? 3 : 1;
                me->claimed = now;
                SendLine(player, "KARTCRAFT\tW\tI\t" + std::to_string(item) + "\t" + std::to_string(me->shots));
            }
            else
            {
                uint32 gain = crate.kind == 'F' ? Scramble(*race) % 4 + 5 : 1;
                me->wumpa = uint8(std::min<uint32>(me->wumpa + gain, MAX_WUMPA));
                SendLine(player, "KARTCRAFT\tW\tF\t" + std::to_string(me->wumpa));
            }
        }
        else if (verb == "S" && f.size() >= 6 && !me->finished)
        {
            char* end = nullptr;
            unsigned long shot = std::strtoul(f[2].c_str(), &end, 10);
            if (end == f[2].c_str() || *end || shot < 1 || shot > 0xFFFF)
                return;
            if (me->held == ITEM_NONE || !me->shots || WorldTimer::getMSTimeDiff(me->claimed, now) < ROLL_MIN)
                return;
            std::vector<std::string> pos;
            for (size_t at = 0, comma; at <= f[3].size(); at = comma + 1)
            {
                comma = f[3].find(',', at);
                if (comma == std::string::npos)
                    comma = f[3].size();
                pos.push_back(f[3].substr(at, comma - at));
            }
            float x, y, z, yaw;
            if (pos.size() != 4 || !ParseFloat(pos[0], x) || !ParseFloat(pos[1], y) || !ParseFloat(pos[2], z) ||
                !ParseFloat(pos[3], yaw))
                return;
            uint32 target = uint32(std::strtoul(f[4].c_str(), nullptr, 10));
            std::string flags;
            for (char c : f[5])
                if ((c == 'b' || c == 'd') && flags.find(c) == std::string::npos)
                    flags += c;

            uint8 item = me->held;
            if (IsProjectile(item))
            {
                size_t live = 0;
                for (auto const& s : race->shots)
                    live += InPool(s.second, now);
                if (live >= MAX_LIVE_SHOTS)
                    return;
            }
            bool juiced = me->wumpa >= MAX_WUMPA;
            if (--me->shots == 0)
                me->held = ITEM_NONE;
            if (item == ITEM_MASK)
                me->maskUntil = now + (juiced ? MASK_TIME_JUICED : MASK_TIME);
            if (item == ITEM_WARP)
            {
                // The leader, or the next racer when the shooter leads (RB_Warpball.c:127-198).
                target = 0;
                for (Racer* r : Standings(*race))
                    if (r != me && !r->finished)
                    {
                        target = r->guid.GetCounter();
                        break;
                    }
            }
            Shot& record = race->shots[(uint64(player->GetGUIDLow()) << 16) | shot];
            record = Shot();
            record.item = item;
            record.at = now;
            record.dropped = flags.find('d') != std::string::npos;
            SendRace(*race, "KARTCRAFT\tW\tS\t" + guid + "\t" + std::to_string(item) + "\t" + (juiced ? "1" : "0") +
                "\t" + std::to_string(shot) + "\t" + Fixed(x, 2) + "," + Fixed(y, 2) + "," + Fixed(z, 2) + "," +
                Fixed(yaw, 3) + "\t" + std::to_string(target) + "\t" + flags);
        }
        else if (verb == "H" && f.size() >= 5)
        {
            uint32 shooter = uint32(std::strtoul(f[2].c_str(), nullptr, 10));
            char* end = nullptr;
            unsigned long shot = std::strtoul(f[3].c_str(), &end, 10);
            if (end == f[3].c_str() || *end || shot > 0xFFFF || f[4].size() != 1)
                return;
            char kind = f[4][0];
            if (kind != 'S' && kind != 'B' && kind != 'Q' && kind != 'T')
                return;
            bool self = shooter == player->GetGUIDLow() && shot == 0;
            if (!self)
            {
                auto found = race->shots.find((uint64(shooter) << 16) | shot);
                if (found == race->shots.end() || !LiveShot(found->second, now))
                    return;
                Shot& s = found->second;
                bool single = s.item == ITEM_MISSILE || s.item == ITEM_MISSILE_3 || s.item == ITEM_WARP;
                if (single && s.hit)
                    return;
                if (single || s.dropped)
                    s.hit = true;
            }
            // Spin-out and squish cost 1, a blast 3; a TNT landing on the head costs nothing until
            // it goes off; none while masked (RB_Player.c:153-161).
            uint32 loss = kind == 'B' ? 3 : kind == 'T' ? 0 : 1;
            if (me->maskUntil && int32(now - me->maskUntil) < 0)
                loss = 0;
            me->wumpa = uint8(me->wumpa > loss ? me->wumpa - loss : 0);
            SendRace(*race, "KARTCRAFT\tW\tH\t" + guid + "\t" + std::to_string(shooter) + "\t" + std::to_string(shot) +
                "\t" + std::string(1, kind) + "\t" + std::to_string(me->wumpa));
        }
    }

    // Sends the course and the countdown: the race is on once it runs out.
    static void StartRace(uint32 id, Race& r, uint32 now, uint32 countdown)
    {
        r.start = now + countdown;
        std::string points;
        for (auto const& p : r.course->points)
            points += (points.empty() ? "" : ";") + std::to_string(int32(p.first)) + ":" + std::to_string(int32(p.second));
        SendRace(r, "KARTCRAFT\tR\tC\t" + std::to_string(id) + "\t" + r.course->key + "\t" +
            std::to_string(r.course->laps) + "\t" + points);
        SendRace(r, "KARTCRAFT\tR\tG\t" + std::to_string(id) + "\t" + std::to_string(countdown) + "\t" + Roster(r));
        StartWeapons(r, now);
    }

    // ── Race Finder ────────────────────────────────────────────────────────────────────────────
    //
    // A dungeon finder for races: from anywhere, a player queues for some courses or any
    // ("R\tQ+\t<key,key|any>", "R\tQ-" to leave) and keeps the place with a heartbeat every 2 s
    // ("R\tW", answered "R\tQs\t<waiting>\t<secs>", or "R\tQx" when not queued). Two or more
    // sharing a course, once the later of the first two has waited FILL_WAIT or eight are in,
    // get a ready check ("R\tPop\t<pop>\t<course>\t<racers>\t<secs>"), answered "R\tY\t<pop>" or
    // "R\tN\t<pop>". All yes: each is put in the kart, teleported to a grid slot behind the start
    // and raced as above. A no or a timeout ends it ("R\tPop-\t<pop>"); those who said yes queue
    // again at the front. ponytail: matching runs on the queued clients' heartbeats, not a world
    // timer; add an update hook if queues get busy.

    constexpr uint32 FILL_WAIT = 30 * IN_MILLISECONDS;
    constexpr uint32 QUEUE_STALE = 10 * IN_MILLISECONDS;
    constexpr uint32 POP_ANSWER = 20 * IN_MILLISECONDS;
    // Countdown plus time for a teleport to settle before it.
    constexpr uint32 FINDER_COUNTDOWN = COUNTDOWN + 4 * IN_MILLISECONDS;
    constexpr float GRID_SPACING = 4.f;

    struct Queued
    {
        ObjectGuid guid;
        std::vector<bool> courses; // [i]: COURSES[i]
        uint32 joined;
        uint32 seen;
    };

    struct Pop
    {
        Course const* course = nullptr;
        uint32 created = 0;
        std::vector<Queued> members;
        std::vector<ObjectGuid> accepted;
    };

    static std::vector<Queued> s_queue;
    static std::unordered_map<uint32, Pop> s_pops;
    static uint32 s_nextPop = 1;

    // The Race Finder's zones, in the order courses.json first names them: the client asks for a
    // zone by its place in this list ("z3"), which keeps a queue line short however many it ticks.
    static std::vector<std::string> const& Zones()
    {
        static std::vector<std::string> zones = [] {
            std::vector<std::string> out;
            for (Course const& c : COURSES)
                if (*c.zone && std::find(out.begin(), out.end(), c.zone) == out.end())
                    out.push_back(c.zone);
            return out;
        }();
        return zones;
    }

    // The courses a queue line asks for: "any", or a comma list of zones ("z<index>") and keys.
    static std::vector<bool> CourseMask(std::string const& asked)
    {
        std::vector<bool> mask(COURSES.size(), false);
        std::vector<std::string> const& zones = Zones();
        for (size_t at = 0, comma; at <= asked.size(); at = comma + 1)
        {
            comma = asked.find(',', at);
            if (comma == std::string::npos)
                comma = asked.size();
            std::string token = asked.substr(at, comma - at);
            std::string const* zone = nullptr;
            if (token.size() > 1 && token[0] == 'z' && std::isdigit(uint8(token[1])))
            {
                size_t index = std::strtoul(token.c_str() + 1, nullptr, 10);
                if (index < zones.size())
                    zone = &zones[index];
            }
            for (size_t i = 0; i < COURSES.size(); ++i)
                if (token == "any" || token == COURSES[i].key || (zone && *zone == COURSES[i].zone))
                    mask[i] = true;
        }
        return mask;
    }

    static bool AnyCourse(std::vector<bool> const& mask)
    {
        return std::find(mask.begin(), mask.end(), true) != mask.end();
    }

    static bool Overlap(std::vector<bool> const& a, std::vector<bool> const& b)
    {
        for (size_t i = 0; i < a.size() && i < b.size(); ++i)
            if (a[i] && b[i])
                return true;
        return false;
    }

    static void SendTo(ObjectGuid guid, std::string const& text)
    {
        if (Player* p = ObjectAccessor::FindPlayer(guid))
            SendLine(p, text);
    }

    // A grid slot behind the start, two abreast, facing down the course.
    // Whether the Race Finder may take a player off to a race now: not dead, fighting or already
    // on the move, not on a flight path, a boat or a zeppelin, and not inside a dungeon, a raid
    // or a battleground (a teleport out of those would lose their place there).
    static bool CanBeRacedOff(Player* player)
    {
        return player->IsAlive() && !player->IsInCombat() && !player->IsBeingTeleported() &&
            !player->IsTaxiFlying() && !player->GetTransport() && !player->InBattleGround() &&
            !player->GetMap()->Instanceable();
    }

    static void TeleportToGrid(Player* player, Course const& c, size_t slot)
    {
        // Their situation may have changed since they said yes: then they stay where they are.
        if (!CanBeRacedOff(player))
            return;
        float sx = c.points[0].first, sy = c.points[0].second;
        float dx = c.points[1].first - sx, dy = c.points[1].second - sy;
        float len = std::max(std::sqrt(dx * dx + dy * dy), 1.f);
        dx /= len;
        dy /= len;
        float back = GRID_SPACING * float(slot / 2 + 1);
        float side = GRID_SPACING * (slot % 2 ? 0.5f : -0.5f);
        float x = sx - dx * back - dy * side;
        float y = sy - dy * back + dx * side;
        float ground = sTerrainMgr.LoadTerrain(c.map)->GetHeightStatic(x, y, 500.f);
        // No terrain data there: better left where they are, to drive over, than dropped from the sky.
        if (ground <= INVALID_HEIGHT)
            return;
        float z = ground + 0.5f;
        if (!player->HasAura(SPELL_SUMMON_KART))
        {
            player->RemoveSpellsCausingAura(SPELL_AURA_MOUNTED);
            player->CastSpell(player, SPELL_SUMMON_KART, true);
        }
        player->TeleportTo(c.map, x, y, z, std::atan2(dy, dx));
    }

    static void EndPop(uint32 id, Pop& pop, uint32 now)
    {
        // Those who said yes go back to the front of the queue, keeping their place.
        for (Queued q : pop.members)
            if (std::find(pop.accepted.begin(), pop.accepted.end(), q.guid) != pop.accepted.end())
            {
                q.seen = now;
                s_queue.insert(s_queue.begin(), q);
            }
        for (Queued const& q : pop.members)
            SendTo(q.guid, "KARTCRAFT\tR\tPop-\t" + std::to_string(id));
    }

    static void LaunchPop(Pop& pop, uint32 now)
    {
        uint32 raceId = s_nextRace++;
        Race& r = s_races[raceId];
        r.course = pop.course;
        r.host = pop.members.front().guid;
        r.created = now;
        size_t slot = 0;
        for (Queued const& q : pop.members)
            if (Player* p = ObjectAccessor::FindPlayer(q.guid))
            {
                r.racers.push_back({ p->GetObjectGuid(), p->GetName() });
                TeleportToGrid(p, *pop.course, slot++);
            }
        SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(raceId) + "\t" + Roster(r));
        StartRace(raceId, r, now, FINDER_COUNTDOWN);
    }

    // Drops the stale, fails late ready checks and pops the courses that are ready.
    static void UpdateFinder(uint32 now)
    {
        s_queue.erase(std::remove_if(s_queue.begin(), s_queue.end(), [now](Queued const& q) {
            return WorldTimer::getMSTimeDiff(q.seen, now) > QUEUE_STALE || !ObjectAccessor::FindPlayer(q.guid);
        }), s_queue.end());

        for (auto it = s_pops.begin(); it != s_pops.end();)
            if (WorldTimer::getMSTimeDiff(it->second.created, now) > POP_ANSWER)
            {
                EndPop(it->first, it->second, now);
                it = s_pops.erase(it);
            }
            else
                it = std::next(it);

        // The course with the most takers that is ready, oldest first.
        std::stable_sort(s_queue.begin(), s_queue.end(), [now](Queued const& a, Queued const& b) {
            return WorldTimer::getMSTimeDiff(a.joined, now) > WorldTimer::getMSTimeDiff(b.joined, now);
        });
        int best = -1;
        size_t bestCount = 0;
        for (size_t i = 0; i < COURSES.size(); ++i)
        {
            std::vector<Queued const*> takers;
            for (Queued const& q : s_queue)
                if (q.courses[i])
                    takers.push_back(&q);
            bool ready = takers.size() >= MAX_RACERS ||
                (takers.size() >= 2 && WorldTimer::getMSTimeDiff(takers[1]->joined, now) >= FILL_WAIT);
            if (ready && takers.size() > bestCount)
            {
                best = int(i);
                bestCount = takers.size();
            }
        }
        if (best < 0)
            return;
        uint32 popId = s_nextPop++;
        Pop& pop = s_pops[popId];
        pop.course = &COURSES[best];
        pop.created = now;
        for (auto it = s_queue.begin(); it != s_queue.end() && pop.members.size() < MAX_RACERS;)
            if (it->courses[best])
            {
                pop.members.push_back(*it);
                it = s_queue.erase(it);
            }
            else
                it = std::next(it);
        std::string check = "KARTCRAFT\tR\tPop\t" + std::to_string(popId) + "\t" + pop.course->key + "\t" +
            std::to_string(pop.members.size()) + "\t" + std::to_string(POP_ANSWER / IN_MILLISECONDS);
        for (Queued const& q : pop.members)
            SendTo(q.guid, check);
    }

    // The finder's verbs; false when the line is not one of them.
    static bool HandleFinder(Player* player, std::vector<std::string> const& f, uint32 now)
    {
        std::string const& verb = f[1];
        ObjectGuid guid = player->GetObjectGuid();
        auto mine = std::find_if(s_queue.begin(), s_queue.end(), [guid](Queued const& q) { return q.guid == guid; });
        if (verb == "Q+" && f.size() > 2)
        {
            std::vector<bool> mask = CourseMask(f[2]);
            // Not from a dungeon, a raid or a battleground: a pop would teleport them out of it.
            if (player->InBattleGround() || player->GetMap()->Instanceable() || !AnyCourse(mask))
                return true;
            // A race still waiting to start is left for the queue; one under way is not: a pop
            // would teleport them off the course, so they are told why instead.
            for (auto it = s_races.begin(); it != s_races.end();)
            {
                Race& r = it->second;
                auto x = std::find_if(r.racers.begin(), r.racers.end(), [guid](Racer const& m) { return m.guid == guid; });
                if (x == r.racers.end() || x->finished)
                {
                    it = std::next(it);
                    continue;
                }
                if (r.start)
                {
                    SendLine(player, "KARTCRAFT\tR\tQx\tracing");
                    return true;
                }
                RemoveRacer(r, x);
                SendLine(player, "KARTCRAFT\tR\tL\t" + std::to_string(it->first) + "\t");
                if (r.racers.empty())
                    it = s_races.erase(it);
                else
                {
                    SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(it->first) + "\t" + Roster(r));
                    it = std::next(it);
                }
            }
            if (mine != s_queue.end())
                mine->courses = mask;
            else
                s_queue.push_back({ guid, mask, now, now });
        }
        else if (verb == "Q-")
        {
            if (mine != s_queue.end())
                s_queue.erase(mine);
        }
        else if (verb == "W")
        {
            if (mine == s_queue.end())
            {
                bool pending = false;
                for (auto const& p : s_pops)
                    for (Queued const& q : p.second.members)
                        pending |= q.guid == guid;
                if (!pending)
                    SendLine(player, "KARTCRAFT\tR\tQx");
                return true;
            }
            mine->seen = now;
            size_t waiting = 0;
            for (Queued const& q : s_queue)
                waiting += Overlap(q.courses, mine->courses) ? 1 : 0;
            SendLine(player, "KARTCRAFT\tR\tQs\t" + std::to_string(waiting) + "\t" +
                std::to_string(WorldTimer::getMSTimeDiff(mine->joined, now) / IN_MILLISECONDS));
        }
        else if ((verb == "Y" || verb == "N") && f.size() > 2)
        {
            uint32 id = uint32(std::strtoul(f[2].c_str(), nullptr, 10));
            auto pop = s_pops.find(id);
            if (pop == s_pops.end())
                return true;
            Pop& p = pop->second;
            bool member = std::any_of(p.members.begin(), p.members.end(), [guid](Queued const& q) { return q.guid == guid; });
            if (!member)
                return true;
            if (verb == "N")
            {
                p.accepted.erase(std::remove(p.accepted.begin(), p.accepted.end(), guid), p.accepted.end());
                EndPop(id, p, now);
                s_pops.erase(pop);
                return true;
            }
            // A racer who can't be raced off yet (dead, fighting, flying, in a dungeon...): their yes
            // waits until they can.
            if (!CanBeRacedOff(player))
                return true;
            if (std::find(p.accepted.begin(), p.accepted.end(), guid) == p.accepted.end())
                p.accepted.push_back(guid);
            if (p.accepted.size() == p.members.size())
            {
                LaunchPop(p, now);
                s_pops.erase(pop);
            }
        }
        else if (verb == "X")
        {
            // Out of everything (/leaverace, F6): the queue, any ready check (as a no), and every
            // race, waiting or under way. Always answered, so a client sure it is racing when the
            // server is not lets go too; the shields come off with the next tick.
            if (mine != s_queue.end())
                s_queue.erase(mine);
            for (auto it = s_pops.begin(); it != s_pops.end();)
            {
                Pop& p = it->second;
                bool member = std::any_of(p.members.begin(), p.members.end(), [guid](Queued const& q) { return q.guid == guid; });
                if (!member)
                {
                    it = std::next(it);
                    continue;
                }
                p.accepted.erase(std::remove(p.accepted.begin(), p.accepted.end(), guid), p.accepted.end());
                EndPop(it->first, p, now);
                it = s_pops.erase(it);
            }
            for (auto it = s_races.begin(); it != s_races.end();)
            {
                Race& r = it->second;
                auto x = std::find_if(r.racers.begin(), r.racers.end(), [guid](Racer const& m) { return m.guid == guid; });
                if (x == r.racers.end())
                {
                    it = std::next(it);
                    continue;
                }
                RemoveRacer(r, x);
                if (r.racers.empty())
                    it = s_races.erase(it);
                else
                {
                    SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(it->first) + "\t" + Roster(r));
                    it = std::next(it);
                }
            }
            // The race shields too, whatever the bookkeeping says (TickShields).
            s_shielded.erase(guid);
            sWorld.GetMessager().AddMessage([guid](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    p->RemoveFlag(UNIT_FIELD_FLAGS, RACE_SHIELD_FLAGS);
            });
            SendLine(player, "KARTCRAFT\tR\tX");
        }
        else
            return false;
        return true;
    }

    static void HandleRace(Player* player, std::string const& body)
    {
        uint32 now = WorldTimer::getMSTime();
        for (auto it = s_races.begin(); it != s_races.end();)
            it = WorldTimer::getMSTimeDiff(it->second.created, now) > RACE_LIFETIME ? s_races.erase(it) : std::next(it);
        UpdateFinder(now);

        std::vector<std::string> f;
        for (size_t at = 0, tab; at <= body.size(); at = tab + 1)
        {
            tab = body.find('\t', at);
            if (tab == std::string::npos)
                tab = body.size();
            f.push_back(body.substr(at, tab - at));
        }
        // The finder's verbs work on foot; the rest need the kart.
        if (f.size() < 2 || HandleFinder(player, f, now) || !player->HasAura(SPELL_SUMMON_KART))
            return;
        std::string const& verb = f[1];
        uint32 id = f.size() > 2 ? uint32(std::strtoul(f[2].c_str(), nullptr, 10)) : 0;
        auto race = s_races.find(id);

        if (verb == "H" && f.size() > 2)
        {
            Course const* course = FindCourse(f[2]);
            if (!course || player->GetMapId() != course->map || DistanceTo(player, course->points[0]) > START_RADIUS)
                return;
            // A race already waiting on this course takes them in: who arrives after the offer
            // (sent once, to those near) still races with the host rather than alone.
            for (auto& entry : s_races)
            {
                uint32 openId = entry.first;
                Race& open = entry.second;
                bool in = std::any_of(open.racers.begin(), open.racers.end(),
                    [player](Racer const& x) { return x.guid == player->GetObjectGuid(); });
                if (open.course == course && !open.start && (in || open.racers.size() < MAX_RACERS))
                {
                    if (!in)
                        open.racers.push_back({ player->GetObjectGuid(), player->GetName() });
                    SendRace(open, "KARTCRAFT\tR\tL\t" + std::to_string(openId) + "\t" + Roster(open));
                    return;
                }
            }
            uint32 raceId = s_nextRace++;
            Race& r = s_races[raceId];
            r.course = course;
            r.host = player->GetObjectGuid();
            r.created = now;
            r.racers.push_back({ player->GetObjectGuid(), player->GetName() });
            WorldPacket data;
            std::string open = "KARTCRAFT\tR\tO\t" + std::to_string(raceId) + "\t" + course->key + "\t" +
                course->name + "\t" + player->GetName();
            ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, open.c_str(), LANG_ADDON, CHAT_TAG_NONE,
                player->GetObjectGuid(), player->GetName());
            player->SendMessageToSetInRange(&data, START_RADIUS * 2.f, true);
            SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(raceId) + "\t" + Roster(r));
        }
        else if (verb == "J" && race != s_races.end())
        {
            Race& r = race->second;
            if (r.start || r.racers.size() >= MAX_RACERS || DistanceTo(player, r.course->points[0]) > START_RADIUS)
                return;
            for (Racer const& x : r.racers)
                if (x.guid == player->GetObjectGuid())
                    return;
            r.racers.push_back({ player->GetObjectGuid(), player->GetName() });
            SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(id) + "\t" + Roster(r));
        }
        else if (verb == "S" && race != s_races.end())
        {
            Race& r = race->second;
            if (r.start || r.host != player->GetObjectGuid())
                return;
            StartRace(id, r, now, COUNTDOWN);
        }
        else if (verb == "K" && race != s_races.end() && f.size() > 3)
        {
            Race& r = race->second;
            if (!r.start || int32(now - r.start) < 0)
                return;
            Racer* me = nullptr;
            for (Racer& x : r.racers)
                if (x.guid == player->GetObjectGuid())
                    me = &x;
            if (!me || me->finished)
                return;
            Course const& c = *r.course;
            uint32 n = uint32(c.points.size());
            uint32 index = uint32(std::strtoul(f[3].c_str(), nullptr, 10));
            uint32 expected = c.laps ? (me->checkpoint + 1) % n : me->checkpoint + 1;
            if (index != expected || index >= n || DistanceTo(player, c.points[index]) > GateRadius(c, index))
                return;
            me->checkpoint = uint8(index);
            bool done = false;
            if (c.laps && index == 0)
                done = ++me->lap >= c.laps;
            else if (!c.laps && index == n - 1)
                done = true;
            SendRace(r, "KARTCRAFT\tR\tP\t" + std::to_string(id) + "\t" + me->name + "\t" +
                std::to_string(me->lap) + "\t" + std::to_string(me->checkpoint));
            if (done)
            {
                me->finished = true;
                uint32 place = ++r.finishers;
                me->place = uint8(place);
                uint32 ms = WorldTimer::getMSTimeDiff(r.start, now);
                SendRace(r, "KARTCRAFT\tR\tF\t" + std::to_string(id) + "\t" + me->name + "\t" +
                    std::to_string(place) + "\t" + std::to_string(ms));
                Reward(player, place, uint32(r.racers.size()));
                if (r.finishers >= r.racers.size())
                    s_races.erase(race);
            }
        }
        else if (verb == "Q" && race != s_races.end())
        {
            // Leaving: out of the roster before the start, a retirement after.
            Race& r = race->second;
            auto x = std::find_if(r.racers.begin(), r.racers.end(),
                [player](Racer const& m) { return m.guid == player->GetObjectGuid(); });
            if (x != r.racers.end())
                RemoveRacer(r, x);
            if (r.racers.empty())
                s_races.erase(race);
            else
                SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(id) + "\t" + Roster(r));
        }
    }

    bool HandleAddonMessage(Player* player, std::string const& message)
    {
        static std::string const prefix = "KARTCRAFT\t";
        if (message.compare(0, prefix.size(), prefix) != 0)
            return false;
        std::string body = message.substr(prefix.size());
        std::lock_guard<std::mutex> lock(s_mutex);
        TickWeapons(WorldTimer::getMSTime());
        if (body.compare(0, 2, "L\t") == 0)
        {
            float points = float(std::atof(body.c_str() + 2));
            if (points > 0.f)
                LandLine(player, points);
        }
        else if (body.compare(0, 2, "S\t") == 0)
            RelayState(player, message);
        else if (body.compare(0, 2, "R\t") == 0)
            HandleRace(player, body);
        else if (body.compare(0, 2, "W\t") == 0)
            HandleWeapon(player, body);
        // After the line: a finish or a leave in it frees the racer at once.
        TickShields(WorldTimer::getMSTime());
        return true;
    }

    SpellCastResult CheckCast(Unit const* caster, SpellEntry const* spell)
    {
        switch (spell->Id)
        {
            case SPELL_SUMMON_KART:
                if (caster->IsMounted())
                    return SPELL_FAILED_NOT_MOUNTED;
                if (caster->IsInCombat())
                    return SPELL_FAILED_AFFECTING_COMBAT;
                break;
            case SPELL_WUMPA_JUICE:
            case SPELL_TURBO_BOOST:
            case SPELL_SUPER_ENGINE:
            case SPELL_AKU_AKU:
                if (!caster->HasAura(SPELL_SUMMON_KART))
                    return SPELL_FAILED_ONLY_MOUNTED;
                break;
        }
        return SPELL_CAST_OK;
    }
}
