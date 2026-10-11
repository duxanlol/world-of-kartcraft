#include "Kartcraft.h"
#include "Database/DBCStores.h"
#include "Chat.h"
#include "Player.h"
#include "SpellMgr.h"
#include "Timer.h"
#include "ObjectAccessor.h"
#include "WorldSession.h"
#include "World.h"
#include "Creature.h"
#include "ObjectMgr.h"
#include "GossipDef.h"
#include "ScriptMgr.h"
#include "Database/DatabaseEnv.h"

#include <cmath>
#include <ctime>
#include <map>
#include <string>
#include <vector>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <random>
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

    static void LeaveRaces(Player* player);

    // ── Cross-faction ──────────────────────────────────────────────────────────────────────────
    //
    // A driver is friendly to everyone, and everyone to them: Horde and Alliance race the same
    // courses, all in Alliance land. Faction template 35 (friendly to all, as the Race Marshal)
    // covers players and creatures without a reputation; reactions to a faction with one go by
    // reputation instead (WorldObject::GetReactionTo), so a Horde driver would still be
    // Stormwind's guards'. Every such faction the player is below Friendly with or at war with is
    // forced Friendly while the kart is out, as SPELL_AURA_FORCE_REACTION does. A faction already
    // forced (a disguise's aura) is left to it, and leaving the kart lifts only the kart's own.
    static constexpr uint32 FACTION_FRIENDLY_TO_ALL = 35;

    // The factions each driver's kart forced, by low guid. Aura handlers run on map threads,
    // and a cast of the kart may come from under s_mutex: this lock is its own.
    static std::unordered_map<uint32, std::set<uint32>> s_forced;
    static std::mutex s_forcedMutex;

    static bool IsForced(ReputationMgr const& reputation, uint32 faction)
    {
        // The lookup reads only the template's faction.
        FactionTemplateEntry probe{};
        probe.faction = faction;
        return reputation.GetForcedRankIfAny(&probe) != nullptr;
    }

    void OnMounted(Unit* target, uint32 spell, bool apply)
    {
        Player* player = target->ToPlayer();
        if (!player || spell != SPELL_SUMMON_KART)
            return;
        ReputationMgr& reputation = player->GetReputationMgr();
        std::set<uint32> ours;
        {
            std::lock_guard<std::mutex> lock(s_forcedMutex);
            auto it = s_forced.find(player->GetGUIDLow());
            if (it != s_forced.end())
            {
                ours = std::move(it->second);
                s_forced.erase(it);
            }
        }
        if (apply)
        {
            for (auto const& faction : sObjectMgr.GetFactionMap())
            {
                FactionEntry const& entry = faction.second;
                if (!entry.CanHaveReputation() || IsForced(reputation, faction.first))
                    continue;
                FactionState const* state = reputation.GetState(&entry);
                if (reputation.GetRank(&entry) >= REP_FRIENDLY && !(state && (state->Flags & FACTION_FLAG_AT_WAR)))
                    continue;
                reputation.ApplyForceReaction(faction.first, REP_FRIENDLY, true);
                player->StopAttackFaction(faction.first);
                ours.insert(faction.first);
            }
            std::lock_guard<std::mutex> lock(s_forcedMutex);
            s_forced[player->GetGUIDLow()] = std::move(ours);
        }
        else
            for (uint32 faction : ours)
                reputation.ApplyForceReaction(faction, REP_FRIENDLY, false);
        if (apply)
            player->SetFactionTemplateId(FACTION_FRIENDLY_TO_ALL);
        else
            player->RestoreFaction();
        if (player->IsInWorld())
            reputation.SendForceReactions();
    }

    void OnLogin(Player* player)
    {
        // The whole map discovered, every zone's every area, as `.explorecheat 1` sets it
        // (CharacterCommands.cpp): the courses are all over the world, and the track browser
        // draws them on it. Saved with the character, so it holds offline too.
        for (uint8 i = 0; i < PLAYER_EXPLORED_ZONES_SIZE; ++i)
            player->SetFlag(PLAYER_EXPLORED_ZONES_1 + i, 0xFFFFFFFF);
        // A client logs in knowing of no race: it is out of any it was in, or a waiting race
        // would hold it (the marshal offers neither hosting nor joining) until it expired.
        LeaveRaces(player);
        // A kart kept over the logout: its aura came back before the client was in the world.
        if (player->HasAura(SPELL_SUMMON_KART))
            OnMounted(player, SPELL_SUMMON_KART, true);
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
        // Where its Race Marshal stands (WoW x, y): courses sharing a start share the marshal.
        std::pair<float, float> marshal;
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
        // A fired shield not yet launched as a shield bomb: when (ms, 0 none), and juiced (blue,
        // no time limit).
        uint32 shieldAt = 0;
        bool shieldJuiced = false;
    };

    // A weapon crate ('Q'), a fruit crate ('F') or a single Wumpa fruit ('W') on a race's course.
    struct Crate
    {
        char kind;
        float x, y, z;
        bool up = true;
        uint32 respawnAt = 0; // ms; 0 while up, or never (a single fruit)
        uint32 grownAt = 0;   // ms; until then it regrows and cannot be taken; 0 full
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

    // ── Records ────────────────────────────────────────────────────────────────────────────────
    //
    // Every finish is a time on its course: each character's best there, and the course record
    // with its holder. kartcraft_records in the characters database keeps every character's best:
    // the core creates it if missing (the world database may be re-created by vmangos-deploy, the
    // characters database never is), reads it once at startup, and writes a new best in the
    // background (an async execute), so a finish never waits on the database. The record is the
    // least of the bests.

    struct Record
    {
        uint32 ms = 0; // 0: none yet
        std::string name;
    };

    static std::map<std::pair<uint32, std::string>, uint32> s_bests; // (guid low, course key) -> ms
    static std::unordered_map<std::string, Record> s_records;        // course key -> its record

    static void LoadRecords()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_records` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'course key, courses/courses.json', "
            "`name` VARCHAR(12) NOT NULL COMMENT 'character name when it was set', "
            "`ms` INT UNSIGNED NOT NULL COMMENT 'race time, ms', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when, unix time', "
            "PRIMARY KEY (`guid`, `course`), KEY `course_ms` (`course`, `ms`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        std::unique_ptr<QueryResult> result =
            CharacterDatabase.Query("SELECT `guid`, `course`, `name`, `ms` FROM `kartcraft_records`");
        if (!result)
            return;
        uint32 count = 0;
        do
        {
            Field* fields = result->Fetch();
            uint32 guid = fields[0].GetUInt32();
            std::string course = fields[1].GetCppString();
            uint32 ms = fields[3].GetUInt32();
            if (!ms)
                continue;
            s_bests[{ guid, course }] = ms;
            Record& record = s_records[course];
            if (!record.ms || ms < record.ms)
                record = { ms, fields[2].GetCppString() };
            ++count;
        } while (result->NextRow());
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "Kartcraft: %u best times on %u courses.", count, uint32(s_records.size()));
    }

    // A finish of `ms` on `course`: "r" a new course record, "p" a new personal best, else "".
    static std::string RecordFinish(uint32 guid, std::string const& name, Course const& course, uint32 ms)
    {
        if (!ms)
            return "";
        uint32& best = s_bests[{ guid, course.key }];
        if (best && best <= ms)
            return "";
        best = ms;
        std::string escaped = name;
        CharacterDatabase.escape_string(escaped);
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_records` (`guid`, `course`, `name`, `ms`, `at`) "
            "VALUES (%u, '%s', '%s', %u, %u)", guid, course.key, escaped.c_str(), ms, uint32(std::time(nullptr)));
        Record& record = s_records[course.key];
        if (record.ms && record.ms <= ms)
            return "p";
        record = { ms, name };
        return "r";
    }

    static uint32 BestOf(uint32 guid, std::string const& course)
    {
        auto found = s_bests.find({ guid, course });
        return found == s_bests.end() ? 0 : found->second;
    }

    // "1:02.40", as the client shows a race time.
    static std::string Clock(uint32 ms)
    {
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%u:%02u.%02u", ms / 60000, ms / 1000 % 60, ms % 1000 / 10);
        return buf;
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

    // A crate comes back after its 30-frame cooldown (RB_Crate.c:125), then regrows 16 frames
    // and cannot be taken until full (RB_Crate.c:48-75, 123).
    constexpr uint32 CRATE_RESPAWN = 1000;
    constexpr uint32 CRATE_GROW = 533;
    // How near a crate (yards, across the ground) a racer must be to take it. The kart touches
    // it 2 yd off, but our position here trails the kart: up to 42 yd/s at top speed, a 1/15 s
    // heartbeat and the world update's turn are 5-8 yd.
    constexpr float CRATE_RADIUS = 12.f;
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
    // A green shield lasts 12 s (VehPickupItem.c:976-983); a second's slack for the line.
    constexpr uint32 SHIELD_TIME = 13 * IN_MILLISECONDS;
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
            r.shieldAt = 0;
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
    // Race mode, asked for from the minimap button's menu, is the same shield at will, untargetable
    // too: for waiting in the Race Finder's queue in peace. Never entered in combat; a login ends it.
    // Each shielded player by the flags they wear.
    static std::map<ObjectGuid, uint32> s_shielded;
    static std::set<ObjectGuid> s_raceMode;
    static uint32 s_shieldedAt = 0;
    constexpr uint32 SHIELD_INTERVAL = 250;
    constexpr uint32 RACE_SHIELD_FLAGS = UNIT_FLAG_IMMUNE_TO_NPC | UNIT_FLAG_IMMUNE_TO_PLAYER;
    constexpr uint32 RACE_MODE_FLAGS = RACE_SHIELD_FLAGS | UNIT_FLAG_NOT_SELECTABLE;

    // Run after every Kartcraft line: a racer let go is freed at once; those racing are covered
    // again at most every SHIELD_INTERVAL.
    static void TickShields(uint32 now)
    {
        std::map<ObjectGuid, uint32> want;
        for (auto const& entry : s_races)
            if (entry.second.start)
                for (Racer const& r : entry.second.racers)
                    if (!r.finished)
                        want[r.guid] |= RACE_SHIELD_FLAGS;
        for (ObjectGuid const& guid : s_raceMode)
            want[guid] |= RACE_MODE_FLAGS;
        // The flags each player no longer needs.
        std::vector<std::pair<ObjectGuid, uint32>> off;
        for (auto const& had : s_shielded)
        {
            auto kept = want.find(had.first);
            uint32 gone = had.second & ~(kept == want.end() ? 0 : kept->second);
            if (gone)
                off.emplace_back(had.first, gone);
        }
        bool newcomer = std::any_of(want.begin(), want.end(),
            [](std::pair<ObjectGuid const, uint32> const& w)
            {
                auto had = s_shielded.find(w.first);
                return had == s_shielded.end() || had->second != w.second;
            });
        s_shielded = want;
        bool due = !want.empty() && WorldTimer::getMSTimeDiff(s_shieldedAt, now) >= SHIELD_INTERVAL;
        if (off.empty() && !newcomer && !due)
            return;
        s_shieldedAt = now;
        sWorld.GetMessager().AddMessage([want, off](World*)
        {
            for (auto const& w : off)
                if (Player* p = ObjectAccessor::FindPlayer(w.first))
                    p->RemoveFlag(UNIT_FIELD_FLAGS, w.second);
            for (auto const& w : want)
                if (Player* p = ObjectAccessor::FindPlayer(w.first))
                    if ((p->GetUInt32Value(UNIT_FIELD_FLAGS) & w.second) != w.second)
                    {
                        p->SetFlag(UNIT_FIELD_FLAGS, w.second);
                        p->CombatStopWithPets(true);
                        p->GetHostileRefManager().deleteReferences();
                    }
        });
    }

    // "R\tM\t1" or "R\tM\t0": race mode on or off. The answer is "R\tM\t<1 or 0>", or "R\tMx"
    // when it can't come on (in combat). The shield follows with the line's TickShields.
    static void RaceMode(Player* player, bool on)
    {
        if (on && player->IsInCombat())
        {
            SendLine(player, "KARTCRAFT\tR\tMx");
            return;
        }
        if (on)
            s_raceMode.insert(player->GetObjectGuid());
        else
            s_raceMode.erase(player->GetObjectGuid());
        SendLine(player, std::string("KARTCRAFT\tR\tM\t") + (on ? "1" : "0"));
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
                    c.grownAt = (now + CRATE_GROW) | 1; // never 0, which means full
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

    // "<x>,<y>,<z>,<yaw>" as the wire writes it, back to the wire's decimals; empty if malformed.
    static std::string ParsePlace(std::string const& text)
    {
        std::vector<std::string> pos;
        for (size_t at = 0, comma; at <= text.size(); at = comma + 1)
        {
            comma = text.find(',', at);
            if (comma == std::string::npos)
                comma = text.size();
            pos.push_back(text.substr(at, comma - at));
        }
        float x, y, z, yaw;
        if (pos.size() != 4 || !ParseFloat(pos[0], x) || !ParseFloat(pos[1], y) || !ParseFloat(pos[2], z) ||
            !ParseFloat(pos[3], yaw))
            return "";
        return Fixed(x, 2) + "," + Fixed(y, 2) + "," + Fixed(z, 2) + "," + Fixed(yaw, 3);
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
            if (crate.grownAt && int32(now - crate.grownAt) >= 0)
                crate.grownAt = 0;
            if (!crate.up || crate.grownAt || DistanceTo(player, { crate.x, crate.y }) > CRATE_RADIUS)
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
            std::string const place = ParsePlace(f[3]);
            if (place.empty())
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
            if (item == ITEM_SHIELD)
            {
                me->shieldAt = now ? now : 1;
                me->shieldJuiced = juiced;
            }
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
                "\t" + std::to_string(shot) + "\t" + place + "\t" + std::to_string(target) + "\t" + flags);
        }
        else if (verb == "D" && !me->finished)
        {
            // Fire again with a bomb rolling: only with a bomb of the sender's still in the pool.
            uint32 const low = player->GetGUIDLow();
            bool rolling = std::any_of(race->shots.begin(), race->shots.end(),
                [low, now](std::pair<uint64 const, Shot> const& s)
                {
                    return uint32(s.first >> 16) == low &&
                        (s.second.item == ITEM_BOMB || s.second.item == ITEM_BOMB_3) && InPool(s.second, now);
                });
            if (rolling)
                SendRace(*race, "KARTCRAFT\tW\tD\t" + guid);
        }
        else if (verb == "Z" && f.size() >= 4 && !me->finished)
        {
            // Fire with a shield up: it leaves as a shield bomb, once per shield.
            char* end = nullptr;
            unsigned long shot = std::strtoul(f[2].c_str(), &end, 10);
            if (end == f[2].c_str() || *end || shot < 1 || shot > 0xFFFF || !me->shieldAt)
                return;
            if (!me->shieldJuiced && WorldTimer::getMSTimeDiff(me->shieldAt, now) > SHIELD_TIME)
            {
                me->shieldAt = 0;
                return;
            }
            std::string const place = ParsePlace(f[3]);
            if (place.empty())
                return;
            me->shieldAt = 0;
            Shot& record = race->shots[(uint64(player->GetGUIDLow()) << 16) | shot];
            record = Shot();
            record.item = ITEM_SHIELD;
            record.at = now;
            SendRace(*race, "KARTCRAFT\tW\tZ\t" + guid + "\t" + std::to_string(shot) + "\t" + place);
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

    // Each racer's "R\tRec": the course record and its holder, and their own best (ms, 0 none).
    static void SendRecords(uint32 id, Race const& r)
    {
        Record const& record = s_records[r.course->key];
        for (Racer const& x : r.racers)
            if (Player* p = ObjectAccessor::FindPlayer(x.guid))
                SendLine(p, "KARTCRAFT\tR\tRec\t" + std::to_string(id) + "\t" + r.course->key + "\t" +
                std::to_string(record.ms) + "\t" + record.name + "\t" +
                std::to_string(BestOf(x.guid.GetCounter(), r.course->key)));
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
        SendRecords(id, r);
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
    //
    // The open Race Finder also shows the queue and the lobbies ("R\tLobby", SendLobby): how many
    // wait for each zone, and the races hosted at a start; one of those is joined from anywhere
    // ("R\tLJ\t<race>"), and the host's start brings everyone to its grid.

    constexpr uint32 FILL_WAIT = 30 * IN_MILLISECONDS;
    // The wait in force: a GM sets it for testing ("R\tFillWait\t<secs>", /racewait), until the
    // server restarts.
    static uint32 s_fillWait = FILL_WAIT;
    constexpr uint32 FILL_WAIT_MAX = 300 * IN_MILLISECONDS;
    constexpr uint32 QUEUE_STALE = 10 * IN_MILLISECONDS;
    constexpr uint32 POP_ANSWER = 20 * IN_MILLISECONDS;
    // Countdown plus time for a teleport to settle before it.
    constexpr uint32 GRID_COUNTDOWN = COUNTDOWN + 4 * IN_MILLISECONDS;
    // More when a racer comes from another map: a loading screen, then the world streaming in
    // before the client acknowledges it, so they reach the grid before GO.
    constexpr uint32 FAR_SETTLE = 20 * IN_MILLISECONDS;
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

    // Puts a driver on foot (or on another mount) in the kart: a triggered cast, with no cast bar
    // to interrupt; the kart aura lands when the cast time has run.
    static void SummonKart(Player* player)
    {
        if (player->HasAura(SPELL_SUMMON_KART))
            return;
        player->RemoveSpellsCausingAura(SPELL_AURA_MOUNTED);
        player->CastSpell(player, SPELL_SUMMON_KART, true);
    }

    // How long after a teleport the kart is summoned again (ms), and how often it looks again
    // while the teleport is still landing (the client's ack, a far teleport's load).
    constexpr uint32 KART_AGAIN_DELAY = 1500;
    constexpr uint32 KART_AGAIN_RETRY = 500;

    // The kart summoned once a teleport has landed. It runs on the player's own event queue,
    // which ticks only while they are in a world, so after a far teleport's load as well.
    class KartAgainEvent : public BasicEvent
    {
    public:
        explicit KartAgainEvent(Player* player) : m_player(player) { }

        bool Execute(uint64 /*e_time*/, uint32 /*p_time*/) override
        {
            if (m_player->IsBeingTeleported())
                m_player->m_Events.AddEventAtOffset(new KartAgainEvent(m_player), KART_AGAIN_RETRY);
            else if (m_player->IsInWorld() && m_player->IsAlive())
                SummonKart(m_player);
            return true;
        }

    private:
        Player* m_player;
    };

    // To (x, y) on `map`, on the ground there, facing `o`, and in the kart once there if `kart`
    // (onto a grid: a countdown follows, or no race). A driver going onto a grid is taken out of
    // the kart for the teleport and put back in after it: on foot a teleport is the stock one,
    // and the client starts the kart again where it put them rather than carrying the kart's old
    // pose over it; the countdown's line gives the kart time to come back. Without `kart` (back
    // to a checkpoint mid-race, where no countdown would cover the kart's absence) a driver stays
    // in it. No terrain data there: better left where they are, to drive over, than dropped from
    // the sky.
    static bool TeleportOnto(Player* player, uint32 map, float x, float y, float o, bool kart)
    {
        float ground = sTerrainMgr.LoadTerrain(map)->GetHeightStatic(x, y, 500.f);
        if (ground <= INVALID_HEIGHT)
            return false;
        if (kart)
            player->RemoveAurasDueToSpell(SPELL_SUMMON_KART);
        player->TeleportTo(map, x, y, ground + 0.5f, o);
        if (kart)
            player->m_Events.AddEventAtOffset(new KartAgainEvent(player), KART_AGAIN_DELAY);
        return true;
    }

    static bool TeleportToGrid(Player* player, Course const& c, size_t slot)
    {
        // Their situation may have changed since they said yes: then they stay where they are.
        if (!CanBeRacedOff(player))
            return false;
        float sx = c.points[0].first, sy = c.points[0].second;
        float dx = c.points[1].first - sx, dy = c.points[1].second - sy;
        float len = std::max(std::sqrt(dx * dx + dy * dy), 1.f);
        dx /= len;
        dy /= len;
        float back = GRID_SPACING * float(slot / 2 + 1);
        float side = GRID_SPACING * (slot % 2 ? 0.5f : -0.5f);
        float x = sx - dx * back - dy * side;
        float y = sy - dy * back + dx * side;
        return TeleportOnto(player, c.map, x, y, std::atan2(dy, dx), true);
    }

    // Where the Race Finder took each racer from, so they can go back when their race is over, as
    // the dungeon finder's teleport out does: offered ("R\tRet") once they are out of every race,
    // answered "R\tBack" (back there) or "R\tStay" (forget it). Forgotten after RETURN_KEEP and
    // at login.
    struct Origin
    {
        uint32 map;
        float x, y, z, o;
        uint32 at; // ms
    };

    static std::unordered_map<uint32, Origin> s_origins; // guid low -> where they were
    constexpr uint32 RETURN_KEEP = 10 * MINUTE * IN_MILLISECONDS;

    static bool InAnyRace(ObjectGuid guid)
    {
        for (auto const& entry : s_races)
            for (Racer const& r : entry.second.racers)
                if (r.guid == guid)
                    return true;
        return false;
    }

    // Out of every race with a spot to go back to: the client asks whether to.
    static void OfferReturn(Player* player, uint32 now)
    {
        auto found = s_origins.find(player->GetGUIDLow());
        if (found == s_origins.end())
            return;
        if (WorldTimer::getMSTimeDiff(found->second.at, now) > RETURN_KEEP)
        {
            s_origins.erase(found);
            return;
        }
        if (!InAnyRace(player->GetObjectGuid()))
            SendLine(player, "KARTCRAFT\tR\tRet");
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
        bool far = false;
        for (Queued const& q : pop.members)
            if (Player* p = ObjectAccessor::FindPlayer(q.guid))
            {
                far |= p->GetMapId() != pop.course->map;
                r.racers.push_back({ p->GetObjectGuid(), p->GetName() });
                Origin from = { p->GetMapId(), p->GetPositionX(), p->GetPositionY(), p->GetPositionZ(),
                    p->GetOrientation(), now };
                if (TeleportToGrid(p, *pop.course, slot++))
                    s_origins[p->GetGUIDLow()] = from;
            }
        SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(raceId) + "\t" + Roster(r));
        StartRace(raceId, r, now, far ? GRID_COUNTDOWN + FAR_SETTLE : GRID_COUNTDOWN);
    }

    // Drops the stale, fails late ready checks and pops the courses that are ready.
    static void UpdateFinder(uint32 now)
    {
        s_queue.erase(std::remove_if(s_queue.begin(), s_queue.end(), [now](Queued const& q) {
            return WorldTimer::getMSTimeDiff(q.seen, now) > QUEUE_STALE || !ObjectAccessor::FindPlayer(q.guid);
        }), s_queue.end());
        for (auto it = s_origins.begin(); it != s_origins.end();)
            it = WorldTimer::getMSTimeDiff(it->second.at, now) > RETURN_KEEP ? s_origins.erase(it) : std::next(it);

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
        // Among the courses tied for the most takers (every one of them, for racers who ticked a
        // whole zone or "any"), one at random: the first in the list would always win.
        int best = -1;
        size_t bestCount = 0;
        uint32 ties = 0;
        for (size_t i = 0; i < COURSES.size(); ++i)
        {
            std::vector<Queued const*> takers;
            for (Queued const& q : s_queue)
                if (q.courses[i])
                    takers.push_back(&q);
            bool ready = takers.size() >= MAX_RACERS ||
                (takers.size() >= 2 && WorldTimer::getMSTimeDiff(takers[1]->joined, now) >= s_fillWait);
            if (!ready || takers.size() < bestCount)
                continue;
            if (takers.size() > bestCount)
            {
                bestCount = takers.size();
                ties = 0;
            }
            // Each tied course kept with chance 1/ties: a uniform pick in one pass.
            static std::mt19937 s_pick{ std::random_device{}() };
            if (std::uniform_int_distribution<uint32>(0, ties++)(s_pick) == 0)
                best = int(i);
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

    // Out of every race still waiting to start; false, and nothing left, when one is under way.
    static bool LeaveWaitingRaces(Player* player)
    {
        ObjectGuid guid = player->GetObjectGuid();
        for (auto const& entry : s_races)
            for (Racer const& x : entry.second.racers)
                if (x.guid == guid && !x.finished && entry.second.start)
                    return false;
        for (auto it = s_races.begin(); it != s_races.end();)
        {
            Race& r = it->second;
            auto x = std::find_if(r.racers.begin(), r.racers.end(), [guid](Racer const& m) { return m.guid == guid; });
            if (x == r.racers.end() || x->finished)
            {
                it = std::next(it);
                continue;
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
        return true;
    }

    // What the Race Finder shows while it is open ("R\tLobby", every 2 s): each zone's queued
    // players, "z<index>=<n>" for those with any, and the races hosted at a start that wait for
    // their host, "<id>:<course>:<host>:<racers>". Answered "R\tLobby\t<zones,...>\t<races;...>".
    static void SendLobby(Player* player)
    {
        std::vector<std::string> const& zones = Zones();
        std::string counts;
        for (size_t z = 0; z < zones.size(); ++z)
        {
            size_t n = 0;
            for (Queued const& q : s_queue)
                for (size_t i = 0; i < COURSES.size(); ++i)
                    if (q.courses[i] && zones[z] == COURSES[i].zone)
                    {
                        ++n;
                        break;
                    }
            if (n)
                counts += (counts.empty() ? "z" : ",z") + std::to_string(z) + "=" + std::to_string(n);
        }
        std::string open;
        for (auto const& entry : s_races)
        {
            Race const& r = entry.second;
            if (r.start || r.racers.empty())
                continue;
            std::string host = r.racers.front().name;
            for (Racer const& x : r.racers)
                if (x.guid == r.host)
                    host = x.name;
            open += (open.empty() ? "" : ";") + std::to_string(entry.first) + ":" + r.course->key + ":" +
                host + ":" + std::to_string(r.racers.size());
        }
        SendLine(player, "KARTCRAFT\tR\tLobby\t" + counts + "\t" + open);
    }

    // The finder's verbs; false when the line is not one of them.
    static bool HandleFinder(Player* player, std::vector<std::string> const& f, uint32 now)
    {
        std::string const& verb = f[1];
        ObjectGuid guid = player->GetObjectGuid();
        auto mine = std::find_if(s_queue.begin(), s_queue.end(), [guid](Queued const& q) { return q.guid == guid; });
        if (verb == "FillWait")
        {
            // GM accounts only; a bare /racewait reports the wait in force.
            ChatHandler chat(player->GetSession());
            if (player->GetSession()->GetSecurity() < SEC_GAMEMASTER)
            {
                chat.SendSysMessage("Only a GM can change the Race Finder's wait.");
                return true;
            }
            if (f.size() > 2 && !f[2].empty())
                s_fillWait = std::min(uint32(std::strtoul(f[2].c_str(), nullptr, 10)) * IN_MILLISECONDS, FILL_WAIT_MAX);
            chat.PSendSysMessage("Race Finder wait: %u s (default %u s, until the server restarts).",
                s_fillWait / IN_MILLISECONDS, FILL_WAIT / IN_MILLISECONDS);
        }
        else if (verb == "Q+" && f.size() > 2)
        {
            std::vector<bool> mask = CourseMask(f[2]);
            // Not from a dungeon, a raid or a battleground: a pop would teleport them out of it.
            if (player->InBattleGround() || player->GetMap()->Instanceable() || !AnyCourse(mask))
                return true;
            // A race still waiting to start is left for the queue; one under way is not: a pop
            // would teleport them off the course, so they are told why instead.
            if (!LeaveWaitingRaces(player))
            {
                SendLine(player, "KARTCRAFT\tR\tQx\tracing");
                return true;
            }
            if (mine != s_queue.end())
                mine->courses = mask;
            else
                s_queue.push_back({ guid, mask, now, now });
        }
        else if (verb == "Lobby")
            SendLobby(player);
        else if (verb == "LJ" && f.size() > 2)
        {
            // A hosted race joined from the Race Finder's list, from anywhere: in the kart and on
            // the roster where they are, as a pop's racer; the host's start puts everyone on the
            // grid. Out of the queue and any other race still waiting.
            uint32 id = uint32(std::strtoul(f[2].c_str(), nullptr, 10));
            auto race = s_races.find(id);
            if (race == s_races.end() || race->second.start || race->second.racers.size() >= MAX_RACERS ||
                !CanBeRacedOff(player) ||
                std::any_of(race->second.racers.begin(), race->second.racers.end(),
                    [guid](Racer const& x) { return x.guid == guid; }))
                return true;
            if (!LeaveWaitingRaces(player))
            {
                SendLine(player, "KARTCRAFT\tR\tQx\tracing");
                return true;
            }
            // Leaving may have emptied and erased races; this one had others in it, but look again.
            race = s_races.find(id);
            if (race == s_races.end())
                return true;
            if (mine != s_queue.end())
                s_queue.erase(mine);
            SummonKart(player);
            Race& r = race->second;
            r.racers.push_back({ guid, player->GetName() });
            SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(race->first) + "\t" + Roster(r));
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
            OfferReturn(player, now);
        }
        else if (verb == "Back" || verb == "Stay")
        {
            auto found = s_origins.find(player->GetGUIDLow());
            if (found == s_origins.end() || InAnyRace(guid))
                return true;
            Origin const to = found->second;
            s_origins.erase(found);
            if (verb == "Stay" || WorldTimer::getMSTimeDiff(to.at, now) > RETURN_KEEP)
                return true;
            // A teleport belongs to the world thread: posted to its messager, as a Race Marshal's
            // choice is, and only for a player who could be raced off (alive, not fighting...).
            sWorld.GetMessager().AddMessage([guid, to](World*)
            {
                Player* p = ObjectAccessor::FindPlayer(guid);
                if (p && p->IsInWorld() && CanBeRacedOff(p))
                    p->TeleportTo(to.map, to.x, to.y, to.z, to.o);
            });
        }
        else
            return false;
        return true;
    }

    // Out of every race (not the Race Finder's queue), as a retirement; the shield comes off.
    static void LeaveRaces(Player* player)
    {
        ObjectGuid const guid = player->GetObjectGuid();
        std::lock_guard<std::mutex> lock(s_mutex);
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
        // Race mode too: a login starts without it.
        s_raceMode.erase(guid);
        if (s_shielded.erase(guid))
            player->RemoveFlag(UNIT_FIELD_FLAGS, RACE_MODE_FLAGS);
        // A spot to go back to does not outlive the session.
        s_origins.erase(guid.GetCounter());
    }

    // "R\tU": a kart stuck in the world (sunk into a hill, wedged in a crease) put back on a course:
    // in a race under way at the last checkpoint it passed, facing the next, so the race goes on;
    // else at the start of the nearest course within UNSTUCK_REACH. Once every UNSTUCK_COOLDOWN
    // and never what CanBeRacedOff refuses (dead, fighting, flying, in an instance), so it is
    // neither a way out of a fight nor a free road across the world. "R\tUx\t<secs>" says how
    // long to wait, "R\tUn" that no course starts near enough.
    constexpr uint32 UNSTUCK_COOLDOWN = 30 * IN_MILLISECONDS;
    constexpr float UNSTUCK_REACH = 300.f; // yards
    static std::unordered_map<uint32, uint32> s_unstuck; // guid low -> when last used (ms)

    static void Unstuck(Player* player, uint32 now)
    {
        if (!CanBeRacedOff(player))
            return;
        auto used = s_unstuck.find(player->GetGUIDLow());
        if (used != s_unstuck.end() && WorldTimer::getMSTimeDiff(used->second, now) < UNSTUCK_COOLDOWN)
        {
            uint32 left = UNSTUCK_COOLDOWN - WorldTimer::getMSTimeDiff(used->second, now);
            SendLine(player, "KARTCRAFT\tR\tUx\t" + std::to_string((left + 999) / IN_MILLISECONDS));
            return;
        }
        for (auto const& entry : s_races)
        {
            Race const& r = entry.second;
            if (!r.start || int32(now - r.start) < 0)
                continue;
            for (Racer const& x : r.racers)
            {
                if (x.guid != player->GetObjectGuid() || x.finished)
                    continue;
                Course const& c = *r.course;
                size_t n = c.points.size();
                auto const& at = c.points[std::min<size_t>(x.checkpoint, n - 1)];
                auto const& next = c.points[c.laps ? (x.checkpoint + 1) % n : std::min<size_t>(x.checkpoint + 1, n - 1)];
                float o = std::atan2(next.second - at.second, next.first - at.first);
                if (TeleportOnto(player, c.map, at.first, at.second, o, false))
                    s_unstuck[player->GetGUIDLow()] = now;
                return;
            }
        }
        Course const* nearest = nullptr;
        float best = 0.f;
        for (Course const& c : COURSES)
        {
            if (c.map != player->GetMapId() || c.points.size() < 2)
                continue;
            float d = DistanceTo(player, c.points[0]);
            if (!nearest || d < best)
            {
                nearest = &c;
                best = d;
            }
        }
        if (!nearest || best > UNSTUCK_REACH)
        {
            SendLine(player, "KARTCRAFT\tR\tUn");
            return;
        }
        if (TeleportToGrid(player, *nearest, 0))
            s_unstuck[player->GetGUIDLow()] = now;
    }

    // `marshal`: the line comes from a Race Marshal's menu (MarshalSelect), which the player used
    // standing at the start: the start's distance and the kart are not asked for.
    static void HandleRace(Player* player, std::string const& body, bool marshal)
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
        // Unstuck and race mode work in the kart or out of it.
        if (f.size() >= 2 && f[1] == "U")
        {
            Unstuck(player, now);
            return;
        }
        if (f.size() >= 3 && f[1] == "M")
        {
            RaceMode(player, f[2] == "1");
            return;
        }
        // The finder's verbs work on foot; the rest need the kart, but for a marshal's and for
        // leaving: a racer the marshal put in on foot may leave before the kart's aura lands.
        if (f.size() < 2 || HandleFinder(player, f, now) ||
            (!marshal && f[1] != "Q" && !player->HasAura(SPELL_SUMMON_KART)))
            return;
        std::string const& verb = f[1];
        uint32 id = f.size() > 2 ? uint32(std::strtoul(f[2].c_str(), nullptr, 10)) : 0;
        auto race = s_races.find(id);

        if (verb == "H" && f.size() > 2)
        {
            Course const* course = FindCourse(f[2]);
            if (!course || player->GetMapId() != course->map || (!marshal && DistanceTo(player, course->points[0]) > START_RADIUS))
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
            if (r.start || r.racers.size() >= MAX_RACERS || player->GetMapId() != r.course->map ||
                (!marshal && DistanceTo(player, r.course->points[0]) > START_RADIUS))
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
            // Everyone to a grid slot behind the start, as a Race Finder race lines up, whether
            // they joined on foot or drove up in the kart; the countdown waits for the settle.
            size_t slot = 0;
            bool far = false;
            for (Racer const& x : r.racers)
                if (Player* p = ObjectAccessor::FindPlayer(x.guid))
                {
                    // One who joined from the Race Finder's list may be on another map.
                    far |= p->GetMapId() != r.course->map;
                    // A racer still fighting would be refused the teleport (CanBeRacedOff); the
                    // race's shield ends the fight anyway, so it ends before the grid.
                    if (p->IsInCombat())
                    {
                        p->CombatStopWithPets(true);
                        p->GetHostileRefManager().deleteReferences();
                    }
                    TeleportToGrid(p, *r.course, slot++);
                }
            StartRace(id, r, now, far ? GRID_COUNTDOWN + FAR_SETTLE : GRID_COUNTDOWN);
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
                std::string const best = RecordFinish(player->GetGUIDLow(), me->name, c, ms);
                SendRace(r, "KARTCRAFT\tR\tF\t" + std::to_string(id) + "\t" + me->name + "\t" +
                    std::to_string(place) + "\t" + std::to_string(ms) + "\t" + best);
                if (!best.empty())
                    SendRecords(id, r);
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
        // Out of the race (a finished one may already be gone): back where the finder took them?
        if (verb == "Q")
            OfferReturn(player, now);
    }

    static std::string const PREFIX = "KARTCRAFT\t";

    // One Kartcraft line, without its prefix; `marshal` as for HandleRace.
    static void HandleLine(Player* player, std::string const& body, bool marshal)
    {
        std::lock_guard<std::mutex> lock(s_mutex);
        TickWeapons(WorldTimer::getMSTime());
        if (body.compare(0, 2, "L\t") == 0)
        {
            float points = float(std::atof(body.c_str() + 2));
            if (points > 0.f)
                LandLine(player, points);
        }
        else if (body.compare(0, 2, "S\t") == 0)
            RelayState(player, PREFIX + body);
        else if (body.compare(0, 2, "R\t") == 0)
            HandleRace(player, body, marshal);
        else if (body.compare(0, 2, "W\t") == 0)
            HandleWeapon(player, body);
        // After the line: a finish or a leave in it frees the racer at once.
        TickShields(WorldTimer::getMSTime());
    }

    bool HandleAddonMessage(Player* player, std::string const& message)
    {
        if (message.compare(0, PREFIX.size(), PREFIX) != 0)
            return false;
        HandleLine(player, message.substr(PREFIX.size()), false);
        return true;
    }

    // ── Race Marshal ───────────────────────────────────────────────────────────────────────────
    //
    // A gossip NPC beside every course start (setup/vmangos/sql/kartcraft.sql, placed by
    // `course-gen marshals`): its menu hosts, joins, starts and leaves races there, and opens the
    // client's Race Finder ("R\tFinder"). Gossip runs on the player's map thread, and maps update
    // in parallel: the menu only reads the races, under the lock, and a choice is posted to the
    // world's messager, which runs it between map updates as the addon line it stands for, so a
    // choice and its key do the same thing.

    constexpr uint32 MARSHAL_TEXT = 190000;      // npc_text: the greeting
    constexpr uint32 MARSHAL_HELP_TEXT = 190001; // npc_text: "How do races work?"
    constexpr uint32 MARSHAL_RECORDS_TEXT = 190002; // npc_text: over the course records
    // A marshal belongs to the courses whose "marshal" spot is its spawn point, this near (yards).
    constexpr float MARSHAL_MATCH = 2.f;

    enum MarshalOption : uint32
    {
        MARSHAL_HOST = 1, // action: the course's index in COURSES
        MARSHAL_JOIN,     // action: the race id
        MARSHAL_START,    // action: the race id
        MARSHAL_LEAVE,
        MARSHAL_FINDER,
        MARSHAL_HELP,
        MARSHAL_BACK,
        MARSHAL_RECORDS,
        MARSHAL_TRACKS, // action: the course's index in COURSES the browser opens at
    };

    // The courses that start at this marshal: those placed with it, else (a marshal spawned
    // anywhere else) those whose start is within START_RADIUS of it.
    static std::vector<size_t> MarshalCourses(Creature* marshal)
    {
        float x, y, z;
        marshal->GetRespawnCoord(x, y, z);
        std::vector<size_t> here, near;
        for (size_t i = 0; i < COURSES.size(); ++i)
        {
            Course const& c = COURSES[i];
            if (c.map != marshal->GetMapId())
                continue;
            if (std::hypot(c.marshal.first - x, c.marshal.second - y) <= MARSHAL_MATCH)
                here.push_back(i);
            else if (std::hypot(c.points[0].first - x, c.points[0].second - y) <= START_RADIUS)
                near.push_back(i);
        }
        return here.empty() ? near : here;
    }

    static bool AtMarshal(std::vector<size_t> const& here, Course const* course)
    {
        return std::any_of(here.begin(), here.end(), [course](size_t i) { return &COURSES[i] == course; });
    }

    static bool MarshalHello(Player* player, Creature* marshal)
    {
        std::vector<size_t> const here = MarshalCourses(marshal);
        ObjectGuid const guid = player->GetObjectGuid();
        PlayerMenu* talk = player->PlayerTalkClass;
        talk->ClearMenus();
        GossipMenu& menu = talk->GetGossipMenu();
        // Room for the three lines every menu ends with.
        size_t room = GOSSIP_MAX_MENU_ITEMS - 3;
        bool busy = false;
        {
            std::lock_guard<std::mutex> lock(s_mutex);
            uint32 const now = WorldTimer::getMSTime();
            busy = std::any_of(s_queue.begin(), s_queue.end(), [guid](Queued const& q) { return q.guid == guid; });
            for (auto const& p : s_pops)
                for (Queued const& q : p.second.members)
                    busy |= q.guid == guid;
            std::vector<Course const*> waiting;
            std::vector<std::pair<uint32, Race const*>> joinable;
            for (auto const& entry : s_races)
            {
                Race const& r = entry.second;
                bool in = std::any_of(r.racers.begin(), r.racers.end(), [guid](Racer const& x) { return x.guid == guid; });
                busy |= in;
                if (r.start || WorldTimer::getMSTimeDiff(r.created, now) > RACE_LIFETIME || !AtMarshal(here, r.course))
                    continue;
                waiting.push_back(r.course);
                if (r.host == guid && room)
                {
                    menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Start the race on " + std::string(r.course->name) + " (" +
                        std::to_string(r.racers.size()) + (r.racers.size() == 1 ? " racer)" : " racers)"),
                        MARSHAL_START, entry.first, "", false);
                    --room;
                }
                else if (!in && r.racers.size() < MAX_RACERS)
                    joinable.emplace_back(entry.first, &r);
            }
            for (auto const& j : joinable)
            {
                if (!room)
                    break;
                Race const& r = *j.second;
                std::string host = r.racers.front().name;
                for (Racer const& x : r.racers)
                    if (x.guid == r.host)
                        host = x.name;
                menu.AddMenuItem(GOSSIP_ICON_TALK, "Join " + host + "'s race on " + r.course->name + " (" +
                    std::to_string(r.racers.size()) + (r.racers.size() == 1 ? " racer)" : " racers)"),
                    MARSHAL_JOIN, j.first, "", false);
                --room;
            }
            // A course with a race waiting is joined, not hosted again (as "R\tH" itself does).
            for (size_t i : here)
            {
                Course const& c = COURSES[i];
                if (!room)
                    break;
                if (std::find(waiting.begin(), waiting.end(), &c) != waiting.end())
                    continue;
                menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Host a race: " + std::string(c.name) + " (" +
                    (c.laps ? std::to_string(c.laps) + (c.laps == 1 ? " lap)" : " laps)") : std::string("sprint)")),
                    MARSHAL_HOST, uint32(i), "", false);
                --room;
            }
        }
        if (busy)
            menu.AddMenuItem(GOSSIP_ICON_CHAT, "Leave the race", MARSHAL_LEAVE, 0, "", false);
        menu.AddMenuItem(GOSSIP_ICON_DOT, "Open the Race Finder", MARSHAL_FINDER, 0, "", false);
        // The client's track browser, at her own course: its description and its route on the map.
        menu.AddMenuItem(GOSSIP_ICON_DOT, "Browse the tracks", MARSHAL_TRACKS,
            here.empty() ? uint32(COURSES.size()) : uint32(here.front()), "", false);
        if (!here.empty())
            menu.AddMenuItem(GOSSIP_ICON_CHAT, "Course records", MARSHAL_RECORDS, 0, "", false);
        menu.AddMenuItem(GOSSIP_ICON_CHAT, "How do races work?", MARSHAL_HELP, 0, "", false);
        talk->SendGossipMenu(MARSHAL_TEXT, marshal->GetObjectGuid());
        return true;
    }

    // The courses that start here, a line each: the record and its holder, and the player's own
    // best. A line chosen shows the page again.
    static void MarshalRecords(Player* player, Creature* marshal)
    {
        std::vector<size_t> const here = MarshalCourses(marshal);
        uint32 const guid = player->GetGUIDLow();
        GossipMenu& menu = player->PlayerTalkClass->GetGossipMenu();
        menu.ClearMenu();
        {
            std::lock_guard<std::mutex> lock(s_mutex);
            for (size_t i = 0; i < here.size() && i + 1 < GOSSIP_MAX_MENU_ITEMS; ++i)
            {
                Course const& c = COURSES[here[i]];
                auto record = s_records.find(c.key);
                std::string line = std::string(c.name) + ": ";
                if (record == s_records.end() || !record->second.ms)
                    line += "no record yet";
                else
                    line += Clock(record->second.ms) + " by " + record->second.name;
                if (uint32 best = BestOf(guid, c.key))
                    line += " (yours " + Clock(best) + ")";
                menu.AddMenuItem(GOSSIP_ICON_DOT, line, MARSHAL_RECORDS, 0, "", false);
            }
        }
        menu.AddMenuItem(GOSSIP_ICON_CHAT, "Back", MARSHAL_BACK, 0, "", false);
        player->PlayerTalkClass->SendGossipMenu(MARSHAL_RECORDS_TEXT, marshal->GetObjectGuid());
    }

    static bool MarshalSelect(Player* player, Creature* marshal, uint32 sender, uint32 action)
    {
        std::string line;
        switch (sender)
        {
            case MARSHAL_HOST:
                if (action < COURSES.size() && AtMarshal(MarshalCourses(marshal), &COURSES[action]))
                    line = std::string("R\tH\t") + COURSES[action].key;
                break;
            case MARSHAL_JOIN:
            case MARSHAL_START:
            {
                std::vector<size_t> const here = MarshalCourses(marshal);
                std::lock_guard<std::mutex> lock(s_mutex);
                auto race = s_races.find(action);
                if (race != s_races.end() && AtMarshal(here, race->second.course))
                    line = (sender == MARSHAL_JOIN ? "R\tJ\t" : "R\tS\t") + std::to_string(action);
                break;
            }
            case MARSHAL_LEAVE:
                line = "R\tX";
                break;
            case MARSHAL_FINDER:
                line = "R\tFinder";
                break;
            case MARSHAL_TRACKS:
                line = std::string("R\tTracks\t") + (action < COURSES.size() ? COURSES[action].key : "");
                break;
            case MARSHAL_HELP:
                player->PlayerTalkClass->GetGossipMenu().AddMenuItem(GOSSIP_ICON_CHAT, "Back", MARSHAL_BACK, 0, "", false);
                player->PlayerTalkClass->SendGossipMenu(MARSHAL_HELP_TEXT, marshal->GetObjectGuid());
                return true;
            case MARSHAL_BACK:
                return MarshalHello(player, marshal);
            case MARSHAL_RECORDS:
                MarshalRecords(player, marshal);
                return true;
        }
        player->PlayerTalkClass->CloseGossip();
        if (line.empty())
            return true;
        ObjectGuid const guid = player->GetObjectGuid();
        sWorld.GetMessager().AddMessage([guid, line](World*)
        {
            Player* p = ObjectAccessor::FindPlayer(guid);
            if (!p || !p->IsInWorld())
                return;
            if (line == "R\tFinder" || line.compare(0, 8, "R\tTracks") == 0)
            {
                // The client opens its Race Finder, or its track browser, at this line.
                SendLine(p, PREFIX + line);
                return;
            }
            // Hosting or joining on foot: into the kart first, as a Race Finder pop does.
            bool const enters = line.compare(0, 4, "R\tH\t") == 0 || line.compare(0, 4, "R\tJ\t") == 0;
            if (enters && CanBeRacedOff(p))
                SummonKart(p);
            HandleLine(p, line, true);
        });
        return true;
    }

    void RegisterScripts()
    {
        // ScriptMgr keeps it only if creature_template names it (sql/kartcraft.sql), and frees it.
        Script* marshal = new Script;
        marshal->Name = "npc_kartcraft_marshal";
        marshal->pGossipHello = &MarshalHello;
        marshal->pGossipSelect = &MarshalSelect;
        marshal->RegisterSelf(true);
        LoadRecords();
    }

    SpellCastResult CheckCast(Unit const* caster, SpellEntry const* spell)
    {
        // No duels in a race under way, asked by a racer or of one: the race's shield keeps
        // players apart, and the duel's cast would also dismount the challenger's kart.
        for (uint32 i = 0; i < MAX_EFFECT_INDEX; ++i)
            if (spell->Effect[i] == SPELL_EFFECT_DUEL)
            {
                std::lock_guard<std::mutex> lock(s_mutex);
                if (s_shielded.count(caster->GetObjectGuid()))
                    return SPELL_FAILED_AFFECTING_COMBAT;
                if (s_shielded.count(caster->GetTargetGuid()))
                    return SPELL_FAILED_BAD_TARGETS;
                break;
            }
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
