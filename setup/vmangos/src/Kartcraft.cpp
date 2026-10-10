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
#include <cstdlib>
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
    constexpr float STATE_RANGE = 150.f;
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
    // Karting skill by place. The course table lives here; clients learn it from "R\tC".

    struct Course
    {
        char const* key;
        char const* name;
        uint32 map;
        // 0: a sprint from the first checkpoint to the last; else laps back through the first.
        uint8 laps;
        std::vector<std::pair<float, float>> points; // WoW x, y
    };

    // Elwynn Forest, along its roads: the waypoints of the guards who patrol them (the world
    // database's `creature_movement`: Northshire Guard 79948 down the valley, Stormwind Guard
    // 80263 from the valley's gate to Goldshire, the city's gate guard 99140), with the farms
    // and Goldshire's inn between.
    static std::vector<Course> const COURSES = {
        { "sprint", "Northshire Sprint", 0, 0,
          { { -8930.f, -118.f }, { -8989.f, -96.f }, { -9040.f, -60.f }, { -9076.f, -46.f },
            { -9123.f, -84.f },
            { -9228.f, -105.f }, { -9346.f, -45.f }, { -9436.f, 59.f } } },
        { "elwynn", "Elwynn Loop", 0, 3,
          { { -9436.f, 59.f }, { -9640.f, 150.f }, { -9880.f, 320.f }, { -9960.f, 180.f },
            { -9960.f, 50.f }, { -9750.f, 60.f } } },
        { "stormwind", "Road to Stormwind", 0, 0,
          { { -9436.f, 59.f }, { -9350.f, 150.f }, { -9250.f, 260.f }, { -9165.f, 352.f },
            { -9121.f, 395.f }, { -9073.f, 427.f } } },
    };

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
    };

    struct Race
    {
        Course const* course = nullptr;
        ObjectGuid host;
        uint32 created = 0;
        uint32 start = 0; // 0 until started
        uint8 finishers = 0;
        std::vector<Racer> racers;
    };

    static std::unordered_map<uint32, Race> s_races;
    static uint32 s_nextRace = 1;

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
        uint32 courses; // bit i: COURSES[i]
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

    static uint32 CourseMask(std::string const& keys)
    {
        uint32 mask = 0;
        for (size_t i = 0; i < COURSES.size(); ++i)
            if (keys == "any" || ("," + keys + ",").find(std::string(",") + COURSES[i].key + ",") != std::string::npos)
                mask |= 1u << i;
        return mask;
    }

    static void SendTo(ObjectGuid guid, std::string const& text)
    {
        if (Player* p = ObjectAccessor::FindPlayer(guid))
            SendLine(p, text);
    }

    // A grid slot behind the start, two abreast, facing down the course.
    static void TeleportToGrid(Player* player, Course const& c, size_t slot)
    {
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
                if (q.courses & (1u << i))
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
            if (it->courses & (1u << best))
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
            uint32 mask = CourseMask(f[2]);
            // Not while racing: a pop would teleport them off the course.
            for (auto const& r : s_races)
                for (Racer const& x : r.second.racers)
                    if (x.guid == guid && !x.finished)
                        mask = 0;
            if (!mask)
                return true;
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
                waiting += (q.courses & mine->courses) ? 1 : 0;
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
            // A dead or fighting racer can't be raced off; their yes waits until they can.
            if (!player->IsAlive() || player->IsInCombat() || player->IsBeingTeleported())
                return true;
            if (std::find(p.accepted.begin(), p.accepted.end(), guid) == p.accepted.end())
                p.accepted.push_back(guid);
            if (p.accepted.size() == p.members.size())
            {
                LaunchPop(p, now);
                s_pops.erase(pop);
            }
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
            if (index != expected || index >= n || DistanceTo(player, c.points[index]) > CHECKPOINT_RADIUS)
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
            for (auto x = r.racers.begin(); x != r.racers.end(); ++x)
                if (x->guid == player->GetObjectGuid())
                {
                    r.racers.erase(x);
                    break;
                }
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
