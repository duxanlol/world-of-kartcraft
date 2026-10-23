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
#include "GameObject.h"
#include "Group.h"
#include "MapManager.h"
#include "QuestDef.h"
#include "Utilities/Random.h"
#include "Maps/MapPersistentStateMgr.h"
#include "BattleGround.h"
#include "BattleGroundMgr.h"
#include "Config/Config.h"
#include "Mail.h"
#include <zlib.h>
#include <fstream>
#include <sys/stat.h>

#include <cmath>
#include <ctime>
#include <map>
#include <string>
#include <vector>

#include <algorithm>
#include <cstdio>
#include <deque>
#include <tuple>
#include <cstdlib>
#include <atomic>
#include <mutex>
#include <shared_mutex>
#include <random>
#include <set>
#include <sstream>
#include <functional>
#include <unordered_map>
#include <unordered_set>
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
    static std::unordered_map<uint32, uint32> s_lastState; // player guid low -> ms, s_stateMutex
    // RelayState's own: a state line is relayed outside s_mutex, the race lock.
    static std::mutex s_stateMutex;
    // Each driver's last relayed state line, for their race's take (Replays), s_stateMutex.
    static std::unordered_map<uint32, std::string> s_stateLines;

    // World of Kartcraft's own cosmetics (Goods, below): the catalogue, ahead of the relay that
    // holds what a driver shows to it.
    struct Good
    {
        char const* key;
        char kind;    // 'K' a kart, 'F' a flag
        uint32 price; // tokens
        uint8 tier;   // 0 sold in the shop; 1 to 4 found only in Mystery Crates (common .. legendary)
    };

    static Good const GOODS[] = {
        { "student", 'F', 100, 0 },
        { "fel", 'K', 400, 0 },
        { "alliance", 'K', 250, 0 },
        { "wok", 'K', 500, 0 },
        // The Mirage Raceway's racers, drawn from each player's own install (the client's
        // kart/wow_karts.rs).
        { "gnomecar", 'K', 500, 0 },
        { "goblincar", 'K', 500, 0 },
    };

    static Good const* FindGood(std::string const& key)
    {
        for (Good const& g : GOODS)
            if (key == g.key)
                return &g;
        return nullptr;
    }

    // A flag's wire index (the client's FLAGS: the flags in GOODS' order, from 1); 0 none.
    static uint32 FlagIndex(std::string const& key)
    {
        uint32 i = 0;
        for (Good const& g : GOODS)
            if (g.kind == 'F')
            {
                ++i;
                if (key == g.key)
                    return i;
            }
        return 0;
    }

    // What a driver may show on the relay: the custom karts they own, the flag they fly, and the
    // paint and trail the server last found earned (SendCosmetics). By guid, s_stateMutex.
    struct Shown
    {
        std::set<std::string> karts;
        uint32 flag = 0;
        uint32 paint = 0;
        uint32 trail = 0;
    };
    static std::unordered_map<uint32, Shown> s_shown;

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
        // A mount goes anywhere, indoors too: no mount is outdoors-only, so neither Spell::CheckCast
        // nor the indoor sweep (Player::CheckAreaExploreAndOutdoor) refuses or drops one. The
        // map, ship and Booty Bay refusals are gone from Spell::CheckCast (kartcraft.patch).
        for (uint32 id = 1; id < sSpellMgr.GetMaxSpellId(); ++id)
            if (SpellEntry const* spell = sSpellMgr.GetSpellEntry(id))
                if (spell->HasAura(SPELL_AURA_MOUNTED))
                    const_cast<SpellEntry*>(spell)->Attributes &= ~SPELL_ATTR_ONLY_OUTDOORS;
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

    bool InKart(Player const* player)
    {
        return player->HasAura(SPELL_SUMMON_KART);
    }

    static void ChallengeGreeting(Player* player);

    static void DeliveryLogin(Player* player);

    static void SkinLogin(Player* player);
    static void GoodsLogin(Player* player);
    static void PropsLogin(Player* player);
    static void ReplayLogin(Player* player);

    static void MarathonLogin(Player* player);

    // Key to Searing Gorge (item_template 5396), given to every character at login.
    constexpr uint32 KEY_SEARING_GORGE = 5396;

    void OnLogin(Player* player)
    {
        // The whole map discovered, every zone's every area, as `.explorecheat 1` sets it
        // (CharacterCommands.cpp): the courses are all over the world, and the track browser
        // draws them on it. Saved with the character, so it holds offline too.
        for (uint8 i = 0; i < PLAYER_EXPLORED_ZONES_SIZE; ++i)
            player->SetFlag(PLAYER_EXPLORED_ZONES_1 + i, 0xFFFFFFFF);
        // The Key to Searing Gorge for everyone: its gate is a way between Loch Modan and Searing
        // Gorge that courses and deliveries drive, and the key is a long questline's reward.
        if (!player->HasItemCount(KEY_SEARING_GORGE, 1, true))
        {
            ItemPosCountVec dest;
            if (player->CanStoreNewItem(NULL_BAG, NULL_SLOT, dest, KEY_SEARING_GORGE, 1) == EQUIP_ERR_OK)
                if (Item* item = player->StoreNewItem(dest, KEY_SEARING_GORGE, true))
                    player->SendNewItem(item, 1, true, false);
        }
        // A client logs in knowing of no race: it is out of any it was in, or a waiting race
        // would hold it (the marshal offers neither hosting nor joining) until it expired.
        LeaveRaces(player);
        // A kart kept over the logout: its aura came back before the client was in the world.
        if (player->HasAura(SPELL_SUMMON_KART))
            OnMounted(player, SPELL_SUMMON_KART, true);
        PropsLogin(player);
        ReplayLogin(player);
        if (!sSkillLineStore.LookupEntry(SKILL_KARTING))
            return;
        if (!player->HasSkill(SKILL_KARTING))
            player->SetSkill(SKILL_KARTING, 1, SKILL_MAX);
        ChallengeGreeting(player);
        LearnUnlocked(player);
        DeliveryLogin(player);
        SkinLogin(player);
        GoodsLogin(player);
        MarathonLogin(player);
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
    // The state line's comma fields: 6 the racer (a kart), 8 the paint, 9 the trail, 10 the flag.
    constexpr size_t STATE_RACER = 6, STATE_PAINT = 8, STATE_TRAIL = 9, STATE_FLAG = 10;

    // `line` ("KARTCRAFT\tS\t<fields>") with what it shows held to what the driver has (`shown`):
    // a custom kart not theirs is Crash's, and the paint, trail and flag are the ones on file. A
    // CTR racer is the player's own disc's, and passes.
    static std::string HeldToShown(std::string const& line, Shown const& shown)
    {
        size_t const head = line.find("\tS\t");
        if (head == std::string::npos)
            return line;
        std::vector<std::string> fields;
        std::string field;
        std::istringstream body(line.substr(head + 3));
        while (std::getline(body, field, ','))
            fields.push_back(field);
        if (fields.size() > STATE_RACER)
            if (Good const* g = FindGood(fields[STATE_RACER]))
                if (g->kind == 'K' && !shown.karts.count(g->key))
                    fields[STATE_RACER] = "crash";
        if (fields.size() > STATE_TRAIL)
        {
            fields[STATE_PAINT] = std::to_string(shown.paint);
            fields[STATE_TRAIL] = std::to_string(shown.trail);
        }
        if (fields.size() > STATE_FLAG)
            fields[STATE_FLAG] = std::to_string(shown.flag);
        std::string out = line.substr(0, head + 3);
        for (size_t i = 0; i < fields.size(); ++i)
            out += (i ? "," : "") + fields[i];
        return out;
    }

    static void RelayState(Player* player, std::string const& message)
    {
        if (!player->HasAura(SPELL_SUMMON_KART) || message.size() > MAX_STATE_LENGTH)
            return;
        uint32 now = WorldTimer::getMSTime();
        std::string held;
        {
            std::lock_guard<std::mutex> lock(s_stateMutex);
            uint32& last = s_lastState[player->GetGUIDLow()];
            if (last && WorldTimer::getMSTimeDiff(last, now) < STATE_INTERVAL)
                return;
            last = now;
            held = HeldToShown(message, s_shown[player->GetGUIDLow()]);
            s_stateLines[player->GetGUIDLow()] = held;
        }
        WorldPacket data;
        ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, held.c_str(), LANG_ADDON, CHAT_TAG_NONE,
            player->GetObjectGuid(), player->GetName());
        player->SendMessageToSetInRange(&data, STATE_RANGE, false);
    }

    // ── Races ──────────────────────────────────────────────────────────────────────────────────
    //
    // A driver at a course's start hosts a race ("R\tH\t<course>"); drivers near it are told
    // ("R\tO") and join ("R\tJ\t<id>"); the host starts it ("R\tS\t<id>") and every racer gets the
    // course's checkpoints ("R\tC") and a countdown ("R\tG"). Each checkpoint a client reports
    // ("R\tK\t<id>\t<index>") is checked against the course's order and against where the server
    // has seen the racer (Witness); progress ("R\tP") and finishes ("R\tF") go to every racer, and
    // a finish pays experience and Karting skill by place. A client that finds its progress ahead
    // of its "R\tP" sends the missing reports again, so one lost report strands no one. DNF_WAIT
    // after the first finish the race closes, the rest out of time ("R\tDNF\t<id>\t<name>"). The courses are courses/courses.json (KartcraftCourses.h); clients
    // learn a course from "R\tC". A time trial ("R\tT\t<course>", the Race Marshal's) is a race
    // of one, started at once and marked "R\tTT\t<id>": no crates, no items, and its own records.

    // A crate or fruit: 'F' fruit crate, 'Q' weapon crate, 'W' fruit; z at or below
    // INVALID_HEIGHT for the ground's.
    struct CrateSpot { char kind; float x, y, z; };

    struct Course
    {
        char const* key;
        char const* name;
        // The zone it is in, or its series ("Grand Prix"), as the Race Finder groups courses;
        // may be empty.
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
        // Gameobject entries a race on it opens near its start while it runs (doors and gates
        // across the course: Stratholme's locked King's Square gates); may be empty.
        std::vector<uint32> doors;
        // Its start height (WoW z), 0 for none (a dungeon course's from pack.py, a player track's
        // from its maker): the floor a teleport onto its grid looks for from a little over it, so
        // a trip lands on the course's floor, not a roof over it (TeleportOnto).
        float z = 0.f;
        // The kart's and the racers' size on it, 1 their own: a small one makes the world drive
        // bigger round them (Stormwind's streets as a kart's city). The client sizes the kart and
        // its physics; the server sizes each racer's body for the race (TickRaceScales).
        float scale = 1.f;
    };

    // A course's scale's bounds, as the client's (kartcraft MIN_RACE_SCALE, MAX_RACE_SCALE).
    constexpr float MIN_RACE_SCALE = 0.2f;
    constexpr float MAX_RACE_SCALE = 3.f;
    // Each racer the race's scale sizes, and their body's scale before it (raw guid -> scale), to
    // give back as they leave its race (TickRaceScales); a skin put on meanwhile is the new one's.
    static std::unordered_map<uint64, float> s_raceScaleBase;
    static std::mutex s_raceScaleMutex; // over s_raceScaleBase; taken after s_mutex, never before
    // A track editor test lap's scale by racer (raw guid), as its client asks ("R\tSz"): no race
    // of the server's, the body sized all the same while they drive. Under s_mutex.
    static std::unordered_map<uint64, float> s_testLapScale;

    // The courses: courses/courses.json, made into this table by setup/vmangos/pack.py.
#include "KartcraftCourses.h"

    // Whether `c` is a party course: in a dungeon or a raid, not the open world, which in 1.12 is
    // its two continents (MapEntry::IsContinent) and here the battlegrounds' open copies too, one a
    // map with no match in it (kartcraft.patch, MapManager::CreateInstance); the client draws the
    // same line (race.rs, Course::dungeon). Each party, in its own copy of the dungeon, races it
    // alone: hosted inside, joined by the host's party in the host's copy, and never offered by the
    // Race Finder, which would take players into copies of their own.
    static bool PartyCourse(uint32 map)
    {
        MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(map);
        return map > 1 && !(entry && entry->IsBattleGround());
    }

    static bool PartyCourse(Course const& c)
    {
        return PartyCourse(c.map);
    }

    // Whether `player` stands in the open world: a continent, or a battleground's open copy, out of
    // a match.
    static bool InOpenWorld(Player* player)
    {
        Map const* map = player->GetMap();
        return !player->InBattleGround() && (!map->Instanceable() || map->IsBattleGround());
    }

    // A showcase run (/kartshow; the client's kart/showcase.rs): its key, what it is and where it
    // starts (WoW x, y, z and facing). The run itself is the client's own input tape; the server
    // only takes a GM to its start.
    struct ShowRun
    {
        char const* name;
        char const* title;
        char const* note;
        uint32 map;
        float x;
        float y;
        float z;
        float o;
    };

    // The runs: showcase/showcase.json, made into this table by setup/vmangos/pack.py.
#include "KartcraftShowcase.h"

    // How near a checkpoint (yards, across the ground) a racer must pass, how near the start a
    // driver must be to host or join, how long a countdown runs (ms), and how long a race may
    // last before it is dropped (ms).
    constexpr float CHECKPOINT_RADIUS = 45.f;
    constexpr float START_RADIUS = 60.f;
    constexpr uint32 COUNTDOWN = 4000;
    constexpr uint32 RACE_LIFETIME = 20 * MINUTE * IN_MILLISECONDS;
    constexpr size_t MAX_RACERS = 8;
    // How long the others have to finish once the first racer is in (ms), then the race closes
    // with them out of time ("R\tDNF"). Kartcraft's own rule: CTR lets its AI drive every
    // unfinished kart home, which a race of players cannot; a minute, as kart racers commonly
    // give the field after the winner.
    constexpr uint32 DNF_WAIT = 60 * IN_MILLISECONDS;

    struct Racer
    {
        ObjectGuid guid;
        std::string name;
        uint8 lap = 0;
        uint8 checkpoint = 0;
        bool finished = false;
        uint8 place = 0; // finishing place, 1 = first; 0 with `finished`, out of time (DNF)
        uint32 ms = 0;   // its time, with a place
        // The passes the server has seen with its own eyes (Witness), counted as Passes counts:
        // a report that reaches us after the position proving it still counts.
        uint32 seen = 0;
        // Joined from the Race Finder's list (LJ), from anywhere: the host's start takes them to
        // the grid as a pop does, the way back with it (Origin).
        bool fromList = false;
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
        // When the turbo item was last fired (ms, 0 none), for a turbo ram.
        uint32 turboAt = 0;
        // A red beaker's rain cloud: until when (ms, 0 none), its effect, and whether the held
        // item went into the roulette under it, to be rolled anew when the cloud goes.
        uint32 cloudUntil = 0;
        uint8 cloudEffect = 0;
        bool cloudLocked = false;
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

    // A relayed shot: the item, when it was relayed (ms), whether it has hit anyone, whether it
    // was dropped behind (TNT, Nitro, beaker), juiced (a red beaker), where it left from (x, y),
    // and whether it is a launched shield bomb (a Shield's "Z", not the shield put up).
    struct Shot
    {
        uint8 item;
        uint32 at;
        bool hit = false;
        bool dropped = false;
        bool juiced = false;
        float x = 0.f, y = 0.f;
        bool shieldBomb = false;
    };

    struct Race
    {
        Course const* course = nullptr;
        ObjectGuid host;
        uint32 created = 0;
        uint32 start = 0; // 0 until started
        // When a warp orb was last rolled in the race (ms), for WARP_COOLDOWN; 0 none yet.
        uint32 warpRolled = 0;
        uint8 finishers = 0;
        // When the first racer finished (ms), for the rest's DNF_WAIT; 0 none yet.
        uint32 firstFinish = 0;
        // A time trial (the Race Marshal's "R\tT"): one racer, no items, its own records.
        bool timeTrial = false;
        // CTR's ground types ("R\tGT"): every racer's client drives off-road as slow as CTR's
        // grounds while it is on, whatever its own KartTerrain says. The host's choice; a Race
        // Finder race takes FINDER_GROUND_TYPES, and a time trial never has them, so its board
        // stays one board.
        bool groundTypes = false;
        // A party race's instance: a course in a dungeon is raced in one copy of it, the host's,
        // by the host's party there (PartyCourse). 0 in the open world.
        uint32 instance = 0;
        // A time trial from the track browser waits for its racer on the grid: their first kart
        // state there starts the countdown (TickArrival), or ARRIVE_WAIT after awaitSince.
        ObjectGuid awaiting;
        uint32 awaitSince = 0;
        std::vector<Racer> racers;
        // For its results (RecordResult): its id, when it started (unix time) and how many raced,
        // and each racer's place as it came (0 did not finish), for the ladder once all are in.
        uint32 id = 0;
        uint32 startedAt = 0;
        uint8 field = 0;
        std::vector<std::pair<ObjectGuid, std::pair<std::string, uint32>>> placed;
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

    // A racer by guid for a line to their client, in the world or not: a Race Finder launch or a
    // host's start sends the roster, course and countdown just after teleporting everyone, and a
    // far teleport outside a map update runs at once (MapManager::ScheduleFarTeleport), taking
    // the racer out of the world. Their session still takes the line, and the client keeps it
    // through the loading screen.
    static Player* Recipient(ObjectGuid guid)
    {
        return ObjectAccessor::FindPlayerNotInWorld(guid);
    }

    static void SendRace(Race const& race, std::string const& text)
    {
        for (Racer const& r : race.racers)
            if (Player* p = Recipient(r.guid))
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

    // Whether the server has the player within `radius` of `point`: where they stand, or where a
    // near teleport not yet acknowledged puts them. vmangos moves a player only when the map
    // thread handles their MSG_MOVE_TELEPORT_ACK (Player::ExecuteTeleportNear), and an addon line
    // sent just after the ack runs on the AsyncPacket thread, often first.
    static bool Near(Player* player, std::pair<float, float> const& point, float radius)
    {
        if (DistanceTo(player, point) <= radius)
            return true;
        if (!player->IsBeingTeleportedNear())
            return false;
        WorldLocation const& dest = player->GetTeleportDest();
        float dx = dest.x - point.first;
        float dy = dest.y - point.second;
        return std::sqrt(dx * dx + dy * dy) <= radius;
    }

    // A racer's checkpoints passed in order: lap * checkpoints + the last one (a sprint's laps stay
    // 0). Pass k is at checkpoint k % n; a circuit finishes on pass laps * n, a sprint on n - 1.
    static uint32 Passes(Racer const& r, uint32 n)
    {
        return uint32(r.lap) * n + r.checkpoint;
    }

    static uint32 FinalPass(Course const& c)
    {
        uint32 n = uint32(c.points.size());
        return c.laps ? uint32(c.laps) * n : n - 1;
    }

    // One look at where the server has the racer: near the checkpoint after the last it has seen
    // them pass, that one is seen too. A step a look, so only in order; the looks come with every
    // line the racer sends (the kart's state, ~20 a second while driving), so a pass is seen
    // whenever the report of it arrives, and however late.
    static void Witness(Racer& r, Course const& c, Player* player)
    {
        uint32 n = uint32(c.points.size());
        r.seen = std::max(r.seen, Passes(r, n));
        if (r.seen >= FinalPass(c))
            return;
        uint32 next = (r.seen + 1) % n;
        if (Near(player, c.points[next], GateRadius(c, next)))
            ++r.seen;
    }

    // Player tracks ("Player tracks" below): each published one is a course too, "pt-<id>",
    // raced and time-trialled as the official ones are, but never in a Race Finder pick of "any"
    // or a zone, at a Race Marshal or in a challenge. Its course stays in s_userCourses, which
    // never shrinks, for a race still on it once it is taken down or published again; the
    // published ones are s_userTracks', each naming its course by `slot`.
    struct UserTrack
    {
        uint32 id = 0;
        uint32 guid = 0; // the author's low guid
        std::string author, name, zone, grid, points;
        std::string objects; // "key,x,y,z,yaw,pitch,roll[,l,w,h];..." the editor's, or empty
        uint32 map = 0;
        uint32 laps = 0;
        float z = 0.f;   // the track's floor height (Course::z), 0 for none
        float scale = 1.f; // the karts' and racers' size on it (Course::scale)
        size_t slot = 0; // its course in s_userCourses
    };
    struct UserCourse
    {
        std::string key, name;
        Course course{};
    };
    static std::vector<UserTrack> s_userTracks; // oldest first
    static std::deque<UserCourse> s_userCourses;

    static Course const* FindCourse(std::string const& key)
    {
        for (Course const& c : COURSES)
            if (key == c.key)
                return &c;
        for (UserTrack const& t : s_userTracks)
            if (key == s_userCourses[t.slot].course.key)
                return &s_userCourses[t.slot].course;
        return nullptr;
    }

    // Every course by one index, the official ones first and then the player tracks' (taken down
    // or not): what a Race Finder queue's mask counts.
    static size_t CourseCount()
    {
        return COURSES.size() + s_userCourses.size();
    }

    static Course const& CourseAt(size_t i)
    {
        return i < COURSES.size() ? COURSES[i] : s_userCourses[i - COURSES.size()].course;
    }

    // The published player track `c` is, if it is one.
    static UserTrack const* UserTrackOf(Course const& c)
    {
        for (UserTrack const& t : s_userTracks)
            if (&s_userCourses[t.slot].course == &c)
                return &t;
        return nullptr;
    }

    // Whether the course at `i` (CourseAt) may be raced: an official one, or a player track still
    // published.
    static bool CourseLive(size_t i)
    {
        return i < COURSES.size() || UserTrackOf(CourseAt(i));
    }

    // A player track's line, as its list and a race on it name it.
    static std::string UserTrackLine(UserTrack const& t)
    {
        return "KARTCRAFT\tR\tPT\t" + std::to_string(t.id) + "\t" + t.name + "\t" + t.author + "\t" +
            std::to_string(t.map) + "\t" + std::to_string(t.laps) + "\t" + t.zone + "\t" + t.grid + "\t" +
            t.points;
    }

    // A player track's scale as its lines carry it: empty for its own size.
    static std::string ScaleField(float scale)
    {
        char s[16];
        std::snprintf(s, sizeof(s), "%.2f", scale);
        return scale != 1.f ? s : "";
    }

    // A player track's line with every field a list line has (its votes empty), its height, its
    // objects and its scale last, for a race on it: the client builds the track's objects from it.
    static std::string UserTrackRaceLine(UserTrack const& t)
    {
        char z[16];
        std::snprintf(z, sizeof(z), "%.1f", t.z);
        return UserTrackLine(t) + "\t\t\t" + (t.z != 0.f ? z : "") + "\t" + t.objects + "\t" +
            ScaleField(t.scale);
    }

    // `t`'s course, new in s_userCourses: its gates from its spots, its Race Marshal's place its
    // start (it has none; the start is where a racer hosts), no zone (it is in none of the Race
    // Finder's).
    static void MakeUserCourse(UserTrack& t)
    {
        s_userCourses.emplace_back();
        UserCourse& uc = s_userCourses.back();
        uc.key = "pt-" + std::to_string(t.id);
        uc.name = t.name;
        Course& c = uc.course;
        c.key = uc.key.c_str();
        c.name = uc.name.c_str();
        c.zone = "";
        c.map = t.map;
        c.laps = uint8(t.laps);
        for (size_t at = 0, end; at < t.points.size(); at = end + 1)
        {
            end = t.points.find(';', at);
            if (end == std::string::npos)
                end = t.points.size();
            float x = 0.f, y = 0.f;
            if (std::sscanf(t.points.substr(at, end - at).c_str(), "%f,%f", &x, &y) == 2)
                c.points.emplace_back(x, y);
        }
        c.marshal = c.points.empty() ? std::make_pair(0.f, 0.f) : c.points[0];
        c.z = t.z;
        c.scale = t.scale;
        t.slot = s_userCourses.size() - 1;
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
    // with its holder. Races and time trials keep separate boards, kartcraft_records and
    // kartcraft_tt_records in the characters database, each keeping every character's best: the
    // core creates them if missing (the world database may be re-created by vmangos-deploy, the
    // characters database never is), reads them once at startup, and writes a new best in the
    // background (an async execute), so a finish never waits on the database. The record is the
    // least of the bests.

    struct Record
    {
        uint32 ms = 0; // 0: none yet
        std::string name;
        uint32 at = 0; // when it was set, unix time (challenge boards: the first to a tie leads)
    };

    // One table's times: every character's best on each course, and each course's record.
    struct Board
    {
        std::string table;
        std::map<std::pair<uint32, std::string>, Record> bests; // (guid low, course key) -> best
        std::unordered_map<std::string, Record> records;        // course key -> its record
    };

    static Board s_raceBoard = { "kartcraft_records", {}, {} };
    static Board s_trialBoard = { "kartcraft_tt_records", {}, {} };

    static void LoadRecords(Board& board, char const* what)
    {
        CharacterDatabase.DirectExecute((
            "CREATE TABLE IF NOT EXISTS `" + board.table + "` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'course key, courses/courses.json', "
            "`name` VARCHAR(12) NOT NULL COMMENT 'character name when it was set', "
            "`ms` INT UNSIGNED NOT NULL COMMENT '" + what + " time, ms', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when, unix time', "
            "PRIMARY KEY (`guid`, `course`), KEY `course_ms` (`course`, `ms`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8").c_str());
        std::unique_ptr<QueryResult> result =
            CharacterDatabase.Query(("SELECT `guid`, `course`, `name`, `ms` FROM `" + board.table + "`").c_str());
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
            Record best;
            best.ms = ms;
            best.name = fields[2].GetCppString();
            board.bests[{ guid, course }] = best;
            Record& record = board.records[course];
            if (!record.ms || ms < record.ms)
                record = best;
            ++count;
        } while (result->NextRow());
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "Kartcraft: %u best %s times on %u courses.", count, what,
            uint32(board.records.size()));
    }

    // Every racer's result in every race, for the website's leaderboards and the ladder: a row a
    // finish (its place and time), a DNF at the race's close and a retirement (place 0).
    static void CreateResults()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_results` ("
            "`id` INT UNSIGNED NOT NULL AUTO_INCREMENT, "
            "`race_at` INT UNSIGNED NOT NULL COMMENT 'when the race started, unix time', "
            "`race` INT UNSIGNED NOT NULL COMMENT 'the server''s race id, since its start', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'course key, courses/courses.json', "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`name` VARCHAR(12) NOT NULL COMMENT 'character name then', "
            "`place` TINYINT UNSIGNED NOT NULL COMMENT '1 the winner; 0 did not finish', "
            "`field` TINYINT UNSIGNED NOT NULL COMMENT 'racers at the start', "
            "`ms` INT UNSIGNED NOT NULL COMMENT 'race time, ms; 0 did not finish', "
            "`trial` TINYINT UNSIGNED NOT NULL COMMENT '1 a time trial', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when the result came, unix time', "
            "PRIMARY KEY (`id`), KEY `guid` (`guid`), KEY `course` (`course`, `ms`), KEY `at` (`at`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    // The ranked ladder: a rating a character a season, a season a calendar month (UTC, YYYYMM).
    // Every race of two or more racers counts (not a time trial): once the last result is in,
    // each pair of racers is a game won by the one placed ahead, a tie between two who did not
    // finish, and each racer's rating moves by Elo's K * (score - expected) summed over the field
    // and shared by its n - 1 games. Everyone starts a season at LADDER_START.
    constexpr int32 LADDER_START = 1500;
    constexpr double LADDER_K = 32.0;

    struct LadderEntry
    {
        int32 rating = LADDER_START;
        uint32 races = 0;
        uint32 wins = 0;
        std::string name;
    };

    static uint32 s_season = 0;
    static std::unordered_map<uint32, LadderEntry> s_ladder; // guid low -> this season's entry

    static uint32 SeasonNow()
    {
        std::time_t t = std::time(nullptr);
        std::tm g{};
#ifdef _WIN32
        gmtime_s(&g, &t);
#else
        gmtime_r(&t, &g);
#endif
        return uint32((g.tm_year + 1900) * 100 + g.tm_mon + 1);
    }

    static void LoadLadder()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_ladder` ("
            "`season` INT UNSIGNED NOT NULL COMMENT 'YYYYMM, a calendar month in UTC', "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`name` VARCHAR(12) NOT NULL COMMENT 'character name when it last raced', "
            "`rating` INT NOT NULL, "
            "`races` INT UNSIGNED NOT NULL, "
            "`wins` INT UNSIGNED NOT NULL, "
            "`at` INT UNSIGNED NOT NULL COMMENT 'last rated race, unix time', "
            "PRIMARY KEY (`season`, `guid`), KEY `season_rating` (`season`, `rating`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        s_season = SeasonNow();
        s_ladder.clear();
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `guid`, `name`, `rating`, `races`, `wins` FROM `kartcraft_ladder` WHERE `season` = %u", s_season);
        if (!result)
            return;
        do
        {
            Field* f = result->Fetch();
            LadderEntry& e = s_ladder[f[0].GetUInt32()];
            e.name = f[1].GetCppString();
            e.rating = f[2].GetInt32();
            e.races = f[3].GetUInt32();
            e.wins = f[4].GetUInt32();
        } while (result->NextRow());
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "Kartcraft: %u racers on season %u's ladder.",
            uint32(s_ladder.size()), s_season);
    }

    // A rating's place this season, 1 the best.
    static uint32 LadderPlace(int32 rating)
    {
        uint32 above = 0;
        for (auto const& e : s_ladder)
            if (e.second.rating > rating)
                ++above;
        return above + 1;
    }

    static void ReplayKeepBest(Race const& r, char const* reason, uint32 guid, std::string const& course,
        uint32 holder, int32 value);

    // A race's results all in: the ratings move, are kept, and each racer is told.
    static void RateRace(Race const& r)
    {
        size_t n = r.placed.size();
        if (r.timeTrial || n < 2)
            return;
        if (SeasonNow() != s_season)
        {
            s_season = SeasonNow();
            s_ladder.clear();
        }
        // A finisher's rank is their place; everyone who did not finish shares the last.
        auto rank = [n](uint32 place) { return place ? place : uint32(n + 1); };
        std::vector<int32> before(n);
        for (size_t i = 0; i < n; ++i)
            before[i] = s_ladder[r.placed[i].first.GetCounter()].rating;
        std::vector<double> delta(n, 0.0);
        for (size_t i = 0; i < n; ++i)
            for (size_t j = 0; j < n; ++j)
            {
                if (i == j)
                    continue;
                uint32 a = rank(r.placed[i].second.second);
                uint32 b = rank(r.placed[j].second.second);
                double score = a < b ? 1.0 : a == b ? 0.5 : 0.0;
                double expected = 1.0 / (1.0 + std::pow(10.0, (before[j] - before[i]) / 400.0));
                delta[i] += LADDER_K * (score - expected) / double(n - 1);
            }
        uint32 now = uint32(std::time(nullptr));
        for (size_t i = 0; i < n; ++i)
        {
            uint32 low = r.placed[i].first.GetCounter();
            LadderEntry& e = s_ladder[low];
            e.rating = before[i] + int32(std::lround(delta[i]));
            e.races += 1;
            if (r.placed[i].second.second == 1)
                e.wins += 1;
            e.name = r.placed[i].second.first;
            std::string escaped = e.name;
            CharacterDatabase.escape_string(escaped);
            CharacterDatabase.PExecute("REPLACE INTO `kartcraft_ladder` (`season`, `guid`, `name`, `rating`, "
                "`races`, `wins`, `at`) VALUES (%u, %u, '%s', %i, %u, %u, %u)", s_season, low, escaped.c_str(),
                e.rating, e.races, e.wins, now);
            // The racer's best race of the season, the one they gained the most rating in, keeps
            // its take (Replays); past the season only its top three's stay.
            ReplayKeepBest(r, "lb", low, std::to_string(s_season), low, e.rating - before[i]);
        }
        for (size_t i = 0; i < n; ++i)
        {
            Player* p = ObjectAccessor::FindPlayer(r.placed[i].first);
            if (!p)
                continue;
            LadderEntry const& e = s_ladder[r.placed[i].first.GetCounter()];
            int32 change = e.rating - before[i];
            char text[160];
            std::snprintf(text, sizeof(text), "Ladder: %i -> %i (%s%i), #%u this season.", before[i], e.rating,
                change >= 0 ? "+" : "", change, LadderPlace(e.rating));
            ChatHandler(p->GetSession()).SendSysMessage(text);
        }
    }

    // The challenges: a course a day (UTC) and a course a week (Monday to Sunday, UTC), picked by
    // FNV-1a over the period's name ("d20261014", "w2911": weeks since the Monday before
    // 1970-01-01) from the open-world courses in courses.json's order, so the website picks the
    // same. A time trial on the day's or the week's course counts for its board: each racer's
    // best in the period, kartcraft_challenge.
    struct ChallengeBoard
    {
        std::string period;
        std::string course;
        std::map<uint32, Record> bests; // guid low -> best
    };

    static std::string Clock(uint32 ms);

    static ChallengeBoard s_daily;
    static ChallengeBoard s_weekly;

    static uint32 Fnv1a(std::string const& text)
    {
        uint32 h = 2166136261u;
        for (unsigned char c : text)
        {
            h ^= c;
            h *= 16777619u;
        }
        return h;
    }

    // Tracks the owner has taken out (kartcraft_course_off, written by the website's admin page):
    // the Race Finder never picks one, the automatic challenge never lands on one, and hosting or a
    // time trial on one is refused; the client hides them from its track browser ("R\tOff").
    // Looked at again every COURSE_OFF_RECHECK_MS.
    static uint32 const COURSE_OFF_RECHECK_MS = 30 * IN_MILLISECONDS;
    static std::set<std::string> s_courseOff;
    static uint32 s_courseOffChecked = 0;

    static void CourseOffNow()
    {
        uint32 now = WorldTimer::getMSTime();
        if (s_courseOffChecked && WorldTimer::getMSTimeDiff(s_courseOffChecked, now) < COURSE_OFF_RECHECK_MS)
            return;
        s_courseOffChecked = now ? now : 1;
        s_courseOff.clear();
        std::unique_ptr<QueryResult> result = CharacterDatabase.Query("SELECT `course` FROM `kartcraft_course_off`");
        if (!result)
            return;
        do
            s_courseOff.insert(result->Fetch()[0].GetCppString());
        while (result->NextRow());
    }

    static bool CourseOff(Course const& c)
    {
        CourseOffNow();
        return s_courseOff.count(c.key) != 0;
    }

    // "KARTCRAFT\tR\tOff\t<key,key,...>": the tracks taken out, for the track browser.
    static void SendCourseOff(Player* player)
    {
        CourseOffNow();
        std::string keys;
        for (std::string const& k : s_courseOff)
            keys += (keys.empty() ? "" : ",") + k;
        SendLine(player, "KARTCRAFT\tR\tOff\t" + keys);
    }

    static bool RefuseOff(Player* player, Course const& c)
    {
        if (!CourseOff(c))
            return false;
        ChatHandler(player->GetSession()).SendSysMessage("That track is closed for now.");
        return true;
    }

    // The course the owner picked for `period` (kartcraft_challenge_pick, written by the website's
    // admin page), if it is an open-world course; else empty.
    static std::string ChallengePick(std::string const& period)
    {
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `course` FROM `kartcraft_challenge_pick` WHERE `period` = '%s'", period.c_str());
        if (!result)
            return std::string();
        std::string key = result->Fetch()[0].GetCppString();
        Course const* c = FindCourse(key);
        return c && !PartyCourse(*c) ? key : std::string();
    }

    static std::string ChallengeCourse(std::string const& period)
    {
        std::string picked = ChallengePick(period);
        if (!picked.empty())
            return picked;
        // The course this server chose for the period before keeps it to the period's end,
        // whatever the course list has become since (a redeploy with tracks added or cut).
        std::unique_ptr<QueryResult> chosen = CharacterDatabase.PQuery(
            "SELECT `course` FROM `kartcraft_challenge_course` WHERE `period` = '%s' AND `auto` = 1", period.c_str());
        if (chosen)
        {
            Course const* c = FindCourse(chosen->Fetch()[0].GetCppString());
            if (c && !PartyCourse(*c) && !CourseOff(*c))
                return c->key;
        }
        std::vector<Course const*> open;
        for (Course const& c : COURSES)
            if (!PartyCourse(c) && !CourseOff(c))
                open.push_back(&c);
        return open.empty() ? std::string() : open[Fnv1a(period) % open.size()]->key;
    }

    static std::pair<std::string, std::string> ChallengePeriods()
    {
        std::time_t t = std::time(nullptr);
        std::tm g{};
#ifdef _WIN32
        gmtime_s(&g, &t);
#else
        gmtime_r(&t, &g);
#endif
        char day[16];
        std::snprintf(day, sizeof(day), "d%04d%02d%02d", g.tm_year + 1900, g.tm_mon + 1, g.tm_mday);
        // 1970-01-01 was a Thursday: three days after the week's Monday.
        long long days = (long long)(t / 86400);
        return { day, "w" + std::to_string((days + 3) / 7) };
    }

    // The board for `period`, loaded on its first use and again when the period's course is
    // changed under it (the owner's pick, looked at every CHALLENGE_RECHECK_MS): its times are
    // the ones set on that course.
    static uint32 const CHALLENGE_RECHECK_MS = 30 * IN_MILLISECONDS;
    static uint32 s_challengeChecked = 0;

    static void ChallengeTo(ChallengeBoard& board, std::string const& period, bool recheck)
    {
        if (board.period == period && !recheck)
            return;
        std::string course = ChallengeCourse(period);
        if (board.period == period && board.course == course)
            return;
        board.period = period;
        board.course = course;
        board.bests.clear();
        std::string escaped = course;
        CharacterDatabase.escape_string(escaped);
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_challenge_course` (`period`, `course`, `auto`, `at`) "
            "VALUES ('%s', '%s', %u, %u)", period.c_str(), escaped.c_str(), ChallengePick(period).empty() ? 1u : 0u,
            uint32(std::time(nullptr)));
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `guid`, `name`, `ms`, `at` FROM `kartcraft_challenge` WHERE `period` = '%s' AND `course` = '%s'",
            period.c_str(), escaped.c_str());
        if (!result)
            return;
        do
        {
            Field* f = result->Fetch();
            Record& r = board.bests[f[0].GetUInt32()];
            r.name = f[1].GetCppString();
            r.ms = f[2].GetUInt32();
            r.at = f[3].GetUInt32();
        } while (result->NextRow());
    }

    static void ChallengesNow()
    {
        std::pair<std::string, std::string> periods = ChallengePeriods();
        uint32 now = WorldTimer::getMSTime();
        bool recheck = !s_challengeChecked || WorldTimer::getMSTimeDiff(s_challengeChecked, now) >= CHALLENGE_RECHECK_MS;
        if (recheck)
            s_challengeChecked = now ? now : 1;
        ChallengeTo(s_daily, periods.first, recheck);
        ChallengeTo(s_weekly, periods.second, recheck);
    }

    static void CreateChallenges()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_challenge_pick` ("
            "`period` VARCHAR(12) NOT NULL COMMENT 'd<YYYYMMDD> a day, w<weeks since 1969-12-29> a week, UTC', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'the course the owner picked for it', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when picked, unix time', "
            "PRIMARY KEY (`period`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_challenge` ("
            "`period` VARCHAR(12) NOT NULL COMMENT 'd<YYYYMMDD> a day, w<weeks since 1969-12-29> a week, UTC', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'the period''s course key', "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`name` VARCHAR(12) NOT NULL, "
            "`ms` INT UNSIGNED NOT NULL COMMENT 'best time trial in the period, ms', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when, unix time', "
            "PRIMARY KEY (`period`, `guid`), KEY `period_ms` (`period`, `ms`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        // The course each period's challenge is on, as this server chose it (the owner's pick, or
        // its own among the open courses), for the website to show the same board.
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_challenge_course` ("
            "`period` VARCHAR(12) NOT NULL COMMENT 'd<YYYYMMDD> a day, w<weeks since 1969-12-29> a week, UTC', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'the course the period is on now', "
            "`auto` TINYINT UNSIGNED NOT NULL COMMENT '1 this server''s own choice, 0 the owner''s pick', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when chosen, unix time', "
            "PRIMARY KEY (`period`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    static Record const* ChallengeLeader(ChallengeBoard const& board)
    {
        // The fastest; on a tie, who set it first, as the record announcement had it.
        Record const* best = nullptr;
        for (auto const& e : board.bests)
            if (!best || e.second.ms < best->ms || (e.second.ms == best->ms && e.second.at < best->at))
                best = &e.second;
        return best;
    }

    // A new leader on a challenge's board, told to every player in the world: a system line to
    // each session, whatever its map, zone, instance or faction (World::SendGlobalMessage).
    static void AnnounceChallengeRecord(bool daily, std::string const& name, std::string const& course,
        uint32 ms, std::string const& was)
    {
        Course const* c = FindCourse(course);
        std::string text = std::string("|cffffd100") + (daily ? "Daily" : "Weekly") + " challenge record!|r " +
            name + ": " + (c ? c->name : course) + " in " + Clock(ms) + was + ".";
        WorldPacket data;
        ChatHandler::BuildChatPacket(data, CHAT_MSG_SYSTEM, text.c_str());
        sWorld.SendGlobalMessage(&data);
    }

    // A time trial's finish on the day's or the week's course: a better time than the racer's
    // in the period goes on its board.
    static void ReplayKeep(Race const& r, char const* reason, uint32 guid, std::string const& course, uint32 holder);

    static void ChallengeFinish(uint32 guid, std::string const& name, std::string const& course, uint32 ms,
        Race const* race = nullptr)
    {
        ChallengesNow();
        for (ChallengeBoard* board : { &s_daily, &s_weekly })
        {
            if (board->course != course || !ms)
                continue;
            // The board's leader before this time, to tell a new record.
            Record const* leader = ChallengeLeader(*board);
            bool record = !leader || ms < leader->ms;
            std::string was = leader ? " (was " + Clock(leader->ms) + " by " + leader->name + ")" : "";
            Record& best = board->bests[guid];
            if (best.ms && best.ms <= ms)
                continue;
            best.ms = ms;
            best.name = name;
            best.at = uint32(std::time(nullptr));
            std::string escaped = name;
            CharacterDatabase.escape_string(escaped);
            CharacterDatabase.PExecute("REPLACE INTO `kartcraft_challenge` (`period`, `course`, `guid`, `name`, "
                "`ms`, `at`) VALUES ('%s', '%s', %u, '%s', %u, %u)", board->period.c_str(), course.c_str(), guid,
                escaped.c_str(), ms, best.at);
            if (race)
                ReplayKeep(*race, board == &s_daily ? "cd" : "cw", guid, board->period, guid);
            if (record)
                AnnounceChallengeRecord(board == &s_daily, name, course, ms, was);
        }
    }

    // Cosmetics: a kart paint and a boost trail, each earned by what a racer has done, the
    // server's tables the measure: races run, races won (of two or more), course records held
    // (races and time trials), challenge times set, and finished ladder seasons ended in the top
    // three or on top. The catalogues are here, sent to the client with each racer's numbers
    // ("CC"); the client draws a paint or trail by its index. What a racer wears is
    // kartcraft_cosmetic; an equip the numbers do not earn is refused.
    enum CosmeticStat : uint8
    {
        STAT_RACES,
        STAT_WINS,
        STAT_RECORDS,
        STAT_CHALLENGES,
        STAT_PODIUMS,
        STAT_CHAMPIONS,
        // Found in an Astral Cache (Caches): owned one by one in kartcraft_found, not counted.
        STAT_FOUND,
        STAT_COUNT
    };

    struct Cosmetic
    {
        char const* name;
        uint8 stat;
        uint32 need; // 0: everyone's
    };

    static std::vector<Cosmetic> const PAINTS = {
        { "Stock", STAT_RACES, 0 },
        { "Copper", STAT_RACES, 10 },
        { "Silver", STAT_WINS, 5 },
        { "Gold", STAT_WINS, 25 },
        { "Midnight", STAT_RACES, 100 },
        { "Record Red", STAT_RECORDS, 1 },
        { "Challenger Teal", STAT_CHALLENGES, 1 },
        { "Champion Violet", STAT_PODIUMS, 1 },
        { "Astral", STAT_FOUND, 1 },
    };

    static std::vector<Cosmetic> const TRAILS = {
        { "None", STAT_RACES, 0 },
        { "Sparks", STAT_RACES, 25 },
        { "Wumpa", STAT_WINS, 10 },
        { "Frost", STAT_RECORDS, 3 },
        { "Rainbow", STAT_CHAMPIONS, 1 },
        { "Stardust", STAT_FOUND, 1 },
        { "Comet", STAT_FOUND, 1 },
    };

    static void CreateCosmetics()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_cosmetic` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`paint` TINYINT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'index into PAINTS', "
            "`trail` TINYINT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'index into TRAILS', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when, unix time', "
            "PRIMARY KEY (`guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_found` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`kind` CHAR(1) NOT NULL COMMENT 'P a paint, T a trail', "
            "`idx` TINYINT UNSIGNED NOT NULL COMMENT 'index into PAINTS or TRAILS', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when found, unix time', "
            "PRIMARY KEY (`guid`, `kind`, `idx`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    // The cosmetics a character has found in caches: "P<index>" and "T<index>".
    static std::set<std::string> Found(uint32 guid)
    {
        std::set<std::string> out;
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `kind`, `idx` FROM `kartcraft_found` WHERE `guid` = %u", guid);
        if (result)
            do
            {
                Field* f = result->Fetch();
                out.insert(f[0].GetCppString() + std::to_string(f[1].GetUInt32()));
            } while (result->NextRow());
        return out;
    }

    static uint32 CountOf(char const* sql)
    {
        std::unique_ptr<QueryResult> result = CharacterDatabase.Query(sql);
        return result ? result->Fetch()[0].GetUInt32() : 0;
    }

    // A racer's numbers, STAT_COUNT of them, read now.
    static std::vector<uint32> CosmeticStats(uint32 guid)
    {
        std::vector<uint32> stats(STAT_COUNT, 0);
        char sql[640];
        std::snprintf(sql, sizeof(sql),
            "SELECT COUNT(*) FROM `kartcraft_results` WHERE `guid` = %u AND `trial` = 0", guid);
        stats[STAT_RACES] = CountOf(sql);
        std::snprintf(sql, sizeof(sql),
            "SELECT COUNT(*) FROM `kartcraft_results` WHERE `guid` = %u AND `trial` = 0 AND `place` = 1 "
            "AND `field` >= 2", guid);
        stats[STAT_WINS] = CountOf(sql);
        for (char const* table : { "kartcraft_records", "kartcraft_tt_records" })
        {
            std::snprintf(sql, sizeof(sql),
                "SELECT COUNT(*) FROM `%s` r WHERE r.`guid` = %u AND r.`ms` > 0 AND r.`ms` = "
                "(SELECT MIN(o.`ms`) FROM `%s` o WHERE o.`course` = r.`course` AND o.`ms` > 0)",
                table, guid, table);
            stats[STAT_RECORDS] += CountOf(sql);
        }
        std::snprintf(sql, sizeof(sql),
            "SELECT COUNT(*) FROM `kartcraft_challenge` WHERE `guid` = %u", guid);
        stats[STAT_CHALLENGES] = CountOf(sql);
        uint32 season = SeasonNow();
        for (uint8 stat : { STAT_PODIUMS, STAT_CHAMPIONS })
        {
            std::snprintf(sql, sizeof(sql),
                "SELECT COUNT(*) FROM `kartcraft_ladder` l WHERE l.`guid` = %u AND l.`season` < %u AND "
                "(SELECT COUNT(*) FROM `kartcraft_ladder` o WHERE o.`season` = l.`season` AND o.`rating` > l.`rating`) < %u",
                guid, season, stat == STAT_PODIUMS ? 3u : 1u);
            stats[stat] = CountOf(sql);
        }
        return stats;
    }

    static bool Earned(std::vector<Cosmetic> const& list, uint32 index, std::vector<uint32> const& stats,
        std::set<std::string> const& found, char kind)
    {
        if (index >= list.size())
            return false;
        if (list[index].stat == STAT_FOUND)
            return found.count(kind + std::to_string(index)) != 0;
        return stats[list[index].stat] >= list[index].need;
    }

    // What a racer wears: (paint, trail), 0 for none on file.
    static std::pair<uint32, uint32> Worn(uint32 guid)
    {
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `paint`, `trail` FROM `kartcraft_cosmetic` WHERE `guid` = %u", guid);
        if (!result)
            return { 0, 0 };
        Field* f = result->Fetch();
        return { f[0].GetUInt32(), f[1].GetUInt32() };
    }

    // A found one is everyone's (need 0) to its finder, and "???" to the rest until they find it.
    static std::string Catalogue(std::vector<Cosmetic> const& list, std::set<std::string> const& found, char kind)
    {
        std::string text;
        for (size_t i = 0; i < list.size(); ++i)
        {
            Cosmetic const& c = list[i];
            if (!text.empty())
                text += ";";
            if (c.stat == STAT_FOUND)
            {
                bool const mine = found.count(kind + std::to_string(i)) != 0;
                text += std::string(mine ? c.name : "???") + ":" + std::to_string(c.stat) + ":" + (mine ? "0" : "1");
                continue;
            }
            text += std::string(c.name) + ":" + std::to_string(c.stat) + ":" + std::to_string(c.need);
        }
        return text;
    }

    // A cosmetic found in an Astral Cache: one of the hidden ones `player` lacks, if any; its
    // name, or "" none.
    static std::string FindHidden(Player* player)
    {
        uint32 const guid = player->GetGUIDLow();
        std::set<std::string> const found = Found(guid);
        std::vector<std::pair<char, uint32>> left;
        for (uint32 i = 0; i < PAINTS.size(); ++i)
            if (PAINTS[i].stat == STAT_FOUND && !found.count("P" + std::to_string(i)))
                left.emplace_back('P', i);
        for (uint32 i = 0; i < TRAILS.size(); ++i)
            if (TRAILS[i].stat == STAT_FOUND && !found.count("T" + std::to_string(i)))
                left.emplace_back('T', i);
        if (left.empty())
            return "";
        std::pair<char, uint32> const pick = left[urand(0, uint32(left.size()) - 1)];
        CharacterDatabase.DirectPExecute("INSERT IGNORE INTO `kartcraft_found` (`guid`, `kind`, `idx`, `at`) "
            "VALUES (%u, '%c', %u, %u)", guid, pick.first, pick.second, uint32(std::time(nullptr)));
        return std::string(pick.first == 'P' ? PAINTS[pick.second].name : TRAILS[pick.second].name) +
            (pick.first == 'P' ? " paint" : " trail");
    }

    // "KARTCRAFT\tR\tCC\tP\t<name:stat:need;...>", the same for "T" (the trails), then
    // "KARTCRAFT\tR\tCO\t<races,wins,records,challenges,podiums,champions>\t<paint>\t<trail>": the
    // garage's Paint and Trail rows, and what we wear. A paint or trail no longer earned (a record
    // lost) is worn no more.
    static void SendCosmetics(Player* player)
    {
        uint32 guid = player->GetGUIDLow();
        std::vector<uint32> stats = CosmeticStats(guid);
        std::set<std::string> const found = Found(guid);
        std::pair<uint32, uint32> worn = Worn(guid);
        if (!Earned(PAINTS, worn.first, stats, found, 'P'))
            worn.first = 0;
        if (!Earned(TRAILS, worn.second, stats, found, 'T'))
            worn.second = 0;
        {
            std::lock_guard<std::mutex> lock(s_stateMutex);
            Shown& shown = s_shown[guid];
            shown.paint = worn.first;
            shown.trail = worn.second;
        }
        SendLine(player, "KARTCRAFT\tR\tCC\tP\t" + Catalogue(PAINTS, found, 'P'));
        SendLine(player, "KARTCRAFT\tR\tCC\tT\t" + Catalogue(TRAILS, found, 'T'));
        std::string numbers;
        for (uint32 v : stats)
            numbers += (numbers.empty() ? "" : ",") + std::to_string(v);
        SendLine(player, "KARTCRAFT\tR\tCO\t" + numbers + "\t" + std::to_string(worn.first) + "\t" +
            std::to_string(worn.second));
    }

    // "R\tCE\tP|T\t<index>": wear a paint or a trail, if earned; the answer is "CO" either way.
    static void EquipCosmetic(Player* player, std::string const& kind, std::string const& index)
    {
        uint32 guid = player->GetGUIDLow();
        uint32 i = uint32(std::strtoul(index.c_str(), nullptr, 10));
        std::vector<uint32> stats = CosmeticStats(guid);
        std::pair<uint32, uint32> worn = Worn(guid);
        bool paint = kind == "P";
        if (Earned(paint ? PAINTS : TRAILS, i, stats, Found(guid), paint ? 'P' : 'T'))
        {
            (paint ? worn.first : worn.second) = i;
            CharacterDatabase.PExecute("REPLACE INTO `kartcraft_cosmetic` (`guid`, `paint`, `trail`, `at`) "
                "VALUES (%u, %u, %u, %u)", guid, worn.first, worn.second, uint32(std::time(nullptr)));
        }
        else
            ChatHandler(player->GetSession()).SendSysMessage("Not earned yet.");
        SendCosmetics(player);
    }

    // "KARTCRAFT\tR\tCH\t<day's course>\t<week's course>\t<your day best>\t<day leader>\t<their ms>
    // \t<your week best>\t<week leader>\t<their ms>": the menu's two entries and their boards.
    static void SendChallenges(Player* player)
    {
        ChallengesNow();
        std::string text = "KARTCRAFT\tR\tCH\t" + s_daily.course + "\t" + s_weekly.course;
        for (ChallengeBoard const* board : { &s_daily, &s_weekly })
        {
            auto mine = board->bests.find(player->GetGUIDLow());
            Record const* leader = ChallengeLeader(*board);
            text += "\t" + std::to_string(mine == board->bests.end() ? 0 : mine->second.ms) + "\t" +
                (leader ? leader->name : std::string()) + "\t" + std::to_string(leader ? leader->ms : 0);
        }
        SendLine(player, text);
    }

    // The login's line about the challenges: the day's and the week's course, who leads, your best.
    static void ChallengeGreeting(Player* player)
    {
        ChallengesNow();
        auto line = [player](char const* which, ChallengeBoard const& board) {
            Course const* c = FindCourse(board.course);
            if (!c)
                return;
            Record const* leader = ChallengeLeader(board);
            auto mine = board.bests.find(player->GetGUIDLow());
            std::string text = std::string(which) + " challenge: " + c->name + " (time trial). ";
            text += leader ? "Leader: " + leader->name + " " + Clock(leader->ms) + "." : "Nobody has set a time yet.";
            if (mine != board.bests.end())
                text += " Your best: " + Clock(mine->second.ms) + ".";
            ChatHandler(player->GetSession()).SendSysMessage(text.c_str());
        };
        line("Today's", s_daily);
        line("This week's", s_weekly);
    }

    // Track ratings: after a race, a racer who covered at least half of it and has not rated the
    // track yet is asked ("R\tRateAsk\t<key>"); the answer ("R\tRate\t<key>\t1|-1\t<reason>") is
    // kept, one vote a racer a track, changed by rating again (the track browser). Only a racer
    // with a result on the track may rate it. With the Unstucks used in races
    // (kartcraft_unstuck) and the results' quits, the website's admin page ranks the tracks.
    static char const* const RATE_REASONS[] = { "stuck", "route", "long", "bumpy", "boring" };

    static void CreateRatings()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_rating` ("
            "`course` VARCHAR(32) NOT NULL, "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`vote` TINYINT NOT NULL COMMENT '1 good, -1 bad', "
            "`reason` VARCHAR(16) NOT NULL DEFAULT '' COMMENT 'a bad vote''s: stuck, route, long, bumpy, boring', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when, unix time', "
            "PRIMARY KEY (`course`, `guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_unstuck` ("
            "`id` INT UNSIGNED NOT NULL AUTO_INCREMENT, "
            "`course` VARCHAR(32) NOT NULL, "
            "`guid` INT UNSIGNED NOT NULL, "
            "`race_at` INT UNSIGNED NOT NULL COMMENT 'the race''s start, unix time', "
            "`at` INT UNSIGNED NOT NULL, "
            "PRIMARY KEY (`id`), KEY `course` (`course`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_course_off` ("
            "`course` VARCHAR(32) NOT NULL, "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when taken out, unix time', "
            "PRIMARY KEY (`course`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    // ── Courier deliveries ────────────────────────────────────────────────────────────────────
    //
    // Repeatable quests between the neutral towns' people (deliveries/deliveries.json, made into
    // kartcraft_deliveries.sql and KartcraftDeliveries.h by pack.py): the giver
    // hands over the Courier's Package, its recipient takes it and pays DELIVERY_PAY tokens (the
    // quest's own reward), and the server adds DELIVERY_BONUS more when it came within the
    // route's par. DELIVERIES_A_DAY of the routes are offered each UTC day, the daily
    // challenge's (ChallengePeriods) and each faction's own, each once a day a character; one
    // package at a time. A
    // package goes by kart: while it is carried no Kartcraft teleport, flight path, hearthstone
    // or teleport spell takes its carrier (CarryingDelivery).
    struct Delivery
    {
        uint32 quest;
        char const* from;  // the towns, for the client
        char const* to;
        uint32 giver;      // their couriers
        uint32 recipient;
        uint32 par;        // seconds
        uint8 team;        // whose: 0 either faction's, 1 the Alliance's, 2 the Horde's
        char const* giverName;
        char const* recipientName;
        uint32 map;
        float gx, gy, gz, go; // where the giver stands and faces (Courier Jobs' take)
        float rx, ry;         // where the recipient stands
        uint32 distance;      // yards, the straight line
        char const* cargo;    // what Courier Jobs says the crate holds
    };
    // A Marathon Delivery's stop: a town no faction holds, at its courier's place.
    struct MarathonTown
    {
        char const* name;
        uint32 map;
        float x, y, z, o;
    };
#include "KartcraftDeliveries.h"
    constexpr size_t DELIVERIES_A_DAY = 10;
    constexpr uint32 DELIVERY_TOKEN = 190100;
    constexpr uint32 DELIVERY_PACKAGE = 190101;
    constexpr uint32 DELIVERY_BONUS = 5;

    static std::mutex s_deliveryMutex;
    // guid low -> the day, and the routes delivered that day (kartcraft_deliveries).
    static std::unordered_map<uint32, std::pair<std::string, std::set<uint32>>> s_delivered;
    // guid low -> the route under way and when it was taken (unix time).
    static std::unordered_map<uint32, std::pair<uint32, std::time_t>> s_deliveryStart;

    static void CreateDeliveries()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_deliveries` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`quest` MEDIUMINT UNSIGNED NOT NULL COMMENT 'the route''s quest', "
            "`day` VARCHAR(12) NOT NULL COMMENT 'd<YYYYMMDD>, UTC', "
            "`seconds` INT UNSIGNED NOT NULL COMMENT 'taken to delivered; 0 unknown', "
            "`bonus` TINYINT UNSIGNED NOT NULL COMMENT '1 within par', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when delivered, unix time', "
            "PRIMARY KEY (`guid`, `quest`, `day`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    // The routes by quest, and the span their quests take: asked for every quest a quest giver
    // has whenever the "!" and "?" over NPCs are worked out, most of them no route at all.
    struct DeliveryIndex
    {
        std::unordered_map<uint32, Delivery const*> byQuest;
        uint32 low = 0xFFFFFFFFu;
        uint32 high = 0;
    };

    static DeliveryIndex const& Deliveries()
    {
        static DeliveryIndex const index = [] {
            DeliveryIndex built;
            for (Delivery const& d : DELIVERIES)
            {
                built.byQuest[d.quest] = &d;
                built.low = std::min(built.low, d.quest);
                built.high = std::max(built.high, d.quest);
            }
            return built;
        }();
        return index;
    }

    static Delivery const* FindDelivery(uint32 quest)
    {
        DeliveryIndex const& index = Deliveries();
        if (quest < index.low || quest > index.high)
            return nullptr;
        auto it = index.byQuest.find(quest);
        return it == index.byQuest.end() ? nullptr : it->second;
    }

    // murmur3's finalizer: FNV-1a alone moves little for a change in the last character, and a
    // town's routes' quests are neighbours, so its routes ranked together and came in bunches.
    static uint32 Mix32(uint32 h)
    {
        h ^= h >> 16;
        h *= 0x85ebca6bu;
        h ^= h >> 13;
        h *= 0xc2b2ae35u;
        h ^= h >> 16;
        return h;
    }

    // Today's routes for a faction (`team` 1 the Alliance, 2 the Horde): the DELIVERIES_A_DAY of
    // its own and the neutral ones whose hash with the day is lowest, in that order.
    static std::vector<Delivery const*> RankToday(std::string const& day, uint8 team)
    {
        std::vector<std::pair<uint32, Delivery const*>> ranked;
        for (Delivery const& d : DELIVERIES)
            if (!d.team || d.team == team)
                ranked.push_back({ Mix32(Fnv1a(day + "/" + std::to_string(d.quest))), &d });
        std::sort(ranked.begin(), ranked.end(),
            [](auto const& a, auto const& b) { return a.first < b.first; });
        std::vector<Delivery const*> today;
        for (size_t i = 0; i < ranked.size() && i < DELIVERIES_A_DAY; ++i)
            today.push_back(ranked[i].second);
        return today;
    }

    // RankToday, worked out once a day a faction: asked from every map thread whenever quest
    // markers are, it built ~750 strings and sorted them each time.
    struct TodaysRoutes
    {
        std::string day;
        std::vector<Delivery const*> list;
        std::unordered_set<uint32> quests;
    };
    static std::mutex s_todayMutex;
    static TodaysRoutes s_today[3];

    static TodaysRoutes const& TodayFor(std::string const& day, uint8 team)
    {
        TodaysRoutes& t = s_today[team < 3 ? team : 0];
        if (t.day != day)
        {
            t.list = RankToday(day, team);
            t.quests.clear();
            for (Delivery const* d : t.list)
                t.quests.insert(d->quest);
            t.day = day;
        }
        return t;
    }

    static std::vector<Delivery const*> Today(std::string const& day, uint8 team)
    {
        std::lock_guard<std::mutex> lock(s_todayMutex);
        return TodayFor(day, team).list;
    }

    static bool OfferedOn(std::string const& day, uint32 quest, uint8 team)
    {
        std::lock_guard<std::mutex> lock(s_todayMutex);
        return TodayFor(day, team).quests.count(quest) != 0;
    }

    static uint8 TeamOf(Player const* player)
    {
        return player->GetTeam() == ALLIANCE ? 1 : 2;
    }

    // Under s_deliveryMutex: the routes `guid` delivered on `day`, from the table at first.
    static std::set<uint32>& DeliveredOn(uint32 guid, std::string const& day)
    {
        auto& entry = s_delivered[guid];
        if (entry.first != day)
        {
            entry.first = day;
            entry.second.clear();
            std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
                "SELECT `quest` FROM `kartcraft_deliveries` WHERE `guid` = %u AND `day` = '%s'",
                guid, day.c_str());
            if (result)
                do
                    entry.second.insert(result->Fetch()[0].GetUInt32());
                while (result->NextRow());
        }
        return entry.second;
    }

    // In the bank too: a package banked for the trip and taken out at the far end is still carried.
    bool CarryingDelivery(Player const* player)
    {
        return player->HasItemCount(DELIVERY_PACKAGE, 1, true);
    }

    static std::string DeliveryClock(uint32 seconds)
    {
        char text[16];
        std::snprintf(text, sizeof(text), "%u:%02u", seconds / 60, seconds % 60);
        return text;
    }

    // The client's delivery timer: the town, the par and the seconds already gone.
    static void SendDeliveryTimer(Player* player, Delivery const& d, uint32 gone)
    {
        SendLine(player, std::string("KARTCRAFT\tR\tDel\t") + d.to + "\t" + std::to_string(d.par) + "\t" +
            std::to_string(gone));
    }

    bool DeliveryOffered(Player const* player, Quest const* quest, bool msg)
    {
        Delivery const* d = FindDelivery(quest->GetQuestId());
        if (!d)
            return true;
        std::string const day = ChallengePeriods().first;
        if (!OfferedOn(day, d->quest, TeamOf(player)))
            return false;
        // The one being delivered is the player's to turn in, whatever else holds.
        if (player->GetQuestStatus(d->quest) != QUEST_STATUS_NONE)
            return true;
        bool done;
        {
            std::lock_guard<std::mutex> lock(s_deliveryMutex);
            done = DeliveredOn(player->GetGUIDLow(), day).count(d->quest) != 0;
        }
        if (done)
        {
            if (msg)
                ChatHandler(player->GetSession()).SendSysMessage("You've made that delivery today: it's back tomorrow.");
            return false;
        }
        if (CarryingDelivery(player))
        {
            if (msg)
                ChatHandler(player->GetSession()).SendSysMessage("Deliver the package you're carrying first.");
            return false;
        }
        // One delivery at a time, its package lost or not: a second would run beside the first's
        // clock, and Courier Jobs' Cancel abandons the first in the log.
        for (Delivery const& other : DELIVERIES)
        {
            QuestStatus const status = player->GetQuestStatus(other.quest);
            if (other.quest != d->quest && (status == QUEST_STATUS_INCOMPLETE || status == QUEST_STATUS_COMPLETE))
            {
                if (msg)
                    ChatHandler(player->GetSession()).SendSysMessage(
                        "Finish or abandon the delivery in your quest log first.");
                return false;
            }
        }
        return true;
    }

    void OnQuestAccepted(Player* player, Quest const* quest)
    {
        Delivery const* d = FindDelivery(quest->GetQuestId());
        if (!d)
            return;
        {
            std::lock_guard<std::mutex> lock(s_deliveryMutex);
            s_deliveryStart[player->GetGUIDLow()] = { d->quest, std::time(nullptr) };
        }
        ChatHandler(player->GetSession()).PSendSysMessage(
            "Delivery to %s: beat %s for %u bonus tokens. It goes by kart: no flight paths, hearthstones, "
            "portals or Kartcraft teleports until it's delivered.", d->to, DeliveryClock(d->par).c_str(), DELIVERY_BONUS);
        SendDeliveryTimer(player, *d, 0);
    }

    void OnQuestRewarded(Player* player, Quest const* quest)
    {
        Delivery const* d = FindDelivery(quest->GetQuestId());
        if (!d)
            return;
        uint32 const guid = player->GetGUIDLow();
        std::time_t const now = std::time(nullptr);
        std::string const day = ChallengePeriods().first;
        uint32 seconds = 0;
        {
            std::lock_guard<std::mutex> lock(s_deliveryMutex);
            auto start = s_deliveryStart.find(guid);
            if (start != s_deliveryStart.end() && start->second.first == d->quest)
                seconds = uint32(std::max<std::time_t>(now - start->second.second, 1));
            s_deliveryStart.erase(guid);
            DeliveredOn(guid, day).insert(d->quest);
        }
        // A start the server forgot (a restart since) pays no bonus: its time is unknown.
        bool const bonus = seconds && seconds <= d->par;
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_deliveries` (`guid`, `quest`, `day`, `seconds`, "
            "`bonus`, `at`) VALUES (%u, %u, '%s', %u, %u, %u)", guid, d->quest, day.c_str(), seconds,
            bonus ? 1u : 0u, uint32(now));
        ChatHandler chat(player->GetSession());
        if (bonus)
        {
            ItemPosCountVec dest;
            if (player->CanStoreNewItem(NULL_BAG, NULL_SLOT, dest, DELIVERY_TOKEN, DELIVERY_BONUS) == EQUIP_ERR_OK)
            {
                if (Item* item = player->StoreNewItem(dest, DELIVERY_TOKEN, true))
                    player->SendNewItem(item, DELIVERY_BONUS, true, false);
                chat.PSendSysMessage("Delivered in %s, under par (%s): %u bonus tokens!",
                    DeliveryClock(seconds).c_str(), DeliveryClock(d->par).c_str(), DELIVERY_BONUS);
            }
            else
                chat.SendSysMessage("Delivered under par, but your bags had no room for the bonus tokens.");
        }
        else if (seconds)
            chat.PSendSysMessage("Delivered in %s (par %s).", DeliveryClock(seconds).c_str(), DeliveryClock(d->par).c_str());
        SendLine(player, "KARTCRAFT\tR\tDel-");
    }

    // A job's state for the player: 0 open, 1 under way (its quest in the log), 2 delivered today.
    static uint8 JobState(Player const* player, Delivery const& d, std::string const& day)
    {
        if (player->GetQuestStatus(d.quest) != QUEST_STATUS_NONE)
            return 1;
        std::lock_guard<std::mutex> lock(s_deliveryMutex);
        return DeliveredOn(player->GetGUIDLow(), day).count(d.quest) ? 2 : 0;
    }

    // "R\tJL", Courier Jobs' list: "R\tJobs\t<n>", then a line a job, "R\tJob\t<i>\t<quest>\t<state>\t<from>\t<to>
    // \t<giver>\t<recipient>\t<yards>\t<par s>\t<cargo>\t<map>\t<giver x>\t<giver y>\t<recipient x>\t<recipient y>".
    static void SendJobs(Player* player)
    {
        std::string const day = ChallengePeriods().first;
        std::vector<Delivery const*> jobs = Today(day, TeamOf(player));
        // A delivery under way that today's list no longer posts (taken before the day turned)
        // leads it, so the board still shows it under way and offers its Cancel.
        for (Delivery const& d : DELIVERIES)
        {
            QuestStatus const status = player->GetQuestStatus(d.quest);
            if ((status == QUEST_STATUS_INCOMPLETE || status == QUEST_STATUS_COMPLETE) &&
                std::find(jobs.begin(), jobs.end(), &d) == jobs.end())
            {
                jobs.insert(jobs.begin(), &d);
                break;
            }
        }
        SendLine(player, "KARTCRAFT\tR\tJobs\t" + std::to_string(jobs.size()));
        for (size_t i = 0; i < jobs.size(); ++i)
        {
            Delivery const& d = *jobs[i];
            SendLine(player, "KARTCRAFT\tR\tJob\t" + std::to_string(i + 1) + "\t" + std::to_string(d.quest) + "\t" +
                std::to_string(JobState(player, d, day)) + "\t" + d.from + "\t" + d.to + "\t" + d.giverName + "\t" +
                d.recipientName + "\t" + std::to_string(d.distance) + "\t" + std::to_string(d.par) + "\t" + d.cargo +
                "\t" + std::to_string(d.map) + "\t" + std::to_string(int32(d.gx)) + "\t" + std::to_string(int32(d.gy)) +
                "\t" + std::to_string(int32(d.rx)) + "\t" + std::to_string(int32(d.ry)));
        }
    }

    // At login: what's posted today, and a delivery under way gets its timer back; a package
    // without a start the server knows (a restart since) has none.
    static void DeliveryLogin(Player* player)
    {
        ChatHandler(player->GetSession()).PSendSysMessage(
            "%u courier jobs are posted today: Courier Jobs, in the Kartcraft menu, takes you to one.",
            uint32(Today(ChallengePeriods().first, TeamOf(player)).size()));
        if (!CarryingDelivery(player))
            return;
        std::pair<uint32, std::time_t> start;
        {
            std::lock_guard<std::mutex> lock(s_deliveryMutex);
            auto found = s_deliveryStart.find(player->GetGUIDLow());
            if (found == s_deliveryStart.end())
                return;
            start = found->second;
        }
        if (Delivery const* d = FindDelivery(start.first))
            SendDeliveryTimer(player, *d, uint32(std::time(nullptr) - start.second));
    }

    // ── Skins ──────────────────────────────────────────────────────────────────────────────────
    //
    // Famous NPCs' and creatures' looks, bought with Kartcraft Tokens in the client's skin shop
    // (BenillaKartShopFrame, whose catalogue kartcraft_tests holds to this one; each sits in the
    // kart, a creature without the chair sit in one the client makes: kart/seat.rs tests it) and worn as the
    // character's own body: the native display, so a form or a transform still shows over it and
    // gives it back, on foot and in the kart, to everyone. Owned and worn in kartcraft_skins.

    struct Skin
    {
        char const* key;
        uint32 display; // CreatureDisplayInfo
        uint32 price;   // tokens
    };

    static Skin const SKINS[] = {
        { "magni", 3597, 300 },
        { "thrall", 4527, 300 },
        { "cairne", 4307, 300 },
        { "jaina", 2970, 300 },
        { "sylvanas", 11657, 300 },
        { "voljin", 10357, 300 },
        { "bolvar", 5566, 200 },
        { "tyrande", 7274, 200 },
        { "mekkatorque", 7006, 200 },
        { "vancleef", 2029, 200 },
        { "gnarlpine", 6800, 150 },
        { "murloc", 617, 150 },
        { "kobold", 2299, 150 },
        { "razormane", 1252, 150 },
        { "goblin", 1233, 150 },
        { "zombie", 1200, 150 },
    };

    static Skin const* FindSkin(std::string const& key)
    {
        for (Skin const& s : SKINS)
            if (key == s.key)
                return &s;
        return nullptr;
    }

    static void CreateSkins()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_skins` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`skin` VARCHAR(16) NOT NULL COMMENT 'Kartcraft.cpp SKINS key', "
            "`worn` TINYINT UNSIGNED NOT NULL DEFAULT 0, "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when bought, unix time', "
            "PRIMARY KEY (`guid`, `skin`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    // The skins a character owns, and the one it wears ("" none).
    static std::pair<std::set<std::string>, std::string> OwnedSkins(uint32 guid)
    {
        std::pair<std::set<std::string>, std::string> out;
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `skin`, `worn` FROM `kartcraft_skins` WHERE `guid` = %u", guid);
        if (result)
            do
            {
                Field* f = result->Fetch();
                std::string const key = f[0].GetCppString();
                if (!FindSkin(key))
                    continue;
                out.first.insert(key);
                if (f[1].GetUInt8())
                    out.second = key;
            } while (result->NextRow());
        return out;
    }

    // "KARTCRAFT\tR\tSkins\t<tokens>\t<worn>\t<owned,...>": the shop's purse and what is ours.
    static void SendSkins(Player* player)
    {
        std::pair<std::set<std::string>, std::string> owned = OwnedSkins(player->GetGUIDLow());
        std::string list;
        for (std::string const& key : owned.first)
            list += (list.empty() ? "" : ",") + key;
        SendLine(player, "KARTCRAFT\tR\tSkins\t" + std::to_string(player->GetItemCount(DELIVERY_TOKEN)) +
            "\t" + owned.second + "\t" + list);
    }

    // The body becomes the skin's, or (none) the race's again; a form or transform showing keeps
    // showing, and gives back the new body when it ends (Unit::DeMorph).
    static void ApplySkin(Player* player, Skin const* skin)
    {
        uint32 const shown = player->GetDisplayId();
        bool const morphed = shown != player->GetNativeDisplayId();
        if (skin)
        {
            player->SetObjectScale(Unit::GetScaleForDisplayId(skin->display));
            player->SetNativeDisplayId(skin->display);
            player->SetDisplayId(skin->display);
        }
        else
            player->InitPlayerDisplayIds();
        if (morphed)
            player->SetDisplayId(shown);
        // In a scaled race: the skin's scale is the one to give back, the race's on it again next
        // tick (TickRaceScales).
        std::lock_guard<std::mutex> lock(s_raceScaleMutex);
        auto base = s_raceScaleBase.find(player->GetObjectGuid().GetRawValue());
        if (base != s_raceScaleBase.end())
            base->second = player->GetObjectScale();
    }

    static void SkinLogin(Player* player)
    {
        if (Skin const* skin = FindSkin(OwnedSkins(player->GetGUIDLow()).second))
            ApplySkin(player, skin);
    }

    // "R\tSkins" asks; "R\tSkinBuy\t<key>" buys with tokens; "R\tSkinWear\t<key>" wears an owned
    // skin, an empty key the race's own body. Each is answered with "Skins". On the world
    // thread: it takes tokens out of the bags and changes the body.
    static void HandleSkins(Player* player, std::string const& verb, std::string const& key)
    {
        ChatHandler chat(player->GetSession());
        uint32 const guid = player->GetGUIDLow();
        Skin const* skin = FindSkin(key);
        if (verb == "SkinBuy" && skin)
        {
            uint32 const tokens = player->GetItemCount(DELIVERY_TOKEN);
            if (OwnedSkins(guid).first.count(skin->key))
                chat.SendSysMessage("That skin is already yours.");
            else if (tokens < skin->price)
                chat.PSendSysMessage("That skin costs %u Kartcraft Tokens; you have %u. Courier deliveries pay them.",
                    skin->price, tokens);
            else
            {
                player->DestroyItemCount(DELIVERY_TOKEN, skin->price, true);
                // Saved at once: the good is written now, so a crash before the next save would keep both.
                player->SaveInventoryAndGoldToDB();
                CharacterDatabase.DirectPExecute("INSERT INTO `kartcraft_skins` (`guid`, `skin`, `worn`, `at`) "
                    "VALUES (%u, '%s', 0, %u)", guid, skin->key, uint32(std::time(nullptr)));
                chat.PSendSysMessage("Skin bought for %u Kartcraft Tokens.", skin->price);
            }
        }
        else if (verb == "SkinWear" && (skin || key.empty()))
        {
            if (skin && !OwnedSkins(guid).first.count(skin->key))
                chat.SendSysMessage("Buy that skin first.");
            else
            {
                CharacterDatabase.DirectPExecute("UPDATE `kartcraft_skins` SET `worn` = (`skin` = '%s') WHERE `guid` = %u",
                    skin ? skin->key : "", guid);
                ApplySkin(player, skin);
            }
        }
        SendSkins(player);
    }

    // ── Goods ──────────────────────────────────────────────────────────────────────────────────
    //
    // World of Kartcraft's own cosmetics, bought with Kartcraft Tokens in the client's shop: custom
    // karts (racer keys the client has built in, crates/kartcraft/src/goods.rs, whose catalogue
    // kartcraft_tests holds to this one) and flags flown on any kart. Owned in kartcraft_goods,
    // the flag worn there too. A kart is driven by the client's own pick (KartRacer); the relay
    // holds the karts, flags, paints and trails a driver shows to what is theirs (Shown).

    static void CreateGoods()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_goods` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`good` VARCHAR(16) NOT NULL COMMENT 'Kartcraft.cpp GOODS key', "
            "`worn` TINYINT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'a flag flown', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when bought, unix time', "
            "PRIMARY KEY (`guid`, `good`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    // The goods a character owns, and the flag it flies ("" none).
    static std::pair<std::set<std::string>, std::string> OwnedGoods(uint32 guid)
    {
        std::pair<std::set<std::string>, std::string> out;
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `good`, `worn` FROM `kartcraft_goods` WHERE `guid` = %u", guid);
        if (result)
            do
            {
                Field* f = result->Fetch();
                std::string const key = f[0].GetCppString();
                Good const* good = FindGood(key);
                if (!good)
                    continue;
                out.first.insert(key);
                if (good->kind == 'F' && f[1].GetUInt8())
                    out.second = key;
            } while (result->NextRow());
        return out;
    }

    static void ShowGoods(uint32 guid, std::pair<std::set<std::string>, std::string> const& owned)
    {
        std::lock_guard<std::mutex> lock(s_stateMutex);
        Shown& shown = s_shown[guid];
        shown.karts.clear();
        for (std::string const& key : owned.first)
            if (Good const* g = FindGood(key))
                if (g->kind == 'K')
                    shown.karts.insert(key);
        shown.flag = FlagIndex(owned.second);
    }

    // "KARTCRAFT\tR\tGoods\t<tokens>\t<flag flown>\t<owned,...>": the shop's purse, the garage's
    // flag, and what is ours.
    static void SendGoods(Player* player)
    {
        std::pair<std::set<std::string>, std::string> owned = OwnedGoods(player->GetGUIDLow());
        ShowGoods(player->GetGUIDLow(), owned);
        std::string list;
        for (std::string const& key : owned.first)
            list += (list.empty() ? "" : ",") + key;
        SendLine(player, "KARTCRAFT\tR\tGoods\t" + std::to_string(player->GetItemCount(DELIVERY_TOKEN)) +
            "\t" + owned.second + "\t" + list);
    }

    static void GoodsLogin(Player* player)
    {
        ShowGoods(player->GetGUIDLow(), OwnedGoods(player->GetGUIDLow()));
    }

    // "R\tGoods" asks; "R\tGoodBuy\t<key>" buys with tokens; "R\tFlagWear\t<key>" flies an owned
    // flag, an empty key none. Each is answered with "Goods". On the world thread: it takes tokens
    // out of the bags.
    static void HandleGoods(Player* player, std::string const& verb, std::string const& key)
    {
        ChatHandler chat(player->GetSession());
        uint32 const guid = player->GetGUIDLow();
        Good const* good = FindGood(key);
        if (verb == "GoodBuy" && good && good->tier)
            chat.SendSysMessage("That is found only in Mystery Crates.");
        else if (verb == "GoodBuy" && good)
        {
            uint32 const tokens = player->GetItemCount(DELIVERY_TOKEN);
            if (OwnedGoods(guid).first.count(good->key))
                chat.SendSysMessage("That is already yours.");
            else if (tokens < good->price)
                chat.PSendSysMessage("That costs %u Kartcraft Tokens; you have %u. Courier deliveries pay them.",
                    good->price, tokens);
            else
            {
                player->DestroyItemCount(DELIVERY_TOKEN, good->price, true);
                // Saved at once: the good is written now, so a crash before the next save would keep both.
                player->SaveInventoryAndGoldToDB();
                CharacterDatabase.DirectPExecute("INSERT INTO `kartcraft_goods` (`guid`, `good`, `worn`, `at`) "
                    "VALUES (%u, '%s', 0, %u)", guid, good->key, uint32(std::time(nullptr)));
                chat.PSendSysMessage("Bought for %u Kartcraft Tokens.", good->price);
            }
        }
        else if (verb == "FlagWear" && ((good && good->kind == 'F') || key.empty()))
        {
            if (good && !OwnedGoods(guid).first.count(good->key))
                chat.SendSysMessage("Buy that flag first.");
            else
                CharacterDatabase.DirectPExecute("UPDATE `kartcraft_goods` SET `worn` = (`good` = '%s') WHERE `guid` = %u",
                    good ? good->key : "", guid);
        }
        SendGoods(player);
    }

    // ── Caches ─────────────────────────────────────────────────────────────────────────────────
    //
    // The Astral and Wumpa Caches: chests on spots along the drivable roads (sql/kartcraft_caches.sql,
    // generated from roads/roads.json), pooled one live a square, coming back somewhere else in it
    // two to five minutes after one is claimed, as a looted chest does (GameObject::Update). A
    // driver claims one by driving through it (the client's "R\tAC\t<guid>", near it in the kart)
    // or anyone by opening it (go_kartcraft_cache); the first claim wins. A cache pays gold by the
    // claimer's level and Kartcraft Tokens; the Astral more. Not in a race, at most one claim every
    // CACHE_INTERVAL, and past CACHE_DAILY claims a day only a quarter of the gold. Each claim is
    // kept in kartcraft_cache_claims.

    constexpr uint32 CACHE_ASTRAL = 190200;
    constexpr uint32 CACHE_WUMPA = 190201;
    // How near (yards) a claim may be made from, the time between two (ms), and claims a day.
    constexpr float CACHE_RADIUS = 12.f;
    constexpr uint32 CACHE_INTERVAL = 20 * IN_MILLISECONDS;
    constexpr uint32 CACHE_DAILY = 20;
    // The chance (%) an Astral Cache holds a cosmetic found nowhere else.
    constexpr uint32 CACHE_HIDDEN_PERCENT = 3;

    static std::unordered_map<uint32, uint32> s_lastClaim; // player guid low -> ms, world thread

    static bool InRace(Unit const* u);
    static void GiveTokens(Player* player, uint32 count);
    static bool CanBeRacedOff(Player* player);
    static bool TeleportOnto(Player* player, uint32 map, float x, float y, float o, bool kart, float z = 0.f);

    static void CreateCacheClaims()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_cache_claims` ("
            "`id` INT UNSIGNED NOT NULL AUTO_INCREMENT, "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`cache` INT UNSIGNED NOT NULL COMMENT 'gameobject spawn guid', "
            "`entry` INT UNSIGNED NOT NULL, "
            "`gold` INT UNSIGNED NOT NULL COMMENT 'copper', "
            "`tokens` INT UNSIGNED NOT NULL, "
            "`at` INT UNSIGNED NOT NULL COMMENT 'unix time', "
            "PRIMARY KEY (`id`), KEY `guid_at` (`guid`, `at`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    static bool IsCache(GameObject const* go)
    {
        return go && (go->GetEntry() == CACHE_ASTRAL || go->GetEntry() == CACHE_WUMPA);
    }

    // `player` claims `go`, if it is a cache still there and theirs to claim: the gold and tokens,
    // the chest gone (to come back elsewhere in its square), and the client told
    // ("KARTCRAFT\tR\tACGot\t<copper>\t<tokens>\t<1 astral, 0 wumpa>"). On the world thread.
    static void CacheClaim(Player* player, GameObject* go)
    {
        ChatHandler chat(player->GetSession());
        if (!IsCache(go) || !go->isSpawned() || go->getLootState() != GO_READY)
            return;
        if (!player->IsWithinDistInMap(go, CACHE_RADIUS))
            return;
        if (InRace(player))
        {
            chat.SendSysMessage("Caches wait for you outside a race.");
            return;
        }
        uint32 const guid = player->GetGUIDLow();
        uint32 const now = WorldTimer::getMSTime();
        uint32& last = s_lastClaim[guid];
        if (last && WorldTimer::getMSTimeDiff(last, now) < CACHE_INTERVAL)
            return;
        last = now;
        bool const astral = go->GetEntry() == CACHE_ASTRAL;
        uint32 const today = uint32(std::time(nullptr)) - uint32(std::time(nullptr)) % DAY;
        char sql[256];
        std::snprintf(sql, sizeof(sql),
            "SELECT COUNT(*) FROM `kartcraft_cache_claims` WHERE `guid` = %u AND `at` >= %u", guid, today);
        uint32 const claimed = CountOf(sql);
        // About a silver a level from a Wumpa Cache, three from an Astral one (60: 60 s, 1 g 80 s).
        uint32 gold = player->GetLevel() * (astral ? 300 : 100) + urand(0, player->GetLevel() * 20);
        uint32 tokens = astral ? urand(10, 25) : urand(3, 8);
        if (claimed >= CACHE_DAILY)
        {
            gold /= 4;
            tokens = 0;
        }
        go->SetLootState(GO_JUST_DEACTIVATED);
        player->ModifyMoney(int32(gold));
        GiveTokens(player, tokens);
        // An Astral Cache's rare prize: a cosmetic found nowhere else.
        if (astral && claimed < CACHE_DAILY && urand(1, 100) <= CACHE_HIDDEN_PERCENT)
        {
            std::string const hidden = FindHidden(player);
            if (!hidden.empty())
            {
                chat.PSendSysMessage("Inside the Astral Cache: the %s, found nowhere else! Wear it in the garage.",
                    hidden.c_str());
                SendCosmetics(player);
            }
        }
        CharacterDatabase.PExecute("INSERT INTO `kartcraft_cache_claims` (`guid`, `cache`, `entry`, `gold`, `tokens`, `at`) "
            "VALUES (%u, %u, %u, %u, %u, %u)", guid, go->GetGUIDLow(), go->GetEntry(), gold, tokens,
            uint32(std::time(nullptr)));
        SendLine(player, "KARTCRAFT\tR\tACGot\t" + std::to_string(gold) + "\t" + std::to_string(tokens) + "\t" +
            (astral ? "1" : "0"));
        if (claimed >= CACHE_DAILY)
            chat.SendSysMessage("You have claimed many caches today: the rest pay little until tomorrow.");
    }

    // Opened on foot (or clicked from the kart): claimed, and no loot window.
    static bool CacheHello(Player* player, GameObject* go)
    {
        CacheClaim(player, go);
        return true;
    }

    // "R\tAC\t<guid>": a driver's kart touched the cache with this full guid. On the world thread.
    static void CacheTouched(Player* player, uint64 raw)
    {
        ObjectGuid const guid(raw);
        if (!guid.IsGameObject() || (guid.GetEntry() != CACHE_ASTRAL && guid.GetEntry() != CACHE_WUMPA))
            return;
        if (!player->HasAura(SPELL_SUMMON_KART))
            return;
        CacheClaim(player, player->GetMap()->GetGameObject(guid));
    }

    // ── Hub ────────────────────────────────────────────────────────────────────────────────────
    //
    // The Mirage Raceway, World of Kartcraft's meeting place: neutral ground on the Shimmering
    // Flats with marshals for its courses, and the Quartermaster (sql/kartcraft.sql, its menu
    // here) for the garage, the shop, the tracks and the Race Finder. Within HUB_RADIUS of it
    // is a sanctuary: nothing attacks anyone there. "R\tHub" (/hub) goes there
    // from the open world, in the kart if in it.

    constexpr uint32 HUB_TEXT = 190020;
    constexpr uint32 HUB_MAP = 1;
    constexpr float HUB_X = -6214.f, HUB_Y = -3894.f, HUB_O = 0.f;
    enum HubAction : uint32 { HUB_GARAGE = 1, HUB_SHOP, HUB_TRACKS, HUB_FINDER };

    // The sanctuary: within HUB_RADIUS of the hub nothing attacks anyone, the race's shield on
    // them (TickShields) while they are there. Who is there, under s_mutex (HubTick).
    constexpr float HUB_RADIUS = 150.f;
    constexpr uint32 HUB_TICK = 2 * IN_MILLISECONDS;
    static std::set<ObjectGuid> s_hubbed;
    static uint32 s_hubTicked = 0;

    static bool HubHello(Player* player, Creature* quartermaster)
    {
        PlayerMenu* talk = player->PlayerTalkClass;
        talk->ClearMenus();
        GossipMenu& menu = talk->GetGossipMenu();
        menu.AddMenuItem(GOSSIP_ICON_VENDOR, "Open the garage (karts, paints, trails, flags)", 0, HUB_GARAGE, "", false);
        menu.AddMenuItem(GOSSIP_ICON_VENDOR, "Open the Kartcraft Shop", 0, HUB_SHOP, "", false);
        menu.AddMenuItem(GOSSIP_ICON_TAXI, "Browse the tracks", 0, HUB_TRACKS, "", false);
        menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Open the Race Finder", 0, HUB_FINDER, "", false);
        talk->SendGossipMenu(HUB_TEXT, quartermaster->GetObjectGuid());
        return true;
    }

    // The client opens the window the line names: "Finder" and "Tracks" as from a marshal, and
    // "Open\tgarage" or "Open\tshop".
    static bool HubSelect(Player* player, Creature* /*quartermaster*/, uint32 /*sender*/, uint32 action)
    {
        player->PlayerTalkClass->CloseGossip();
        switch (action)
        {
            case HUB_GARAGE: SendLine(player, "KARTCRAFT\tR\tOpen\tgarage"); break;
            case HUB_SHOP: SendLine(player, "KARTCRAFT\tR\tOpen\tshop"); break;
            case HUB_TRACKS: SendLine(player, "KARTCRAFT\tR\tTracks"); break;
            case HUB_FINDER: SendLine(player, "KARTCRAFT\tR\tFinder"); break;
            default: break;
        }
        return true;
    }

    // "R\tHub": to the Raceway, from the open world (not a race, a battleground, a dungeon or a
    // delivery's road). On the world thread.
    static void HubTeleport(Player* player)
    {
        ChatHandler chat(player->GetSession());
        if (InRace(player) || !CanBeRacedOff(player))
        {
            chat.SendSysMessage("The hub is reached from the open world, out of a race and not carrying a delivery.");
            return;
        }
        if (TeleportOnto(player, HUB_MAP, HUB_X, HUB_Y, HUB_O, player->HasAura(SPELL_SUMMON_KART)))
            chat.SendSysMessage("Off to the Mirage Raceway, the racers' hub.");
    }

    // ── Props ──────────────────────────────────────────────────────────────────────────────────
    //
    // The client's props (props/props.json: the hub's stunt park, Skyhop's platforms, and their
    // pads) are out only where mangosd.conf's Kartcraft.Props is 1, off by default: each client
    // is told at login ("R\tProps\t1" or "0") and draws, drives and races none until told, and
    // Skyhop's finishes are taken only while they are out.

    static bool PropsOn()
    {
        static bool const on = sConfig.GetBoolDefault("Kartcraft.Props", false);
        return on;
    }

    static void PropsLogin(Player* player)
    {
        SendLine(player, PropsOn() ? "KARTCRAFT\tR\tProps\t1" : "KARTCRAFT\tR\tProps\t0");
    }

    // ── Replays ────────────────────────────────────────────────────────────────────────────────
    //
    // Every race and time trial is recorded here, from what the server already sees: each racer's
    // place and facing (their kart's movement, 15 a second) and their kart's state line (steer,
    // drift, flags; RelayState), sampled every REPLAY_STEP from GO until the race is gone, a
    // racer's own run REPLAY_AFTER past their finish. A race's take is written once it is over, to
    // Kartcraft.ReplayDir as <key>.krps (the race's start, unix time, and its id), and kept in
    // kartcraft_replays with its racers in kartcraft_replay_racers. A take lives REPLAY_KEEP; past
    // that only one that holds an achievement (kartcraft_replay_keeps: a personal best or a course
    // record on either board, a challenge best, each replaced as it is beaten) or that a racer has
    // pinned (kartcraft_replay_pins) stays. Clients fetch the files over HTTP from
    // Kartcraft.ReplayUrl, told at login ("R\tRUrl\t<url>", empty none); "R\tRList" lists our races'
    // takes, "R\tRRec\t<course>" a course's record takes, "R\tRPin\t<key>\t<1|0>" pins one of ours.

    constexpr uint32 REPLAY_STEP = 66;
    constexpr uint32 REPLAY_AFTER = 8 * IN_MILLISECONDS;
    constexpr uint32 REPLAY_MAX_SAMPLES = 20 * 60 * 15;
    constexpr uint32 REPLAY_KEEP = 2 * DAY;
    constexpr uint32 REPLAY_SWEEP = HOUR * IN_MILLISECONDS;
    constexpr uint8 REPLAY_VERSION = 1;

    // One sample: the place in hundredths of a yard, the facing over 65536 of a turn, and the
    // state line's steer (-100..100), drift yaw (milliradians) and flags.
    struct ReplaySample
    {
        int32 x, y, z;
        uint16 o;
        int8 steer;
        int16 drift;
        uint8 flags;
    };

    struct ReplayTrack
    {
        ObjectGuid guid;
        std::string name;
        std::string kart = "crash";
        uint8 paint = 0, trail = 0, flag = 0;
        uint8 place = 0;
        uint32 ms = 0;
        uint32 finishedAt = 0; // ms, 0 not yet
        uint32 from = 0;       // the sample its first is
        std::vector<ReplaySample> samples;
    };

    struct ReplayTake
    {
        std::string key;
        std::string course;
        uint32 startedAt = 0;
        bool trial = false;
        uint32 start = 0;
        uint32 lastSample = 0;
        std::vector<ReplayTrack> tracks;
    };

    static std::unordered_map<uint32, ReplayTake> s_takes; // race id -> its take, s_mutex
    static uint32 s_replaySwept = 0;

    static std::string ReplayKey(Race const& r)
    {
        return std::to_string(r.startedAt) + "-" + std::to_string(r.id);
    }

    static std::string ReplayDir()
    {
        return sConfig.GetStringDefault("Kartcraft.ReplayDir", "/opt/vmangos/storage/replays");
    }

    static void CreateReplays()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_replays` ("
            "`key` VARCHAR(24) NOT NULL COMMENT 'the race start (unix time) and its id', "
            "`course` VARCHAR(32) NOT NULL, "
            "`trial` TINYINT UNSIGNED NOT NULL, "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when it started, unix time', "
            "`field` TINYINT UNSIGNED NOT NULL, "
            "`bytes` INT UNSIGNED NOT NULL, "
            "PRIMARY KEY (`key`), KEY `at` (`at`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_replay_racers` ("
            "`key` VARCHAR(24) NOT NULL, "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`name` VARCHAR(12) NOT NULL, "
            "`place` TINYINT UNSIGNED NOT NULL COMMENT '0 did not finish', "
            "`ms` INT UNSIGNED NOT NULL, "
            "PRIMARY KEY (`key`, `guid`), KEY `guid` (`guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_replay_keeps` ("
            "`reason` CHAR(2) NOT NULL COMMENT 'pR/pT a personal best, rR/rT a course record (race/trial), cd/cw a challenge best, lb a ladder season best', "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'whose best; 0 for a record', "
            "`course` VARCHAR(32) NOT NULL COMMENT 'course key, or a challenge period', "
            "`key` VARCHAR(24) NOT NULL, "
            "`holder` INT UNSIGNED NOT NULL COMMENT 'who set it, character low guid', "
            "PRIMARY KEY (`reason`, `guid`, `course`), KEY `key` (`key`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute("ALTER TABLE `kartcraft_replay_keeps` ADD COLUMN IF NOT EXISTS `value` INT "
            "NOT NULL DEFAULT 0 COMMENT 'what a best is measured by (lb: the rating gained), the greater kept'");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_replay_pins` ("
            "`key` VARCHAR(24) NOT NULL, "
            "`guid` INT UNSIGNED NOT NULL, "
            "PRIMARY KEY (`key`, `guid`), KEY `guid` (`guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        mkdir(ReplayDir().c_str(), 0755);
    }

    // A take holds an achievement: `reason` for `guid` (0 a record) on `course` (or a challenge
    // period), set by `holder` in race `r`, replacing the one before it.
    static void ReplayKeep(Race const& r, char const* reason, uint32 guid, std::string const& course, uint32 holder)
    {
        std::string escaped = course;
        CharacterDatabase.escape_string(escaped);
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_replay_keeps` (`reason`, `guid`, `course`, `key`, `holder`) "
            "VALUES ('%s', %u, '%s', '%s', %u)", reason, guid, escaped.c_str(), ReplayKey(r).c_str(), holder);
    }

    // A take holds `reason` for `guid` on `course` while `value` is the best of it: replaced by a
    // greater one only.
    static void ReplayKeepBest(Race const& r, char const* reason, uint32 guid, std::string const& course,
        uint32 holder, int32 value)
    {
        std::string escaped = course;
        CharacterDatabase.escape_string(escaped);
        CharacterDatabase.PExecute("INSERT INTO `kartcraft_replay_keeps` (`reason`, `guid`, `course`, `key`, `holder`, "
            "`value`) VALUES ('%s', %u, '%s', '%s', %u, %i) ON DUPLICATE KEY UPDATE "
            "`key` = IF(VALUES(`value`) > `value`, VALUES(`key`), `key`), "
            "`holder` = IF(VALUES(`value`) > `value`, VALUES(`holder`), `holder`), "
            "`value` = GREATEST(`value`, VALUES(`value`))",
            reason, guid, escaped.c_str(), ReplayKey(r).c_str(), holder, value);
    }

    // "KARTCRAFT\tS\t<steer>,<drift>,<fire>,<distortion>,<volume>,<flags>,<racer>,<wheelie>,
    // <paint>,<trail>,<flag>" (kart/remote.rs state_line): its fields.
    static std::vector<std::string> StateFields(std::string const& line)
    {
        std::vector<std::string> out;
        size_t head = line.find("\tS\t");
        if (head == std::string::npos)
            return out;
        std::string body = line.substr(head + 3);
        size_t at = 0;
        while (at <= body.size())
        {
            size_t comma = body.find(',', at);
            if (comma == std::string::npos)
                comma = body.size();
            out.push_back(body.substr(at, comma - at));
            at = comma + 1;
        }
        return out;
    }

    static int32 StateInt(std::vector<std::string> const& f, size_t i, int32 lo, int32 hi)
    {
        if (i >= f.size())
            return 0;
        int32 v = int32(std::strtol(f[i].c_str(), nullptr, 10));
        return std::max(lo, std::min(hi, v));
    }

    // A racer's finish into the race's take at once: the last finisher's race is gone before the
    // next sample could see it. Under s_mutex.
    static void ReplayFinish(uint32 race, ObjectGuid guid, uint8 place, uint32 ms, uint32 now)
    {
        auto take = s_takes.find(race);
        if (take == s_takes.end())
            return;
        for (ReplayTrack& t : take->second.tracks)
            if (t.guid == guid && !t.finishedAt)
            {
                t.finishedAt = now ? now : 1;
                t.place = place;
                t.ms = ms;
            }
    }

    // Every race under way: each racer's place now, every REPLAY_STEP. Under s_mutex.
    static void SampleReplays(uint32 now)
    {
        for (auto& entry : s_races)
        {
            Race& r = entry.second;
            if (!r.start || !r.course || int32(now - r.start) < 0)
                continue;
            ReplayTake& take = s_takes[entry.first];
            if (take.key.empty())
            {
                take.key = ReplayKey(r);
                take.course = r.course->key;
                take.startedAt = r.startedAt;
                take.trial = r.timeTrial;
                take.start = r.start;
            }
            if (take.lastSample && WorldTimer::getMSTimeDiff(take.lastSample, now) < REPLAY_STEP)
                continue;
            take.lastSample = now;
            uint32 const index = WorldTimer::getMSTimeDiff(r.start, now) / REPLAY_STEP;
            if (index >= REPLAY_MAX_SAMPLES)
                continue;
            for (Racer const& x : r.racers)
            {
                auto t = std::find_if(take.tracks.begin(), take.tracks.end(),
                    [&x](ReplayTrack const& k) { return k.guid == x.guid; });
                if (t == take.tracks.end())
                {
                    take.tracks.emplace_back();
                    t = std::prev(take.tracks.end());
                    t->guid = x.guid;
                    t->name = x.name;
                    t->from = index;
                }
                if (x.finished && !t->finishedAt)
                {
                    t->finishedAt = now ? now : 1;
                    t->place = x.place;
                    t->ms = x.ms;
                }
                if (t->finishedAt && WorldTimer::getMSTimeDiff(t->finishedAt, now) > REPLAY_AFTER)
                    continue;
                Player* p = ObjectAccessor::FindPlayer(x.guid);
                if (!p || !p->IsInWorld() || p->GetMapId() != r.course->map)
                    continue;
                ReplaySample s;
                s.x = int32(std::lround(p->GetPositionX() * 100.f));
                s.y = int32(std::lround(p->GetPositionY() * 100.f));
                s.z = int32(std::lround(p->GetPositionZ() * 100.f));
                s.o = uint16(std::lround(p->GetOrientation() / 6.2831853f * 65536.f) & 0xFFFF);
                std::vector<std::string> f;
                {
                    std::lock_guard<std::mutex> lock(s_stateMutex);
                    auto line = s_stateLines.find(x.guid.GetCounter());
                    if (line != s_stateLines.end())
                        f = StateFields(line->second);
                }
                s.steer = int8(StateInt(f, 0, -100, 100));
                s.drift = int16(StateInt(f, 1, -3200, 3200));
                s.flags = uint8(StateInt(f, 5, 0, 255));
                if (f.size() > 6 && !f[6].empty() && f[6].size() <= 12)
                    t->kart = f[6];
                t->paint = uint8(StateInt(f, 8, 0, 255));
                t->trail = uint8(StateInt(f, 9, 0, 255));
                t->flag = uint8(StateInt(f, 10, 0, 255));
                // A gap (out of the world a while) holds the last place, so a sample's index is
                // its time.
                if (index < t->from)
                    continue;
                if (t->samples.empty())
                    t->from = index;
                while (!t->samples.empty() && t->from + t->samples.size() < index)
                    t->samples.push_back(t->samples.back());
                if (t->from + t->samples.size() == index)
                    t->samples.push_back(s);
            }
        }
    }

    static void PutVar(std::string& out, uint32 v)
    {
        while (v >= 0x80)
        {
            out.push_back(char((v & 0x7F) | 0x80));
            v >>= 7;
        }
        out.push_back(char(v));
    }

    static void PutSigned(std::string& out, int32 v)
    {
        PutVar(out, (uint32(v) << 1) ^ uint32(v >> 31));
    }

    static void PutStr(std::string& out, std::string const& s)
    {
        std::string const cut = s.substr(0, 255);
        out.push_back(char(cut.size()));
        out += cut;
    }

    static void PutU32(std::string& out, uint32 v)
    {
        for (int i = 0; i < 4; ++i)
            out.push_back(char((v >> (8 * i)) & 0xFF));
    }

    // The file: "KRPS", a version byte and the body's size (u32), then the body deflated: the
    // step (ms, u16), the start (u32), a time trial (u8), the course, the track count (u8), and
    // per track its name, kart, paint, trail, flag, place (u8), time (ms, u32), first sample
    // (u32), sample count (u32) and samples, each field the change from the one before as a
    // zigzag varint (the first from zero), the steer and flags as bytes. Little endian; a string
    // is a length byte and its bytes.
    static std::string EncodeTake(ReplayTake const& take)
    {
        std::string body;
        body.push_back(char(REPLAY_STEP & 0xFF));
        body.push_back(char(REPLAY_STEP >> 8));
        PutU32(body, take.startedAt);
        body.push_back(char(take.trial ? 1 : 0));
        PutStr(body, take.course);
        size_t const n = std::min<size_t>(take.tracks.size(), 16);
        body.push_back(char(n));
        for (size_t i = 0; i < n; ++i)
        {
            ReplayTrack const& t = take.tracks[i];
            PutStr(body, t.name);
            PutStr(body, t.kart);
            body.push_back(char(t.paint));
            body.push_back(char(t.trail));
            body.push_back(char(t.flag));
            body.push_back(char(t.place));
            PutU32(body, t.ms);
            PutU32(body, t.from);
            PutU32(body, uint32(t.samples.size()));
            ReplaySample last = {};
            for (ReplaySample const& s : t.samples)
            {
                PutSigned(body, s.x - last.x);
                PutSigned(body, s.y - last.y);
                PutSigned(body, s.z - last.z);
                PutSigned(body, int16(uint16(s.o - last.o)));
                body.push_back(char(s.steer));
                PutSigned(body, s.drift - last.drift);
                body.push_back(char(s.flags));
                last = s;
            }
        }
        uLongf packed = compressBound(uLong(body.size()));
        std::string out = "KRPS";
        out.push_back(char(REPLAY_VERSION));
        PutU32(out, uint32(body.size()));
        std::string z(packed, '\0');
        if (compress2(reinterpret_cast<Bytef*>(&z[0]), &packed, reinterpret_cast<Bytef const*>(body.data()),
                uLong(body.size()), 9) != Z_OK)
            return "";
        z.resize(packed);
        return out + z;
    }

    // A take whose race is gone: to its file and its rows. Under s_mutex.
    static void WriteTake(ReplayTake const& take)
    {
        bool any = false;
        for (ReplayTrack const& t : take.tracks)
            any = any || t.samples.size() > 15;
        if (!any || take.key.empty())
            return;
        std::string const bytes = EncodeTake(take);
        if (bytes.empty())
            return;
        std::string const path = ReplayDir() + "/" + take.key + ".krps";
        std::ofstream file(path, std::ios::binary | std::ios::trunc);
        if (!file)
        {
            sLog.Out(LOG_BASIC, LOG_LVL_ERROR, "Kartcraft: cannot write replay %s", path.c_str());
            return;
        }
        file.write(bytes.data(), std::streamsize(bytes.size()));
        file.close();
        std::string course = take.course;
        CharacterDatabase.escape_string(course);
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_replays` (`key`, `course`, `trial`, `at`, `field`, `bytes`) "
            "VALUES ('%s', '%s', %u, %u, %u, %u)", take.key.c_str(), course.c_str(), take.trial ? 1u : 0u,
            take.startedAt, uint32(take.tracks.size()), uint32(bytes.size()));
        for (ReplayTrack const& t : take.tracks)
        {
            std::string name = t.name;
            CharacterDatabase.escape_string(name);
            CharacterDatabase.PExecute("REPLACE INTO `kartcraft_replay_racers` (`key`, `guid`, `name`, `place`, `ms`) "
                "VALUES ('%s', %u, '%s', %u, %u)", take.key.c_str(), t.guid.GetCounter(), name.c_str(),
                uint32(t.place), t.ms);
        }
    }

    // Takes whose race is over, written. Under s_mutex.
    static void FinishTakes()
    {
        for (auto it = s_takes.begin(); it != s_takes.end();)
        {
            if (s_races.count(it->first))
            {
                ++it;
                continue;
            }
            WriteTake(it->second);
            it = s_takes.erase(it);
        }
    }

    // Takes past REPLAY_KEEP that hold no achievement and no pin: gone, file and rows. On the
    // world thread.
    static void SweepReplays()
    {
        uint32 const before = uint32(std::time(nullptr)) - REPLAY_KEEP;
        // A finished season's ladder bests: its top three's only (three rated above, gone).
        CharacterDatabase.DirectPExecute("DELETE k FROM `kartcraft_replay_keeps` k WHERE k.`reason` = 'lb' "
            "AND k.`course` < '%u' AND (SELECT COUNT(*) FROM `kartcraft_ladder` l WHERE l.`season` = k.`course` "
            "AND l.`rating` > (SELECT m.`rating` FROM `kartcraft_ladder` m WHERE m.`season` = k.`course` "
            "AND m.`guid` = k.`guid`)) >= 3", SeasonNow());
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT r.`key` FROM `kartcraft_replays` r WHERE r.`at` < %u "
            "AND NOT EXISTS (SELECT 1 FROM `kartcraft_replay_keeps` k WHERE k.`key` = r.`key`) "
            "AND NOT EXISTS (SELECT 1 FROM `kartcraft_replay_pins` p WHERE p.`key` = r.`key`) LIMIT 1000", before);
        if (!result)
            return;
        uint32 gone = 0;
        do
        {
            std::string const key = result->Fetch()[0].GetCppString();
            std::remove((ReplayDir() + "/" + key + ".krps").c_str());
            CharacterDatabase.PExecute("DELETE FROM `kartcraft_replays` WHERE `key` = '%s'", key.c_str());
            CharacterDatabase.PExecute("DELETE FROM `kartcraft_replay_racers` WHERE `key` = '%s'", key.c_str());
            ++gone;
        } while (result->NextRow());
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "Kartcraft: %u replays past %u days swept.", gone, REPLAY_KEEP / DAY);
    }

    // A replay key as the client sends it: digits, a dash, digits.
    static bool ReplayKeyOk(std::string const& key)
    {
        if (key.empty() || key.size() > 24)
            return false;
        bool dash = false;
        for (char c : key)
        {
            if (c == '-' && !dash)
                dash = true;
            else if (!std::isdigit(static_cast<unsigned char>(c)))
                return false;
        }
        return dash;
    }

    // "R\tRList": our races' takes, newest first, then "R\tRL-"; each
    // "R\tRL\t<key>\t<course>\t<trial>\t<at>\t<field>\t<place>\t<ms>\t<pinned>\t<kept>".
    // "R\tRRec\t<course>": its record takes, "R\tRR\t<course>\t<R|T>\t<key>\t<name>\t<ms>" each,
    // then "R\tRR-\t<course>". "R\tRPin\t<key>\t<1|0>": one of ours pinned or not. World thread.
    static void HandleReplays(Player* player, std::vector<std::string> const& f)
    {
        uint32 const guid = player->GetGUIDLow();
        if (f[1] == "RList")
        {
            // A take saved before ReplayFinish holds 0 ms and place 0 for its last finisher: the
            // race's result (kartcraft_results, the same start and race id) gives them.
            std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
                "SELECT r.`key`, r.`course`, r.`trial`, r.`at`, r.`field`, "
                "IF(m.`ms` = 0, COALESCE((SELECT s.`place` FROM `kartcraft_results` s WHERE s.`race_at` = r.`at` "
                "AND s.`race` = SUBSTRING_INDEX(r.`key`, '-', -1) AND s.`guid` = m.`guid` ORDER BY s.`ms` DESC LIMIT 1), "
                "m.`place`), m.`place`), "
                "IF(m.`ms` = 0, COALESCE((SELECT s.`ms` FROM `kartcraft_results` s WHERE s.`race_at` = r.`at` "
                "AND s.`race` = SUBSTRING_INDEX(r.`key`, '-', -1) AND s.`guid` = m.`guid` ORDER BY s.`ms` DESC LIMIT 1), "
                "0), m.`ms`), "
                "EXISTS (SELECT 1 FROM `kartcraft_replay_pins` p WHERE p.`key` = r.`key` AND p.`guid` = %u), "
                "EXISTS (SELECT 1 FROM `kartcraft_replay_keeps` k WHERE k.`key` = r.`key`) "
                "FROM `kartcraft_replay_racers` m JOIN `kartcraft_replays` r ON r.`key` = m.`key` "
                "WHERE m.`guid` = %u ORDER BY r.`at` DESC LIMIT 50", guid, guid);
            if (result)
            {
                do
                {
                    Field* x = result->Fetch();
                    SendLine(player, "KARTCRAFT\tR\tRL\t" + x[0].GetCppString() + "\t" + x[1].GetCppString() + "\t" +
                        std::to_string(x[2].GetUInt32()) + "\t" + std::to_string(x[3].GetUInt32()) + "\t" +
                        std::to_string(x[4].GetUInt32()) + "\t" + std::to_string(x[5].GetUInt32()) + "\t" +
                        std::to_string(x[6].GetUInt32()) + "\t" + std::to_string(x[7].GetUInt32()) + "\t" +
                        std::to_string(x[8].GetUInt32()));
                } while (result->NextRow());
            }
            SendLine(player, "KARTCRAFT\tR\tRL-");
        }
        else if (f[1] == "RRec" && f.size() >= 3 && FindCourse(f[2]))
        {
            std::string const course = FindCourse(f[2])->key;
            std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
                "SELECT k.`reason`, k.`key`, m.`name`, m.`ms` FROM `kartcraft_replay_keeps` k "
                "JOIN `kartcraft_replay_racers` m ON m.`key` = k.`key` AND m.`guid` = k.`holder` "
                "JOIN `kartcraft_replays` r ON r.`key` = k.`key` "
                "WHERE k.`guid` = 0 AND k.`course` = '%s' AND k.`reason` IN ('rR', 'rT')", course.c_str());
            if (result)
            {
                do
                {
                    Field* x = result->Fetch();
                    std::string const reason = x[0].GetCppString();
                    std::string const name = x[2].GetCppString();
                    uint32 ms = x[3].GetUInt32();
                    // A take saved before ReplayFinish holds 0 ms for its record holder: the board's time.
                    if (!ms)
                    {
                        std::lock_guard<std::mutex> lock(s_mutex);
                        Board const& board = reason == "rT" ? s_trialBoard : s_raceBoard;
                        auto record = board.records.find(course);
                        if (record != board.records.end() && record->second.name == name)
                            ms = record->second.ms;
                    }
                    SendLine(player, "KARTCRAFT\tR\tRR\t" + course + "\t" + (reason == "rT" ? "T" : "R") + "\t" +
                        x[1].GetCppString() + "\t" + name + "\t" + std::to_string(ms));
                } while (result->NextRow());
            }
            SendLine(player, "KARTCRAFT\tR\tRR-\t" + course);
        }
        else if (f[1] == "RPin" && f.size() >= 4 && ReplayKeyOk(f[2]))
        {
            std::unique_ptr<QueryResult> ours = CharacterDatabase.PQuery(
                "SELECT 1 FROM `kartcraft_replay_racers` WHERE `key` = '%s' AND `guid` = %u", f[2].c_str(), guid);
            if (!ours)
                return;
            if (f[3] == "1")
                CharacterDatabase.PExecute("REPLACE INTO `kartcraft_replay_pins` (`key`, `guid`) VALUES ('%s', %u)",
                    f[2].c_str(), guid);
            else
                CharacterDatabase.PExecute("DELETE FROM `kartcraft_replay_pins` WHERE `key` = '%s' AND `guid` = %u",
                    f[2].c_str(), guid);
            SendLine(player, "KARTCRAFT\tR\tRP\t" + f[2] + "\t" + (f[3] == "1" ? "1" : "0"));
        }
    }

    static void ReplayLogin(Player* player)
    {
        SendLine(player, "KARTCRAFT\tR\tRUrl\t" + sConfig.GetStringDefault("Kartcraft.ReplayUrl", ""));
    }

    // ── Skyhop ─────────────────────────────────────────────────────────────────────────────────
    //
    // The client's challenges of floating platforms (props/props.json, kart/skyhop.rs), run out of
    // any race: a finish is told ("R\tHop\t<key>\t<ms>") from the goal, in the kart, no faster
    // than a run can be (the client's bot, on CTR's physics, takes 11.17 s), and kept as the
    // racer's best in kartcraft_hops; a new record is said to everyone. The answer is
    // "R\tHopBest\t<key>\t<ms>\t<best>\t<record>\t<holder>".

    struct HopCourse
    {
        char const* key;
        float goal[3];  // the last gate, props.json's (kart/skyhop.rs's tests hold them alike)
        uint32 map;
        uint32 fastest; // ms
    };

    static HopCourse const HOP_COURSES[] = {
        { "skyhop", { -5651.06f, -4029.94f, -23.8f }, 1, 9000 },
    };
    // How near the goal (yards) a finish is told from.
    constexpr float HOP_GOAL_RADIUS = 30.f;

    static void CreateHops()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_hops` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`course` VARCHAR(16) NOT NULL COMMENT 'HOP_COURSES key', "
            "`ms` INT UNSIGNED NOT NULL, "
            "`at` INT UNSIGNED NOT NULL COMMENT 'unix time', "
            "PRIMARY KEY (`guid`, `course`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    static std::string HopClock(uint32 ms)
    {
        char text[32];
        std::snprintf(text, sizeof(text), "%u.%02u s", ms / 1000, (ms % 1000) / 10);
        return text;
    }

    // A finish told: kept if it is the racer's best, said to all if it is the record. On the world
    // thread.
    static void HandleHop(Player* player, std::string const& key, uint32 ms)
    {
        HopCourse const* course = nullptr;
        for (HopCourse const& c : HOP_COURSES)
            if (key == c.key)
                course = &c;
        if (!PropsOn() || !course || ms < course->fastest || !player->HasAura(SPELL_SUMMON_KART) || InRace(player) ||
            player->GetMapId() != course->map ||
            player->GetDistance(course->goal[0], course->goal[1], course->goal[2]) > HOP_GOAL_RADIUS)
            return;
        uint32 const guid = player->GetGUIDLow();
        auto best_of = [&](char const* sql) -> std::pair<uint32, std::string>
        {
            std::unique_ptr<QueryResult> result = CharacterDatabase.Query(sql);
            if (!result)
                return { 0, "" };
            Field* f = result->Fetch();
            return { f[0].GetUInt32(), f[1].GetCppString() };
        };
        char sql[384];
        std::snprintf(sql, sizeof(sql), "SELECT h.`ms`, c.`name` FROM `kartcraft_hops` h JOIN `characters` c ON "
            "c.`guid` = h.`guid` WHERE h.`course` = '%s' ORDER BY h.`ms` LIMIT 1", course->key);
        std::pair<uint32, std::string> const record = best_of(sql);
        std::snprintf(sql, sizeof(sql), "SELECT `ms`, '' FROM `kartcraft_hops` WHERE `guid` = %u AND `course` = '%s'",
            guid, course->key);
        uint32 best = best_of(sql).first;
        if (!best || ms < best)
        {
            CharacterDatabase.DirectPExecute("REPLACE INTO `kartcraft_hops` (`guid`, `course`, `ms`, `at`) "
                "VALUES (%u, '%s', %u, %u)", guid, course->key, ms, uint32(std::time(nullptr)));
            best = ms;
        }
        uint32 rec = record.first;
        std::string holder = record.second;
        if (!rec || ms < rec)
        {
            if (rec)
                sWorld.SendGlobalText((std::string(player->GetName()) + " set a new Skyhop record: " + HopClock(ms) +
                    " (was " + HopClock(rec) + " by " + holder + ")!").c_str(), nullptr);
            rec = ms;
            holder = player->GetName();
        }
        SendLine(player, "KARTCRAFT\tR\tHopBest\t" + std::string(course->key) + "\t" + std::to_string(ms) + "\t" +
            std::to_string(best) + "\t" + std::to_string(rec) + "\t" + holder);
    }

    // ── Mystery Crates ─────────────────────────────────────────────────────────────────────────
    //
    // A crate for CRATE_PRICE Kartcraft Tokens, at most CRATE_DAILY a day, holds one of the goods
    // found only in crates (GOODS' tier 1 to 4: common, rare, epic, legendary), rolled here and
    // never on the client: the tier by CRATE_WEIGHTS among those the opener lacks (none ever
    // twice; an emptied tier gives way up, then down), an epic or better guaranteed by
    // CRATE_PITY_EPIC crates without one and a legendary by CRATE_PITY_LEGENDARY, the odds and the
    // counts said to the shop ("R\tCrateOdds"). With no crate goods yet the crate is closed.
    // Opened in kartcraft_crates, each roll kept in kartcraft_crate_log; the prize is a good
    // owned as a bought one is.

    constexpr uint32 CRATE_PRICE = 100;
    constexpr uint32 CRATE_DAILY = 5;
    constexpr uint32 CRATE_WEIGHTS[4] = { 60, 28, 10, 2 };
    constexpr uint32 CRATE_PITY_EPIC = 20;
    constexpr uint32 CRATE_PITY_LEGENDARY = 50;

    static void CreateCrates()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_crates` ("
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`since_epic` INT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'crates since an epic or better', "
            "`since_legendary` INT UNSIGNED NOT NULL DEFAULT 0, "
            "`day` INT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'unix day of the count', "
            "`today` INT UNSIGNED NOT NULL DEFAULT 0 COMMENT 'crates opened that day', "
            "PRIMARY KEY (`guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_crate_log` ("
            "`id` INT UNSIGNED NOT NULL AUTO_INCREMENT, "
            "`guid` INT UNSIGNED NOT NULL, "
            "`good` VARCHAR(16) NOT NULL, "
            "`tier` TINYINT UNSIGNED NOT NULL, "
            "`at` INT UNSIGNED NOT NULL, "
            "PRIMARY KEY (`id`), KEY `guid` (`guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    struct CrateState
    {
        uint32 sinceEpic = 0;
        uint32 sinceLegendary = 0;
        uint32 today = 0;
    };

    static CrateState CrateStateOf(uint32 guid)
    {
        CrateState s;
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT `since_epic`, `since_legendary`, `day`, `today` FROM `kartcraft_crates` WHERE `guid` = %u", guid);
        if (result)
        {
            Field* f = result->Fetch();
            s.sinceEpic = f[0].GetUInt32();
            s.sinceLegendary = f[1].GetUInt32();
            s.today = f[2].GetUInt32() == uint32(std::time(nullptr)) / DAY ? f[3].GetUInt32() : 0;
        }
        return s;
    }

    // The crate goods `guid` lacks, by tier (index 0 the commons).
    static std::vector<std::vector<Good const*>> CrateLeft(uint32 guid)
    {
        std::set<std::string> const owned = OwnedGoods(guid).first;
        std::vector<std::vector<Good const*>> left(4);
        for (Good const& g : GOODS)
            if (g.tier >= 1 && g.tier <= 4 && !owned.count(g.key))
                left[g.tier - 1].push_back(&g);
        return left;
    }

    static bool CrateHasGoods()
    {
        for (Good const& g : GOODS)
            if (g.tier >= 1 && g.tier <= 4)
                return true;
        return false;
    }

    // "KARTCRAFT\tR\tCrateOdds\t<price>\t<left today>\t<since epic>\t<since legendary>\t<lacking
    // c,r,e,l>\t<weights c,r,e,l>\t<pity epic>\t<pity legendary>"; with no crate goods, lacking is
    // "-" (closed).
    static void SendCrateOdds(Player* player)
    {
        uint32 const guid = player->GetGUIDLow();
        CrateState const s = CrateStateOf(guid);
        std::string lacking = "-";
        if (CrateHasGoods())
        {
            std::vector<std::vector<Good const*>> const left = CrateLeft(guid);
            lacking.clear();
            for (size_t t = 0; t < 4; ++t)
                lacking += (t ? "," : "") + std::to_string(left[t].size());
        }
        std::string weights;
        for (size_t t = 0; t < 4; ++t)
            weights += (t ? "," : "") + std::to_string(CRATE_WEIGHTS[t]);
        SendLine(player, "KARTCRAFT\tR\tCrateOdds\t" + std::to_string(CRATE_PRICE) + "\t" +
            std::to_string(CRATE_DAILY > s.today ? CRATE_DAILY - s.today : 0) + "\t" + std::to_string(s.sinceEpic) +
            "\t" + std::to_string(s.sinceLegendary) + "\t" + lacking + "\t" + weights + "\t" +
            std::to_string(CRATE_PITY_EPIC) + "\t" + std::to_string(CRATE_PITY_LEGENDARY));
    }

    // The tier (0 common .. 3 legendary) a crate gives, among those with goods `left`: the pity's
    // first, else by weight; an empty tier gives way up, then down. -1 none at all.
    static int32 CrateTier(std::vector<std::vector<Good const*>> const& left, CrateState const& s)
    {
        auto has = [&](int32 t) { return t >= 0 && t < 4 && !left[t].empty(); };
        auto nearest = [&](int32 t)
        {
            for (int32 up = t; up < 4; ++up)
                if (has(up))
                    return up;
            for (int32 down = t - 1; down >= 0; --down)
                if (has(down))
                    return down;
            return int32(-1);
        };
        if (s.sinceLegendary + 1 >= CRATE_PITY_LEGENDARY && has(3))
            return 3;
        if (s.sinceEpic + 1 >= CRATE_PITY_EPIC && (has(2) || has(3)))
            return has(2) ? 2 : 3;
        uint32 total = 0;
        for (uint32 w : CRATE_WEIGHTS)
            total += w;
        uint32 roll = urand(1, total);
        int32 t = 0;
        while (t < 3 && roll > CRATE_WEIGHTS[t])
            roll -= CRATE_WEIGHTS[t++];
        return nearest(t);
    }

    // "R\tCrateOdds" asks; "R\tCrateBuy" opens one: the tokens taken, the roll, the good owned, and
    // "KARTCRAFT\tR\tCrate\t<good>\t<tier 1-4>". On the world thread.
    static void HandleCrate(Player* player, std::string const& verb)
    {
        ChatHandler chat(player->GetSession());
        uint32 const guid = player->GetGUIDLow();
        if (verb == "CrateBuy")
        {
            CrateState s = CrateStateOf(guid);
            std::vector<std::vector<Good const*>> const left = CrateLeft(guid);
            uint32 const tokens = player->GetItemCount(DELIVERY_TOKEN);
            int32 const tier = CrateTier(left, s);
            if (!CrateHasGoods())
                chat.SendSysMessage("Mystery Crates are coming soon.");
            else if (tier < 0)
                chat.SendSysMessage("Your collection is complete: every crate's prize is yours.");
            else if (s.today >= CRATE_DAILY)
                chat.PSendSysMessage("You have opened %u crates today; more tomorrow.", CRATE_DAILY);
            else if (tokens < CRATE_PRICE)
                chat.PSendSysMessage("A Mystery Crate costs %u Kartcraft Tokens; you have %u.", CRATE_PRICE, tokens);
            else
            {
                Good const* prize = left[tier][urand(0, uint32(left[tier].size()) - 1)];
                player->DestroyItemCount(DELIVERY_TOKEN, CRATE_PRICE, true);
                // Saved at once: the good is written now, so a crash before the next save would keep both.
                player->SaveInventoryAndGoldToDB();
                CharacterDatabase.DirectPExecute("INSERT INTO `kartcraft_goods` (`guid`, `good`, `worn`, `at`) "
                    "VALUES (%u, '%s', 0, %u)", guid, prize->key, uint32(std::time(nullptr)));
                s.sinceEpic = tier >= 2 ? 0 : s.sinceEpic + 1;
                s.sinceLegendary = tier >= 3 ? 0 : s.sinceLegendary + 1;
                CharacterDatabase.DirectPExecute("REPLACE INTO `kartcraft_crates` (`guid`, `since_epic`, "
                    "`since_legendary`, `day`, `today`) VALUES (%u, %u, %u, %u, %u)", guid, s.sinceEpic,
                    s.sinceLegendary, uint32(std::time(nullptr)) / DAY, s.today + 1);
                CharacterDatabase.PExecute("INSERT INTO `kartcraft_crate_log` (`guid`, `good`, `tier`, `at`) "
                    "VALUES (%u, '%s', %u, %u)", guid, prize->key, uint32(tier + 1), uint32(std::time(nullptr)));
                SendLine(player, "KARTCRAFT\tR\tCrate\t" + std::string(prize->key) + "\t" + std::to_string(tier + 1));
                SendGoods(player);
            }
        }
        SendCrateOdds(player);
    }

    static bool HasRated(uint32 guid, char const* course)
    {
        std::unique_ptr<QueryResult> result = CharacterDatabase.PQuery(
            "SELECT 1 FROM `kartcraft_rating` WHERE `course` = '%s' AND `guid` = %u", course, guid);
        return bool(result);
    }

    // After a racer's result: ask them to rate the track if they raced at least half of it.
    static void AskRating(Race const& r, Racer const& x, uint32 place)
    {
        Course const& c = *r.course;
        bool half = place > 0 || Passes(x, uint32(c.points.size())) * 2 >= FinalPass(c);
        if (!half || HasRated(x.guid.GetCounter(), c.key))
            return;
        if (Player* p = Recipient(x.guid))
            SendLine(p, std::string("KARTCRAFT\tR\tRateAsk\t") + c.key);
    }

    static void Rate(Player* player, std::string const& key, std::string const& vote, std::string const& why)
    {
        Course const* c = FindCourse(key);
        int v = vote == "1" ? 1 : vote == "-1" ? -1 : 0;
        if (!c || !v)
            return;
        std::string reason;
        if (v < 0)
            for (char const* r : RATE_REASONS)
                if (why == r)
                    reason = r;
        uint32 guid = player->GetGUIDLow();
        std::unique_ptr<QueryResult> raced = CharacterDatabase.PQuery(
            "SELECT 1 FROM `kartcraft_results` WHERE `course` = '%s' AND `guid` = %u LIMIT 1", c->key, guid);
        if (!raced)
        {
            ChatHandler(player->GetSession()).SendSysMessage("Race a track before you rate it.");
            return;
        }
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_rating` (`course`, `guid`, `vote`, `reason`, `at`) "
            "VALUES ('%s', %u, %d, '%s', %u)", c->key, guid, v, reason.c_str(), uint32(std::time(nullptr)));
    }

    // One racer's result in a started race: `place` 0 did not finish. The last of a race's
    // results rates it on the ladder.
    static void RecordResult(Race& r, Racer const& x, uint32 place, uint32 ms)
    {
        if (!r.start || !r.course)
            return;
        r.placed.push_back({ x.guid, { x.name, place } });
        if (r.placed.size() >= r.field)
            RateRace(r);
        std::string escaped = x.name;
        CharacterDatabase.escape_string(escaped);
        CharacterDatabase.PExecute("INSERT INTO `kartcraft_results` (`race_at`, `race`, `course`, `guid`, "
            "`name`, `place`, `field`, `ms`, `trial`, `at`) VALUES (%u, %u, '%s', %u, '%s', %u, %u, %u, %u, %u)",
            r.startedAt, r.id, r.course->key, x.guid.GetCounter(), escaped.c_str(), place, uint32(r.field), ms,
            r.timeTrial ? 1u : 0u, uint32(std::time(nullptr)));
        AskRating(r, x, place);
    }

    // A finish of `ms` on `course`: "r" a new course record, "p" a new personal best, else "".
    static std::string RecordFinish(Board& board, uint32 guid, std::string const& name, Course const& course, uint32 ms)
    {
        if (!ms)
            return "";
        Record& best = board.bests[{ guid, course.key }];
        if (best.ms && best.ms <= ms)
            return "";
        best.ms = ms;
        best.name = name;
        std::string escaped = name;
        CharacterDatabase.escape_string(escaped);
        CharacterDatabase.PExecute("REPLACE INTO `%s` (`guid`, `course`, `name`, `ms`, `at`) "
            "VALUES (%u, '%s', '%s', %u, %u)", board.table.c_str(), guid, course.key, escaped.c_str(), ms,
            uint32(std::time(nullptr)));
        Record& record = board.records[course.key];
        if (record.ms && record.ms <= ms)
            return "p";
        record = best;
        return "r";
    }

    static uint32 BestOf(Board const& board, uint32 guid, std::string const& course)
    {
        auto found = board.bests.find({ guid, course });
        return found == board.bests.end() ? 0 : found->second.ms;
    }

    // A course's times on `board`, fastest first (a tie by name), as (guid low, best).
    static std::vector<std::pair<uint32, Record>> Ranking(Board const& board, std::string const& course)
    {
        std::vector<std::pair<uint32, Record>> out;
        for (auto const& entry : board.bests)
            if (entry.first.second == course && entry.second.ms)
                out.emplace_back(entry.first.first, entry.second);
        std::sort(out.begin(), out.end(), [](std::pair<uint32, Record> const& a, std::pair<uint32, Record> const& b)
        {
            return a.second.ms != b.second.ms ? a.second.ms < b.second.ms : a.second.name < b.second.name;
        });
        return out;
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
    // A shot breaks a crate no farther from where it left than it can have flown at the warp
    // orb's 112 lu a frame, the fastest, 92 yd/s, plus a juiced bomb's 512 lu blast, the crate's
    // size and slack, 16 yd. A dropped mine breaks only the crate it lands on: CRATE_RADIUS.
    constexpr float SHOT_SPEED = 92.f;
    constexpr float BREAK_REACH = 16.f;
    // CTR's projectile pool, and the W lines one sender may send a second: half again the 20 a
    // second the client paces itself to, so a burst its sliding window allows never lands over a
    // fixed second here.
    constexpr size_t MAX_LIVE_SHOTS = 12;
    constexpr uint32 MAX_WEAPON_LINES = 30;
    // Wumpa: capped at 10, juiced at 10 (DRIVER_WUMPA_JUICED_COUNT).
    constexpr uint8 MAX_WUMPA = 10;
    // The mask protects 7680 ms, 11520 ms juiced (VehPickupItem.c:193-351).
    constexpr uint32 MASK_TIME = 7680;
    constexpr uint32 MASK_TIME_JUICED = 11520;
    // A green shield lasts 12 s (VehPickupItem.c:976-983); a second's slack for the line.
    constexpr uint32 SHIELD_TIME = 13 * IN_MILLISECONDS;
    // The turbo item burns 2400 ms of reserves; a ram under it counts this long after its relay.
    constexpr uint32 TURBO_TIME = 3 * IN_MILLISECONDS;
    // A rammer and its victim are this near (yards): both positions here trail their karts.
    constexpr float RAM_RADIUS = 20.f;
    // A red beaker's rain cloud lasts 7680 ms (RB_MaskShieldCloud.c:615). Its effects
    // (namespace_Vehicle.h:629-632): slow, the held item spinning, ice, no hop.
    constexpr uint32 CLOUD_TIME = 0x1E00;
    enum RainEffect : uint8 { CLOUD_SLOW = 0, CLOUD_ITEM_ROLL = 1 };
    // A whole addon line stays under this (docs/ctr-weapons-assets-and-net.md, B1).
    constexpr size_t MAX_LINE = 200;

    // Per segment between consecutive checkpoints: a fruit crate a quarter of the way, a row of
    // weapon crates across the path halfway (yards apart), and single fruit three quarters along.
    constexpr float CRATE_ROW_SPACING = 4.f;
    constexpr uint32 CRATE_ROW = 4;
    constexpr uint32 SEGMENT_FRUIT = 3;
    // Kartcraft's own: a weapon crate row and a fruit crate on one segment in CRATES_EVERY, not
    // on every one (a quarter of the crates); the rows a segment apart from the fruit crates, and
    // a course of fewer segments still gets one row.
    constexpr size_t CRATES_EVERY = 4;
    constexpr float FRUIT_SPACING = 4.f;

    // The roulette tables (zGlobal_DATA.c:8108-8174; Race4 samples only its first 19 entries,
    // which these are). Kartcraft's own: N. Tropy's clock (8) is taken out of every table and
    // nothing takes its place, so each table rolls its other items in their own proportions.
    constexpr uint8 TABLE_RACE1[] = { 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 0, 6 };
    constexpr uint8 TABLE_RACE2[] = {
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 1, 10, 10, 10,
        0, 0, 0, 0, 0, 6, 6, 6, 2, 2, 2, 11, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7 };
    constexpr uint8 TABLE_RACE3[] = { 4, 3, 1, 10, 10, 0, 0, 6, 2, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7 };
    constexpr uint8 TABLE_RACE4[] = { 0, 6, 2, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 7 };
    constexpr uint8 TABLE_BOSS[] = { 4, 3, 1, 10, 10, 0, 0, 6, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7 };

    // A table's item for a roll of 0-199: `item = table[rng * N / 200]` (VehPhysGeneral.c:1807-1841).
    template <size_t N>
    static uint8 Pick(uint8 const (&table)[N], uint32 rng)
    {
        return table[rng * N / 200];
    }

    // The strongest items wait and take turns, as Mario Kart's do (Kartcraft's own rule: CTR
    // has none): no Missile x3 or mask in a race's first STRONG_DELAY, no warp orb in its first
    // WARP_DELAY nor within WARP_COOLDOWN of the last one rolled in the race. A roll that comes
    // to one of them too soon is a turbo, as Mario Kart Wii's is a mushroom.
    constexpr uint32 STRONG_DELAY = 15 * IN_MILLISECONDS;
    constexpr uint32 WARP_DELAY = 30 * IN_MILLISECONDS;
    constexpr uint32 WARP_COOLDOWN = 30 * IN_MILLISECONDS;

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
        size_t const rowAt = segments ? std::min(CRATES_EVERY / 2, segments - 1) : 0;
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
            if (i % CRATES_EVERY == 0)
            {
                auto fruitCrate = at(0.25f, 0.f, 0.f);
                place('F', fruitCrate.first, fruitCrate.second);
            }
            if (i % CRATES_EVERY == rowAt)
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

    // A crate's LInC breaks it for whatever reaches it (RB_Crate.c:123-131): a bomb, missile or
    // shield bomb touching it or bursting near it, a warp orb touching it, a TNT, Nitro or beaker
    // dropped on it (RB_MovingExplosive.c:220-277, RB_Burst.c:286-317, RB_Warpball.c:607-640,
    // VehPickupItem.c:1199-1251). A TNT or Nitro going off breaks none (RB_Blowup.c:159-180).
    static bool BreaksCrates(Shot const& shot)
    {
        switch (shot.item)
        {
            case ITEM_BOMB: case ITEM_BOMB_3: case ITEM_MISSILE: case ITEM_MISSILE_3: case ITEM_WARP:
                return !shot.dropped;
            case ITEM_SHIELD:
                return shot.shieldBomb;
            case ITEM_TNT: case ITEM_BEAKER:
                return shot.dropped;
        }
        return false;
    }

    // Only a bomb, missile or shield bomb gives its shooter the crate it breaks
    // (RB_Crate.c:152-226, :315-355).
    static bool RewardsShooter(Shot const& shot)
    {
        return BreaksCrates(shot) && shot.item != ITEM_WARP && !shot.dropped;
    }

    // The x and y of a place ParsePlace wrote.
    static void PlaceXY(std::string const& place, float& x, float& y)
    {
        char* end = nullptr;
        x = std::strtof(place.c_str(), &end);
        y = std::strtof(end + 1, nullptr);
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
            // Seven, which CTR never fields (Kartcraft's own): its 8- and 6-driver pattern, the
            // leader Race1, then two each to Race2, Race3 and Race4.
            case 7: set = rank == 0 ? SET_RACE1 : int((rank - 1) / 2 + 1); break;
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
            // A race with one driver: the leader's.
            case 1: set = SET_RACE1; break;
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
            case SET_RACE1: item = Pick(TABLE_RACE1, rng); break;
            case SET_RACE2: item = Pick(TABLE_RACE2, rng); break;
            case SET_RACE3: item = Pick(TABLE_RACE3, rng); break;
            case SET_RACE4: item = Pick(TABLE_RACE4, rng); break;
            default: item = Pick(TABLE_BOSS, rng); break;
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
        // The strongest items wait and take turns (STRONG_DELAY, WARP_DELAY, WARP_COOLDOWN).
        uint32 const raced = race.start && int32(now - race.start) > 0 ? now - race.start : 0;
        if (item == ITEM_WARP && (raced < WARP_DELAY ||
            (race.warpRolled && WorldTimer::getMSTimeDiff(race.warpRolled, now) < WARP_COOLDOWN)))
            item = ITEM_TURBO;
        if ((item == ITEM_MISSILE_3 || item == ITEM_MASK) && raced < STRONG_DELAY)
            item = ITEM_TURBO;
        if (item == ITEM_WARP)
            race.warpRolled = now;
        return item;
    }

    // A roulette a rain cloud held spins out: a new roll, shown at once, so its shots are taken
    // at once (RB_MaskShieldCloud.c:556-561).
    static void Reroll(Race& race, Racer& me, uint32 now)
    {
        me.cloudLocked = false;
        me.held = Roll(race, me, now);
        me.shots = me.held == ITEM_BOMB_3 || me.held == ITEM_MISSILE_3 ? 3 : 1;
        me.claimed = now - ROLL_MIN;
        if (Player* p = Recipient(me.guid))
            SendLine(p, "KARTCRAFT\tW\tI\t" + std::to_string(me.held) + "\t" + std::to_string(me.shots));
    }

    static bool CloudUp(Racer const& me, uint32 now)
    {
        return me.cloudUntil && int32(now - me.cloudUntil) < 0;
    }

    // A red beaker's spin-out took on a racer (RB_GenericMine.c:172-176, RB_RainCloud_Init at
    // RB_MaskShieldCloud.c:568-641): a first cloud spins a held item (ITEM_ROLL) or slows a racer
    // holding nothing; under a cloud the time starts again and the effect is drawn anew,
    // (MixRNG % 400) / 100. ITEM_ROLL takes a held item into the roulette (:543-551); a redraw
    // away from it rolls a roulette it held at once, its timer no longer held at 5.
    static void Rain(Race& race, Racer& me, uint32 now)
    {
        bool holds = me.held != ITEM_NONE && me.shots;
        if (CloudUp(me, now))
            me.cloudEffect = uint8(Scramble(race) % 400 / 100);
        else
        {
            me.cloudEffect = holds ? CLOUD_ITEM_ROLL : CLOUD_SLOW;
            me.cloudLocked = false;
        }
        if (me.cloudEffect == CLOUD_ITEM_ROLL && holds)
        {
            me.cloudLocked = true;
            me.held = ITEM_NONE;
            me.shots = 0;
        }
        me.cloudUntil = (now + CLOUD_TIME) | 1;
        SendRace(race, "KARTCRAFT\tW\tR\t" + std::to_string(me.guid.GetCounter()) + "\t" +
            std::to_string(me.cloudEffect) + "\t" + (me.cloudLocked ? "1" : "0"));
        if (me.cloudLocked && me.cloudEffect != CLOUD_ITEM_ROLL)
            Reroll(race, me, now);
    }

    // What a crate broken by `me` (their kart or their shot) gives them: a weapon crate the
    // roulette's roll, unless they still hold an item or a cloud spins one, which breaks it for
    // nothing (RB_Crate.c:152-226); a fruit crate 5-8 Wumpa (RB_Crate.c:316), a single fruit 1.
    static void GiveCrate(Race& race, Racer& me, Player* player, char kind, uint32 now)
    {
        if (kind == 'Q')
        {
            if (CloudUp(me, now) && me.cloudEffect == CLOUD_ITEM_ROLL)
                return;
            // Holding one already, as the client claimed a weapon crate only with its slot empty:
            // the two disagree (a fire line lost), so its roulette lands on what we hold.
            if (me.held != ITEM_NONE || me.shots)
            {
                SendLine(player, "KARTCRAFT\tW\tY\t" + std::to_string(me.held) + "\t" + std::to_string(me.shots));
                return;
            }
            uint8 item = Roll(race, me, now);
            me.held = item;
            me.shots = item == ITEM_BOMB_3 || item == ITEM_MISSILE_3 ? 3 : 1;
            me.claimed = now;
            SendLine(player, "KARTCRAFT\tW\tI\t" + std::to_string(item) + "\t" + std::to_string(me.shots));
            return;
        }
        uint32 gain = kind == 'F' ? Scramble(race) % 4 + 5 : 1;
        me.wumpa = uint8(std::min<uint32>(me.wumpa + gain, MAX_WUMPA));
        SendLine(player, "KARTCRAFT\tW\tF\t" + std::to_string(me.wumpa));
    }

    // The crates' layout to every racer, or to `only` (one who asked again, "W\tL?": theirs was
    // lost on the way, a loading screen or a relog in between).
    static void SendLayout(Race const& race, Player* only = nullptr)
    {
        std::string const head = "KARTCRAFT\tW\tL\t";
        std::string line;
        size_t first = 0;
        auto flush = [&](size_t next)
        {
            if (!line.empty())
            {
                if (only)
                    SendLine(only, head + std::to_string(first) + "\t" + line);
                else
                    SendRace(race, head + std::to_string(first) + "\t" + line);
            }
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
            r.turboAt = 0;
            r.cloudUntil = 0;
            r.cloudLocked = false;
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
        if (race.start && !racer->finished)
            RecordResult(race, *racer, 0, 0);
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
        for (ObjectGuid const& guid : s_hubbed)
            want[guid] |= RACE_SHIELD_FLAGS;
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

    // ── Race layers ────────────────────────────────────────────────────────────────────────────
    //
    // Each race under way is its own instance of the world: its racers see only each other, and
    // nobody else sees them (Player::IsVisibleInGridForPlayer), nor hears their /say, emotes or karts
    // (the player message deliverers). Creatures are in every layer. A racer is in the race's
    // layer from its start until they leave it, the results included.
    //
    // s_layer is written on the world thread between map updates (the messager), then read by
    // the map threads' visibility checks and by the async packet thread's kart relay: a shared
    // lock for the readers, and s_layered so a world with no race under way takes no lock.
    static std::unordered_map<uint32, uint32> s_layer; // player low guid -> race id
    static std::shared_timed_mutex s_layerMutex;
    static std::atomic<size_t> s_layered{ 0 };
    // What the last TickLayers handed the messager, under s_mutex.
    static std::map<ObjectGuid, uint32> s_layerPosted;

    static uint32 LayerOf(WorldObject const* o)
    {
        auto it = s_layer.find(o->GetGUIDLow());
        return it == s_layer.end() ? 0 : it->second;
    }

    // The kart abilities (the Karting profession's buffs) are the open world's: never in a race
    // or a time trial.
    static constexpr uint32 KART_ABILITIES[] = { SPELL_WUMPA_JUICE, SPELL_TURBO_BOOST, SPELL_SUPER_ENGINE, SPELL_AKU_AKU };

    // Whether a player is in a race's layer: from its countdown until they leave it.
    static bool InRace(Unit const* u)
    {
        if (s_layered.load(std::memory_order_acquire) == 0 || !u->IsPlayer())
            return false;
        std::shared_lock<std::shared_timed_mutex> lock(s_layerMutex);
        return LayerOf(u) != 0;
    }

    bool SameLayer(WorldObject const* a, WorldObject const* b)
    {
        if (s_layered.load(std::memory_order_acquire) == 0 || a == b || !a->IsPlayer() || !b->IsPlayer())
            return true;
        std::shared_lock<std::shared_timed_mutex> lock(s_layerMutex);
        return LayerOf(a) == LayerOf(b);
    }

    // Run after every Kartcraft line and at login: who is in which race's layer now. A change
    // goes through the messager, which swaps the table and redraws what the moved players see and
    // who sees them, as GM mode's toggle does.
    static void TickLayers()
    {
        std::map<ObjectGuid, uint32> layers;
        for (auto const& entry : s_races)
            if (entry.second.start)
                for (Racer const& r : entry.second.racers)
                    layers[r.guid] = entry.first;
        if (layers == s_layerPosted)
            return;
        std::vector<ObjectGuid> moved;
        for (auto const& had : s_layerPosted)
        {
            auto now = layers.find(had.first);
            if (now == layers.end() || now->second != had.second)
                moved.push_back(had.first);
        }
        for (auto const& now : layers)
            if (!s_layerPosted.count(now.first))
                moved.push_back(now.first);
        s_layerPosted = layers;
        sWorld.GetMessager().AddMessage([layers, moved](World*)
        {
            {
                std::unique_lock<std::shared_timed_mutex> lock(s_layerMutex);
                s_layer.clear();
                for (auto const& l : layers)
                    s_layer[l.first.GetCounter()] = l.second;
                s_layered.store(s_layer.size(), std::memory_order_release);
            }
            for (ObjectGuid const& guid : moved)
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                {
                    p->UpdateVisibilityAndView();
                    if (layers.count(guid))
                        for (uint32 spell : KART_ABILITIES)
                            p->RemoveAurasDueToSpell(spell);
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

    // Crates due back come up; old shots are forgotten; rain clouds go, rolling the roulettes
    // they held. Run on every Kartcraft line: a racer's kart sends its state ten times a second,
    // so a respawn is late by at most about 100 ms.
    static void TickWeapons(uint32 now)
    {
        for (auto& entry : s_races)
        {
            Race& race = entry.second;
            for (Racer& r : race.racers)
                if (r.cloudUntil && int32(now - r.cloudUntil) >= 0)
                {
                    r.cloudUntil = 0;
                    if (r.cloudLocked)
                        Reroll(race, r, now);
                }
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
            if (!r.start || int32(now - r.start) < 0 || r.crates.empty() || r.timeTrial)
                continue;
            if (Racer* x = FindRacer(r, player->GetGUIDLow()))
            {
                race = &r;
                me = x;
            }
        }
        if (!race || player->GetMapId() != race->course->map ||
            (race->instance && player->GetInstanceId() != race->instance))
            return;
        std::string const& verb = f[1];
        if (verb == "L?")
        {
            sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "Kartcraft: %s asked for the crates again", player->GetName());
            SendLayout(*race, player);
            return;
        }
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
            GiveCrate(*race, *me, player, crate.kind, now);
        }
        else if (verb == "K" && f.size() >= 4)
        {
            // Our shot broke a crate: a live shot of the sender's that breaks crates, and a full-
            // grown crate (no single fruit) it can have reached by now. It comes back as any crate.
            char* end = nullptr;
            unsigned long shot = std::strtoul(f[2].c_str(), &end, 10);
            if (end == f[2].c_str() || *end || shot < 1 || shot > 0xFFFF)
                return;
            unsigned long index = std::strtoul(f[3].c_str(), &end, 10);
            if (end == f[3].c_str() || *end || index >= race->crates.size())
                return;
            auto found = race->shots.find((uint64(player->GetGUIDLow()) << 16) | shot);
            if (found == race->shots.end() || !LiveShot(found->second, now) || !BreaksCrates(found->second))
                return;
            Shot const& s = found->second;
            Crate& crate = race->crates[index];
            if (crate.grownAt && int32(now - crate.grownAt) >= 0)
                crate.grownAt = 0;
            if (crate.kind == 'W' || !crate.up || crate.grownAt)
                return;
            float dx = crate.x - s.x;
            float dy = crate.y - s.y;
            float reach = s.dropped ? CRATE_RADIUS
                                    : SHOT_SPEED * float(WorldTimer::getMSTimeDiff(s.at, now)) / float(IN_MILLISECONDS) + BREAK_REACH;
            if (std::sqrt(dx * dx + dy * dy) > reach)
                return;
            crate.up = false;
            crate.respawnAt = now + CRATE_RESPAWN;
            // A warp orb or a mine breaks it for no one: taken by guid 0.
            bool reward = RewardsShooter(s);
            SendRace(*race, "KARTCRAFT\tW\tX\t" + std::to_string(index) + "\t" + (reward ? guid : std::string("0")));
            if (reward && !me->finished)
                GiveCrate(*race, *me, player, crate.kind, now);
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
            // The client spent the shot when it fired: so does this, relayed or not.
            if (--me->shots == 0)
                me->held = ITEM_NONE;
            if (IsProjectile(item))
            {
                size_t live = 0;
                for (auto const& s : race->shots)
                    live += InPool(s.second, now);
                if (live >= MAX_LIVE_SHOTS)
                    return;
            }
            bool juiced = me->wumpa >= MAX_WUMPA;
            if (item == ITEM_MASK)
            {
                me->maskUntil = now + (juiced ? MASK_TIME_JUICED : MASK_TIME);
                // ACTION_MASK_WEAPON blows a rain cloud away (RB_MaskShieldCloud.c:527-532).
                me->cloudUntil = 0;
            }
            if (item == ITEM_TURBO)
                me->turboAt = now ? now : 1;
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
            record.juiced = juiced;
            PlaceXY(place, record.x, record.y);
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
            record.shieldBomb = true;
            PlaceXY(place, record.x, record.y);
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
            bool red = false;
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
                red = s.item == ITEM_BEAKER && s.juiced && kind == 'S';
            }
            // Spin-out and squish cost 1, a blast 3; a TNT landing on the head costs nothing until
            // it goes off; none while masked (RB_Player.c:153-161).
            uint32 loss = kind == 'B' ? 3 : kind == 'T' ? 0 : 1;
            if (me->maskUntil && int32(now - me->maskUntil) < 0)
                loss = 0;
            me->wumpa = uint8(me->wumpa > loss ? me->wumpa - loss : 0);
            SendRace(*race, "KARTCRAFT\tW\tH\t" + guid + "\t" + std::to_string(shooter) + "\t" + std::to_string(shot) +
                "\t" + std::string(1, kind) + "\t" + std::to_string(me->wumpa));
            if (red)
                Rain(*race, *me, now);
        }
        else if (verb == "B" && f.size() >= 4)
        {
            // Rammed (VehPhysCrash_Attack, VehPhysCrash.c:199-252): only by a racer near us with
            // its mask on, its shield up (spent by the ram) or its turbo item burning.
            uint32 low = uint32(std::strtoul(f[2].c_str(), nullptr, 10));
            Racer* attacker = low == player->GetGUIDLow() ? nullptr : FindRacer(*race, low);
            Player* rammer = attacker ? ObjectAccessor::FindPlayer(attacker->guid) : nullptr;
            if (!rammer || f[3].size() != 1 ||
                DistanceTo(rammer, { player->GetPositionX(), player->GetPositionY() }) > RAM_RADIUS)
                return;
            char ram = f[3][0];
            bool armed = false;
            if (ram == 'M')
                armed = attacker->maskUntil && int32(now - attacker->maskUntil) < 0;
            else if (ram == 'S')
            {
                armed = attacker->shieldAt &&
                    (attacker->shieldJuiced || WorldTimer::getMSTimeDiff(attacker->shieldAt, now) <= SHIELD_TIME);
                attacker->shieldAt = 0;
            }
            else if (ram == 'T')
                armed = attacker->turboAt && WorldTimer::getMSTimeDiff(attacker->turboAt, now) <= TURBO_TIME;
            if (!armed)
                return;
            // As the hit it is: a blast costs 3, a squish 1; none while masked.
            uint32 loss = ram == 'T' ? 1 : 3;
            if (me->maskUntil && int32(now - me->maskUntil) < 0)
                loss = 0;
            me->wumpa = uint8(me->wumpa > loss ? me->wumpa - loss : 0);
            SendRace(*race, "KARTCRAFT\tW\tB\t" + guid + "\t" + std::to_string(low) + "\t" + std::string(1, ram) +
                "\t" + std::to_string(me->wumpa));
        }
        else if (verb == "P" && f.size() >= 3 && f[2].size() == 1)
        {
            // Wumpa lost by no one's hit: a powerslide spun out 1 (VehPhysProc.c:2868-2871), Aku
            // Aku's grab after a fall 2 (VehStuckProc.c:648-651); none while masked. The total goes
            // to the racer alone.
            char why = f[2][0];
            if (why != 'S' && why != 'M')
                return;
            uint32 loss = why == 'M' ? 2 : 1;
            if (me->maskUntil && int32(now - me->maskUntil) < 0)
                loss = 0;
            me->wumpa = uint8(me->wumpa > loss ? me->wumpa - loss : 0);
            SendLine(player, "KARTCRAFT\tW\tF\t" + std::to_string(me->wumpa));
        }
    }

    // A race's records: a time trial's own, or the races'.
    static Board& BoardOf(Race const& r)
    {
        return r.timeTrial ? s_trialBoard : s_raceBoard;
    }

    // Each racer's "R\tRec": the course record and its holder, and their own best (ms, 0 none),
    // on the race's board.
    static void SendRecords(uint32 id, Race const& r)
    {
        Board& board = BoardOf(r);
        Record const& record = board.records[r.course->key];
        for (Racer const& x : r.racers)
            if (Player* p = Recipient(x.guid))
                SendLine(p, "KARTCRAFT\tR\tRec\t" + std::to_string(id) + "\t" + r.course->key + "\t" +
                std::to_string(record.ms) + "\t" + record.name + "\t" +
                std::to_string(BestOf(board, x.guid.GetCounter(), r.course->key)));
    }

    // A course's doors (Course::doors) a race has opened, to close once it is over: the race,
    // and the door by its map, copy and guid.
    struct OpenDoor
    {
        uint32 race;
        uint32 map;
        uint32 instance;
        ObjectGuid door;
    };
    static std::vector<OpenDoor> s_doors;
    // How near the host (yards) a course's door is looked for. A door opened stays open a race's
    // lifetime at most (RACE_LIFETIME), should the close never come.
    constexpr float DOOR_REACH = 150.f;

    // Opens the course's doors nearest the host, from the world's messager (a game object belongs
    // to its map's thread): each stays open until TickDoors sees the race gone.
    static void OpenDoors(uint32 id, Race const& r)
    {
        if (r.course->doors.empty())
            return;
        ObjectGuid const host = r.host;
        std::vector<uint32> const doors = r.course->doors;
        sWorld.GetMessager().AddMessage([id, host, doors](World*)
        {
            Player* p = ObjectAccessor::FindPlayer(host);
            if (!p || !p->IsInWorld())
                return;
            for (uint32 entry : doors)
            {
                GameObject* door = p->FindNearestGameObject(entry, DOOR_REACH);
                if (!door || door->GetGoState() != GO_STATE_READY)
                    continue;
                door->UseDoorOrButton(RACE_LIFETIME / IN_MILLISECONDS);
                std::lock_guard<std::mutex> lock(s_mutex);
                s_doors.push_back({ id, p->GetMapId(), p->GetInstanceId(), door->GetObjectGuid() });
            }
        });
    }

    // Closes the doors of races that are over (run with every line, as CloseLateRaces is).
    static void TickDoors()
    {
        for (auto it = s_doors.begin(); it != s_doors.end();)
        {
            if (s_races.count(it->race))
            {
                it = std::next(it);
                continue;
            }
            OpenDoor const door = *it;
            sWorld.GetMessager().AddMessage([door](World*)
            {
                if (Map* map = sMapMgr.FindMap(door.map, door.instance))
                    if (GameObject* go = map->GetGameObject(door.door))
                        if (go->GetGoState() != GO_STATE_READY)
                            go->ResetDoorOrButton();
            });
            it = s_doors.erase(it);
        }
    }

    // Sends the ground types, the course and the countdown: the race is on once it runs out. A
    // time trial has no crates: its racer starts with nothing and is sent no layout. The course's
    // doors open.
    static void StartRace(uint32 id, Race& r, uint32 now, uint32 countdown)
    {
        r.start = now + countdown;
        r.id = id;
        r.startedAt = uint32(std::time(nullptr));
        r.field = uint8(std::min<size_t>(r.racers.size(), 255));
        OpenDoors(id, r);
        SendRace(r, "KARTCRAFT\tR\tGT\t" + std::to_string(id) + "\t" + (r.groundTypes ? "1" : "0"));
        std::string points;
        for (auto const& p : r.course->points)
            points += (points.empty() ? "" : ";") + std::to_string(int32(p.first)) + ":" + std::to_string(int32(p.second));
        // A player track's name and maker, which the client has no copy of.
        if (UserTrackOf(*r.course))
            SendRace(r, UserTrackRaceLine(*UserTrackOf(*r.course)));
        SendRace(r, "KARTCRAFT\tR\tC\t" + std::to_string(id) + "\t" + r.course->key + "\t" +
            std::to_string(r.course->laps) + "\t" + points);
        SendRace(r, "KARTCRAFT\tR\tG\t" + std::to_string(id) + "\t" + std::to_string(countdown) + "\t" + Roster(r));
        SendRecords(id, r);
        if (!r.timeTrial)
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
    // A track browser time trial: the countdown once its racer is on the grid in the kart, and
    // how long it waits for them at most.
    constexpr uint32 ARRIVED_COUNTDOWN = COUNTDOWN + 1 * IN_MILLISECONDS;
    constexpr uint32 ARRIVE_WAIT = 45 * IN_MILLISECONDS;
    constexpr float GRID_SPACING = 4.f;
    // A Race Finder race's ground types: off, as nobody hosts it to choose.
    constexpr bool FINDER_GROUND_TYPES = false;

    struct Queued
    {
        ObjectGuid guid;
        std::vector<bool> courses; // [i]: CourseAt(i)
        uint32 joined;
        uint32 seen;
        // Queued with their party (its group id): the party pops together or not at all, and
        // leaves the queue together. 0 alone.
        uint32 party = 0;
    };

    struct Pop
    {
        Course const* course = nullptr;
        uint32 created = 0;
        std::vector<Queued> members;
        std::vector<ObjectGuid> accepted;
        // Said yes while they could not be raced off (dead, fighting, flying, in a dungeon...):
        // accepted as soon as they can be, within the pop's time.
        std::vector<ObjectGuid> waiting;
    };

    static std::vector<Queued> s_queue;
    static std::unordered_map<uint32, Pop> s_pops;
    static uint32 s_nextPop = 1;

    // In a ready check: the pop has them, out of the queue, until it launches or ends.
    static bool InPop(ObjectGuid guid)
    {
        for (auto const& entry : s_pops)
            for (Queued const& q : entry.second.members)
                if (q.guid == guid)
                    return true;
        return false;
    }

    // The Race Finder's zones, in the order courses.json first names them: the client asks for a
    // zone by its place in this list ("z3"), which keeps a queue line short however many it ticks.
    // A party course's zone is not in it.
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

    // The courses a queue line asks for: "any", or a comma list of zones ("z<index>") and keys;
    // never a party course.
    static std::vector<bool> CourseMask(std::string const& asked)
    {
        std::vector<bool> mask(CourseCount(), false);
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
            // A player track only by its own key.
            for (UserTrack const& t : s_userTracks)
                if (token == s_userCourses[t.slot].course.key)
                    mask[COURSES.size() + t.slot] = true;
        }
        return mask;
    }

    static bool AnyCourse(std::vector<bool> const& mask)
    {
        return std::find(mask.begin(), mask.end(), true) != mask.end();
    }

    // Whether queue entry `q` takes the course at `i` (CourseAt); a mask made before a player
    // track was published is shorter than the courses are now.
    template <typename Q>
    static bool Wants(Q const& q, size_t i)
    {
        return i < q.courses.size() && q.courses[i];
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
        if (Player* p = Recipient(guid))
            SendLine(p, text);
    }

    // A grid slot behind the start, two abreast, facing down the course.
    // Whether a player may be put in the kart or on a grid where they are: not dead, fighting or
    // already on the move, and not on a flight path, a boat or a zeppelin.
    static bool Settled(Player* player)
    {
        return player->IsAlive() && !player->IsInCombat() && !player->IsBeingTeleported() &&
            !player->IsTaxiFlying() && !player->GetTransport();
    }

    // Whether the Race Finder may take a player off to a race now: settled, and not inside a
    // dungeon, a raid or a battleground's match (a teleport out of those would lose their place).
    static bool CanBeRacedOff(Player* player)
    {
        return Settled(player) && InOpenWorld(player) && !CarryingDelivery(player);
    }

    // Whether a racer may be put on `c`'s grid now: off to it from the open world, or, on a party
    // course, only from inside the race's copy of its dungeon (`instance`): a teleport within one
    // map stays in its copy (Player::TeleportTo's near teleport), so the party never leaves it.
    static DungeonPersistentState* CopyState(uint32 map, uint32 instance);

    static bool CanRaceOn(Player* player, Course const& c, uint32 instance)
    {
        if (!PartyCourse(c))
            return CanBeRacedOff(player);
        // Inside the race's copy already, or off to it from the open world (Race copies).
        if (Settled(player) && player->GetMapId() == c.map && player->GetInstanceId() == instance)
            return true;
        return instance && CopyState(c.map, instance) && CanBeRacedOff(player);
    }

    // Whether `player` may go to a time trial on `c` now: off to it from the open world, as the
    // Race Finder takes them, or, on a party course, from inside its dungeon, whose copy the
    // trial reuses (TrialCopy).
    static bool CanTrialOn(Player* player, Course const& c)
    {
        if (PartyCourse(c) && player->GetMapId() == c.map)
            return Settled(player) && !CarryingDelivery(player);
        return CanBeRacedOff(player);
    }

    // Whether `player` may join `r`: anyone near its start, but a party race only the host's party
    // (one group) in the host's copy of the dungeon; a host with no party races alone.
    static bool MayJoin(Player* player, Race const& r)
    {
        if (!r.instance)
            return true;
        Player* host = ObjectAccessor::FindPlayer(r.host);
        return host && player->GetInstanceId() == r.instance && player->IsInSameRaidWith(host);
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

    // How far over a racer in a dungeon (yards) the ground a teleport lands on is looked for from.
    constexpr float DUNGEON_LOOK_UP = 12.f;

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
    static bool TeleportOnto(Player* player, uint32 map, float x, float y, float o, bool kart, float z)
    {
        // The ground under the highest point looked from: from a little over the floor wanted
        // (`z`, where the course or track says), on any map, so a road under a roof (the Caverns
        // of Time's way down, a dungeon's floors and roofs) is found under it; in a dungeon without
        // one, from over the racer, who is on its streets; else from the sky.
        MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(map);
        bool const dungeon = entry && entry->IsDungeon();
        float const from = z != 0.f ? z + DUNGEON_LOOK_UP :
            dungeon && player->GetMapId() == map ? player->GetPositionZ() + DUNGEON_LOOK_UP : 500.f;
        float ground = sTerrainMgr.LoadTerrain(map)->GetHeightStatic(x, y, from);
        if (ground <= INVALID_HEIGHT)
            return false;
        if (kart)
            player->RemoveAurasDueToSpell(SPELL_SUMMON_KART);
        player->TeleportTo(map, x, y, ground + 0.5f, o);
        if (kart)
            player->m_Events.AddEventAtOffset(new KartAgainEvent(player), KART_AGAIN_DELAY);
        return true;
    }
    // ── Race copies ────────────────────────────────────────────────────────────────────────────
    //
    // A dungeon or raid course (PartyCourse) is raced in a copy of the race's own, made for it
    // (RaceCopy) and entered by anyone the race takes, party or not: each racer is bound to it,
    // not for keeps, just before the grid's teleport (IntoCopy), so the core's own choice of copy
    // (Player::GetBoundInstanceSaveForSelfOrGroup, a player's own bind first) puts every racer in
    // the one copy. A player's time trials on a dungeon reuse their copy while it lives
    // (TrialCopy), so a run after a run counts no new copy against Instance.PerHourLimit. A raid
    // is entered alone, as a dungeon is (kartcraft.patch, MapManager::CanPlayerEnter).

    static std::unordered_map<uint64, uint32> s_trialCopies; // guid low << 32 | map -> instance

    static DungeonPersistentState* CopyState(uint32 map, uint32 instance)
    {
        MapPersistentState* state = instance ? sMapPersistentStateMgr.GetPersistentState(map, instance) : nullptr;
        return state && state->GetMapEntry() && state->GetMapEntry()->IsDungeon()
            ? static_cast<DungeonPersistentState*>(state) : nullptr;
    }

    // A new copy of `c`'s dungeon or raid for a race on it, its instance id; 0 for an open-world
    // course.
    static uint32 RaceCopy(Course const& c)
    {
        MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(c.map);
        if (!PartyCourse(c) || !entry || !entry->IsDungeon())
            return 0;
        uint32 const instance = sMapMgr.GenerateInstanceId();
        return sMapPersistentStateMgr.AddPersistentState(entry, instance, 0, true) ? instance : 0;
    }

    // `player`'s copy of `c`'s dungeon for a time trial: the one they are in, the one their last
    // trial there made while it lives, else a new one.
    static uint32 TrialCopy(Player* player, Course const& c)
    {
        if (!PartyCourse(c))
            return 0;
        if (player->GetMapId() == c.map)
            return player->GetInstanceId();
        uint64 const key = (uint64(player->GetGUIDLow()) << 32) | c.map;
        auto kept = s_trialCopies.find(key);
        if (kept != s_trialCopies.end() && CopyState(c.map, kept->second))
            return kept->second;
        uint32 const instance = RaceCopy(c);
        if (instance)
            s_trialCopies[key] = instance;
        else
            s_trialCopies.erase(key);
        return instance;
    }

    // Bound to copy `instance` of `map`, not for keeps, for the teleport into it to land there.
    static bool IntoCopy(Player* player, uint32 map, uint32 instance)
    {
        DungeonPersistentState* state = CopyState(map, instance);
        if (!state)
            return false;
        player->BindToInstance(state, false);
        return true;
    }


    // `instance`: a party race's (Race::instance), 0 in the open world.
    // Where grid slot `slot` of `c` stands (WoW x, y) and the way it faces: behind the start down
    // the first leg, or on a player track's own grid where it has one.
    static void GridSpot(Course const& c, size_t slot, float& x, float& y, float& o)
    {
        float sx = c.points[0].first, sy = c.points[0].second;
        float dx = c.points[1].first - sx, dy = c.points[1].second - sy;
        float len = std::max(std::sqrt(dx * dx + dy * dy), 1.f);
        dx /= len;
        dy /= len;
        // A player track's own grid: its first slot where its maker put it, facing their way.
        float gx, gy, go;
        UserTrack const* mine = UserTrackOf(c);
        if (mine && std::sscanf(mine->grid.c_str(), "%f,%f,%f", &gx, &gy, &go) == 3)
        {
            dx = std::cos(go);
            dy = std::sin(go);
            sx = gx + dx * GRID_SPACING;
            sy = gy + dy * GRID_SPACING;
        }
        float back = GRID_SPACING * float(slot / 2 + 1);
        float side = GRID_SPACING * (slot % 2 ? 0.5f : -0.5f);
        x = sx - dx * back - dy * side;
        y = sy - dy * back + dx * side;
        o = std::atan2(dy, dx);
    }

    static bool TeleportToGrid(Player* player, Course const& c, size_t slot, uint32 instance)
    {
        // Their situation may have changed since they said yes: then they stay where they are.
        if (!CanRaceOn(player, c, instance))
            return false;
        float x, y, o;
        GridSpot(c, slot, x, y, o);
        // From outside a dungeon course's copy: bound to it, so the teleport lands in it.
        if (PartyCourse(c) && !(player->GetMapId() == c.map && player->GetInstanceId() == instance) &&
            !IntoCopy(player, c.map, instance))
            return false;
        return TeleportOnto(player, c.map, x, y, o, true, c.z);
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

    // Each racer's time trial just finished, its course's key: the way out of it offers another
    // go ("R\tRet\t<course>", or "R\tAgain\t<course>" with no spot to go back to), answered
    // "R\tTA\t<course>". A finished trial is closed at once, so the course is kept here until
    // they leave it (or log in again).
    static std::unordered_map<uint32, std::string> s_lastTrial; // guid low -> course key

    // Out of every race: the client asks whether to go back to the spot the Race Finder or the
    // track browser took them from, and after a time trial offers it again.
    static void OfferReturn(Player* player, uint32 now)
    {
        std::string trial;
        auto last = s_lastTrial.find(player->GetGUIDLow());
        if (last != s_lastTrial.end())
        {
            trial = last->second;
            s_lastTrial.erase(last);
        }
        auto found = s_origins.find(player->GetGUIDLow());
        if (found != s_origins.end() && WorldTimer::getMSTimeDiff(found->second.at, now) > RETURN_KEEP)
        {
            s_origins.erase(found);
            found = s_origins.end();
        }
        if (InAnyRace(player->GetObjectGuid()))
            return;
        if (found != s_origins.end())
            SendLine(player, "KARTCRAFT\tR\tRet" + (trial.empty() ? std::string() : "\t" + trial));
        else if (!trial.empty())
            SendLine(player, "KARTCRAFT\tR\tAgain\t" + trial);
    }

    static void EndPop(uint32 id, Pop& pop, uint32 now)
    {
        // A yes still waiting for its racer to be able to go is a yes.
        auto accepted = [&pop](ObjectGuid guid)
        {
            return std::find(pop.accepted.begin(), pop.accepted.end(), guid) != pop.accepted.end() ||
                std::find(pop.waiting.begin(), pop.waiting.end(), guid) != pop.waiting.end();
        };
        // A party goes back only whole: one who said no takes their party out with them.
        auto whole = [&pop, &accepted](uint32 party)
        {
            return !party || std::all_of(pop.members.begin(), pop.members.end(),
                [&](Queued const& m) { return m.party != party || accepted(m.guid); });
        };
        // Those who said yes go back to the front of the queue, keeping their place.
        for (Queued q : pop.members)
            if (accepted(q.guid) && whole(q.party))
            {
                q.seen = now;
                s_queue.insert(s_queue.begin(), q);
            }
        for (Queued const& q : pop.members)
        {
            SendTo(q.guid, "KARTCRAFT\tR\tPop-\t" + std::to_string(id));
            if (accepted(q.guid) && !whole(q.party))
                SendTo(q.guid, "KARTCRAFT\tR\tQx\tparty");
        }
    }

    // Out of the queue, and their whole party with them ("R\tQx\tparty" to the rest).
    static void LeaveQueue(ObjectGuid guid)
    {
        auto mine = std::find_if(s_queue.begin(), s_queue.end(), [guid](Queued const& q) { return q.guid == guid; });
        if (mine == s_queue.end())
            return;
        uint32 const party = mine->party;
        for (auto it = s_queue.begin(); it != s_queue.end();)
            if (it->guid == guid || (party && it->party == party))
            {
                if (it->guid != guid)
                    SendTo(it->guid, "KARTCRAFT\tR\tQx\tparty");
                it = s_queue.erase(it);
            }
            else
                it = std::next(it);
    }

    static void LaunchPop(Pop& pop, uint32 now)
    {
        uint32 raceId = s_nextRace++;
        Race& r = s_races[raceId];
        r.course = pop.course;
        r.host = pop.members.front().guid;
        r.created = now;
        r.groundTypes = FINDER_GROUND_TYPES;
        // A dungeon course's race is run in a copy of its own (Race copies).
        r.instance = RaceCopy(*pop.course);
        size_t slot = 0;
        bool far = false;
        for (Queued const& q : pop.members)
            if (Player* p = ObjectAccessor::FindPlayer(q.guid))
            {
                far |= p->GetMapId() != pop.course->map;
                r.racers.push_back({ p->GetObjectGuid(), p->GetName() });
                Origin from = { p->GetMapId(), p->GetPositionX(), p->GetPositionY(), p->GetPositionZ(),
                    p->GetOrientation(), now };
                if (TeleportToGrid(p, *pop.course, slot++, r.instance))
                    s_origins[p->GetGUIDLow()] = from;
            }
        SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(raceId) + "\t" + Roster(r));
        StartRace(raceId, r, now, far ? GRID_COUNTDOWN + FAR_SETTLE : GRID_COUNTDOWN);
    }

    // Drops the stale, fails late ready checks and pops the courses that are ready.
    static void UpdateFinder(uint32 now)
    {
        // The stale go, and a stale racer's party with them.
        std::vector<ObjectGuid> stale;
        for (Queued const& q : s_queue)
            if (WorldTimer::getMSTimeDiff(q.seen, now) > QUEUE_STALE || !ObjectAccessor::FindPlayer(q.guid))
                stale.push_back(q.guid);
        for (ObjectGuid const& guid : stale)
            LeaveQueue(guid);
        for (auto it = s_origins.begin(); it != s_origins.end();)
            it = WorldTimer::getMSTimeDiff(it->second.at, now) > RETURN_KEEP ? s_origins.erase(it) : std::next(it);

        for (auto it = s_pops.begin(); it != s_pops.end();)
        {
            Pop& p = it->second;
            // A yes that waited (the "Y" handler) counts once they can be raced off.
            for (auto w = p.waiting.begin(); w != p.waiting.end();)
            {
                Player* racer = ObjectAccessor::FindPlayer(*w);
                if (!racer || !CanBeRacedOff(racer))
                {
                    w = std::next(w);
                    continue;
                }
                if (std::find(p.accepted.begin(), p.accepted.end(), *w) == p.accepted.end())
                    p.accepted.push_back(*w);
                w = p.waiting.erase(w);
            }
            if (p.accepted.size() == p.members.size())
            {
                LaunchPop(p, now);
                it = s_pops.erase(it);
            }
            else if (WorldTimer::getMSTimeDiff(p.created, now) > POP_ANSWER)
            {
                EndPop(it->first, p, now);
                it = s_pops.erase(it);
            }
            else
                it = std::next(it);
        }

        // The course with the most takers that is ready, oldest first.
        std::stable_sort(s_queue.begin(), s_queue.end(), [now](Queued const& a, Queued const& b) {
            return WorldTimer::getMSTimeDiff(a.joined, now) > WorldTimer::getMSTimeDiff(b.joined, now);
        });
        // Among the courses tied for the most takers (every one of them, for racers who ticked a
        // whole zone or "any"), one at random: the first in the list would always win.
        int best = -1;
        size_t bestCount = 0;
        uint32 ties = 0;
        for (size_t i = 0; i < CourseCount(); ++i)
        {
            if (!CourseLive(i) || CourseOff(CourseAt(i)))
                continue;
            std::vector<Queued const*> takers;
            for (Queued const& q : s_queue)
                if (Wants(q, i))
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
        pop.course = &CourseAt(best);
        pop.created = now;
        // Oldest first, a party whole: all of it ticked the course and fits, or none of it.
        std::vector<size_t> take;
        std::set<uint32> parties;
        for (size_t i = 0; i < s_queue.size() && take.size() < MAX_RACERS; ++i)
        {
            Queued const& q = s_queue[i];
            if (!Wants(q, best))
                continue;
            if (!q.party)
            {
                take.push_back(i);
                continue;
            }
            if (!parties.insert(q.party).second)
                continue;
            std::vector<size_t> unit;
            bool all = true;
            for (size_t j = 0; j < s_queue.size(); ++j)
                if (s_queue[j].party == q.party)
                {
                    unit.push_back(j);
                    all = all && Wants(s_queue[j], best);
                }
            if (all && take.size() + unit.size() <= MAX_RACERS)
                take.insert(take.end(), unit.begin(), unit.end());
        }
        // A party that does not fit can leave too few: no race of one.
        if (take.size() < 2)
        {
            s_pops.erase(popId);
            return;
        }
        std::sort(take.begin(), take.end());
        for (size_t i : take)
            pop.members.push_back(s_queue[i]);
        for (auto i = take.rbegin(); i != take.rend(); ++i)
            s_queue.erase(s_queue.begin() + *i);
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
                    if (Wants(q, i) && zones[z] == COURSES[i].zone)
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
            if (r.start || r.racers.empty() || r.timeTrial || r.instance)
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

    // Out of everything: the queue, any ready check (as a no), and every race, waiting, under way
    // or finished; the racers left behind get their new roster.
    static void QuitAll(ObjectGuid guid, uint32 now)
    {
        LeaveQueue(guid);
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
    }

    // The showcase's verb, GM accounts only: "R\tShow" lists the runs, "R\tShow\t<name>" takes us
    // to the run's start and back into the kart, and answers "R\tShow\t<name>" for the client to
    // play it once there; "R\tShow\tstop" answers "R\tShow-". False when the line is not it.
    static bool HandleShow(Player* player, std::vector<std::string> const& f)
    {
        if (f[1] != "Show")
            return false;
        ChatHandler chat(player->GetSession());
        if (player->GetSession()->GetSecurity() < SEC_GAMEMASTER)
        {
            chat.SendSysMessage("Only a GM can play the showcase runs.");
            return true;
        }
        if (f.size() < 3 || f[2].empty())
        {
            chat.SendSysMessage("Showcase runs (/kartshow <name> plays one, /kartshow stop):");
            for (ShowRun const& r : SHOW_RUNS)
                chat.PSendSysMessage("  %s: %s, %s", r.name, r.title, r.note);
            return true;
        }
        if (f[2] == "stop")
        {
            SendLine(player, "KARTCRAFT\tR\tShow-");
            return true;
        }
        ShowRun const* run = nullptr;
        for (ShowRun const& r : SHOW_RUNS)
            if (f[2] == r.name)
                run = &r;
        if (!run)
        {
            chat.PSendSysMessage("No showcase run is named %s: /kartshow lists them.", f[2].c_str());
            return true;
        }
        ObjectGuid const guid = player->GetObjectGuid();
        if (InAnyRace(guid))
        {
            chat.SendSysMessage("Leave the race first (/leaverace).");
            return true;
        }
        // A teleport belongs to the world thread, as a Race Marshal's is; on foot for it, as onto a
        // grid (TeleportOnto), and back in the kart after.
        ShowRun const to = *run;
        sWorld.GetMessager().AddMessage([guid, to](World*)
        {
            Player* p = ObjectAccessor::FindPlayer(guid);
            if (!p || !p->IsInWorld() || !p->IsAlive())
                return;
            p->RemoveAurasDueToSpell(SPELL_SUMMON_KART);
            p->TeleportTo(to.map, to.x, to.y, to.z + 0.5f, to.o);
            p->m_Events.AddEventAtOffset(new KartAgainEvent(p), KART_AGAIN_DELAY);
        });
        SendLine(player, std::string("KARTCRAFT\tR\tShow\t") + to.name);
        return true;
    }

    // A time trial for `player` on `c`: out of the queue, any ready check and every other race
    // first, then a race of one, its roster and the line that marks it a time trial. The caller
    // puts them on the grid and starts it (StartRace). No one joins it, and the Race Finder never
    // lists it.
    static uint32 OpenTimeTrial(Player* player, Course const& c, uint32 instance, uint32 now)
    {
        ObjectGuid const guid = player->GetObjectGuid();
        QuitAll(guid, now);
        uint32 raceId = s_nextRace++;
        Race& r = s_races[raceId];
        r.course = &c;
        r.host = guid;
        r.created = now;
        r.timeTrial = true;
        r.instance = instance;
        r.racers.push_back({ guid, player->GetName() });
        // The roster makes the client's race, the next line marks it a time trial, and the
        // course, countdown and records follow (StartRace).
        SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(raceId) + "\t" + Roster(r));
        SendRace(r, "KARTCRAFT\tR\tTT\t" + std::to_string(raceId));
        return raceId;
    }

    // "R\tMT\t<course>": to the course's Race Marshal from the track browser, a few yards in front
    // of them on the start's side, facing them. Once every MARSHAL_TRIP_COOLDOWN ("R\tMTx\t<secs>"
    // says how long to wait), never in a race under way nor what CanBeRacedOff refuses, and not
    // to a party course's marshal, who stands inside a dungeon. A driver goes out of the kart for
    // the teleport and back into it there, as onto a grid.
    constexpr uint32 MARSHAL_TRIP_COOLDOWN = 30 * IN_MILLISECONDS;
    constexpr float MARSHAL_TRIP_GAP = 3.f; // yards in front of the marshal
    static std::unordered_map<uint32, uint32> s_marshalTrips; // guid low -> when last made (ms)

    static void ToMarshal(Player* player, Course const& c, uint32 now)
    {
        if (PartyCourse(c) || InRace(player) || !CanBeRacedOff(player))
            return;
        auto made = s_marshalTrips.find(player->GetGUIDLow());
        if (made != s_marshalTrips.end() && WorldTimer::getMSTimeDiff(made->second, now) < MARSHAL_TRIP_COOLDOWN)
        {
            uint32 left = MARSHAL_TRIP_COOLDOWN - WorldTimer::getMSTimeDiff(made->second, now);
            SendLine(player, "KARTCRAFT\tR\tMTx\t" + std::to_string((left + 999) / IN_MILLISECONDS));
            return;
        }
        // Toward the start the marshal stands beside; straight on in x where they stand on it.
        float dx = c.points[0].first - c.marshal.first, dy = c.points[0].second - c.marshal.second;
        float len = std::sqrt(dx * dx + dy * dy);
        if (len < 0.5f)
        {
            dx = 1.f;
            dy = 0.f;
            len = 1.f;
        }
        dx /= len;
        dy /= len;
        float x = c.marshal.first + dx * MARSHAL_TRIP_GAP, y = c.marshal.second + dy * MARSHAL_TRIP_GAP;
        if (TeleportOnto(player, c.map, x, y, std::atan2(-dy, -dx), player->HasAura(SPELL_SUMMON_KART), c.z))
            s_marshalTrips[player->GetGUIDLow()] = now;
    }

    // "R\tTG\t<x>\t<y>\t<o>[\t<z>]", the track editor's test lap: onto its grid on our own map, in
    // the kart, as a time trial's grid takes a racer, on the floor at `z` in a dungeon; the lap and
    // its countdown are the client's. Only within EDIT_TRIP_REACH of where we are ("R\tTGf"
    // further), in the open world or inside a dungeon, out of every race and not carrying a
    // delivery, once every EDIT_TRIP_COOLDOWN ("R\tTGx\t<secs>").
    constexpr uint32 EDIT_TRIP_COOLDOWN = 10 * IN_MILLISECONDS;
    constexpr float EDIT_TRIP_REACH = 1000.f;
    static std::unordered_map<uint32, uint32> s_editTrips; // guid low -> when last made (ms)

    static void ToEditorGrid(Player* player, float x, float y, float o, float z, uint32 now)
    {
        // Inside a dungeon too: the trip stays on its map, so in its copy (a near teleport).
        bool const inDungeon = player->GetMap()->IsDungeon() && Settled(player) && !CarryingDelivery(player);
        if (InAnyRace(player->GetObjectGuid()) || !(CanBeRacedOff(player) || inDungeon))
            return;
        float dx = x - player->GetPositionX(), dy = y - player->GetPositionY();
        if (dx * dx + dy * dy > EDIT_TRIP_REACH * EDIT_TRIP_REACH)
        {
            SendLine(player, "KARTCRAFT\tR\tTGf");
            return;
        }
        auto made = s_editTrips.find(player->GetGUIDLow());
        if (made != s_editTrips.end() && WorldTimer::getMSTimeDiff(made->second, now) < EDIT_TRIP_COOLDOWN)
        {
            uint32 left = EDIT_TRIP_COOLDOWN - WorldTimer::getMSTimeDiff(made->second, now);
            SendLine(player, "KARTCRAFT\tR\tTGx\t" + std::to_string((left + 999) / IN_MILLISECONDS));
            return;
        }
        if (TeleportOnto(player, player->GetMapId(), x, y, o, player->HasAura(SPELL_SUMMON_KART), z))
            s_editTrips[player->GetGUIDLow()] = now;
    }

    // The finder's verbs; false when the line is not one of them.
    // "R\tJT\t<quest>", Courier Jobs' take: to the courier of one of today's routes, in front of them
    // and facing them (where they stand, which may be indoors: not the ground from above, as
    // TeleportOnto finds it). The job starts when they take its quest there. Once every
    // JOB_TRIP_COOLDOWN ("R\tJTx\t<secs>" says how long), never in a race nor what CanBeRacedOff
    // refuses, a carrier among them; a driver goes out of the kart for it and back in after.
    constexpr uint32 JOB_TRIP_COOLDOWN = 30 * IN_MILLISECONDS;
    constexpr float JOB_GAP = 2.f; // yards in front of the courier
    static std::unordered_map<uint32, uint32> s_jobTrips; // guid low -> when last made (ms)

    static void TakeJob(Player* player, uint32 quest, uint32 now)
    {
        Delivery const* d = FindDelivery(quest);
        if (!d || InAnyRace(player->GetObjectGuid()) || !CanBeRacedOff(player))
            return;
        std::string const day = ChallengePeriods().first;
        if (!OfferedOn(day, quest, TeamOf(player)))
            return;
        if (JobState(player, *d, day) == 2)
        {
            ChatHandler(player->GetSession()).SendSysMessage("You've made that delivery today: it's back tomorrow.");
            return;
        }
        auto made = s_jobTrips.find(player->GetGUIDLow());
        if (made != s_jobTrips.end() && WorldTimer::getMSTimeDiff(made->second, now) < JOB_TRIP_COOLDOWN)
        {
            uint32 left = JOB_TRIP_COOLDOWN - WorldTimer::getMSTimeDiff(made->second, now);
            SendLine(player, "KARTCRAFT\tR\tJTx\t" + std::to_string((left + 999) / IN_MILLISECONDS));
            return;
        }
        float const x = d->gx + std::cos(d->go) * JOB_GAP, y = d->gy + std::sin(d->go) * JOB_GAP;
        float facing = d->go + 3.14159265f;
        if (facing > 6.2831853f)
            facing -= 6.2831853f;
        bool const kart = player->HasAura(SPELL_SUMMON_KART);
        if (kart)
            player->RemoveAurasDueToSpell(SPELL_SUMMON_KART);
        player->TeleportTo(d->map, x, y, d->gz + 0.5f, facing);
        if (kart)
            player->m_Events.AddEventAtOffset(new KartAgainEvent(player), KART_AGAIN_DELAY);
        s_jobTrips[player->GetGUIDLow()] = now;
    }

    // ── Marathon Delivery ──────────────────────────────────────────────────────────────────────
    //
    // A weekend event, Kartcraft's own: at each start in `kartcraft_marathons` (the world
    // database, unix time; setup/vmangos/marathons.py writes it), a delivery across a continent's
    // towns no faction holds. Its queue opens MARATHON_OPEN before the start ("R\tMarOpen\t<secs>\t
    // <stops>", to everyone, at login too); a player joins or leaves it ("R\tMar+", "R\tMar-",
    // answered "R\tMarQ\t<queued>\t<in>"). At the start everyone queued who may go (in no race,
    // CanBeRacedOff) is put in front of the first town's courier, out of the kart and back in, with
    // the package; MARATHON_COUNTDOWN later ("R\tMarGo\t<secs>") the clock runs. The stops are
    // driven to in order, MARATHON_ARRIVE yards of each courier, the GPS led to the next
    // ("R\tMDel\t<town>\t<par>\t<gone>\t<stop>\t<stops>", the delivery's line without a quest);
    // the first three to the last win MARATHON_PRIZES tokens, every other finisher MARATHON_FINISH
    // ("R\tMarDone\t<place>\t<ms>\t<tokens>"). The package is a delivery's, so its carrier's rules
    // hold (CarryingDelivery): no teleport, flight path or race. MARATHON_LIMIT after the go, or
    // once everyone is in or out, it ends ("R\tMarEnd"), told to the world with its podium.
    // Runs on the world thread (Update), under s_mutex then s_marathonMutex.

    constexpr uint32 MARATHON_OPEN = 10 * MINUTE;         // seconds before the start
    constexpr uint32 MARATHON_COUNTDOWN = 10 * IN_MILLISECONDS;
    constexpr uint32 MARATHON_LIMIT = 90 * MINUTE * IN_MILLISECONDS;
    // How long a runner may be logged out and still be in it: a reconnect's time, not a race's.
    constexpr uint32 MARATHON_OFFLINE = 5 * MINUTE * IN_MILLISECONDS;
    constexpr uint32 MARATHON_TICK = IN_MILLISECONDS;
    constexpr uint32 MARATHON_SCHEDULE_RECHECK = MINUTE * IN_MILLISECONDS;
    constexpr float MARATHON_ARRIVE = 40.f;
    constexpr uint32 MARATHON_STOPS = 4; // the start and three more, where the continent has them
    // Placeholder prizes until the event's own: tokens to the first three, and to every finisher.
    constexpr uint32 MARATHON_PRIZES[] = { 100, 50, 25 };
    constexpr uint32 MARATHON_FINISH = 10;

    struct MarathonRunner
    {
        ObjectGuid guid;
        std::string name;
        size_t stop = 1;     // the next stop's index
        uint8 place = 0;     // 0 not in
        uint32 ms = 0;       // the time in
        bool out = false;    // dropped: the package gone
        uint32 offlineSince = 0; // ms, while logged out (MARATHON_OFFLINE)
    };

    struct Marathon
    {
        enum Phase { NONE, OPEN, COUNTDOWN, RUNNING };
        Phase phase = NONE;
        uint32 starts = 0;   // unix time
        uint32 map = 0;
        std::vector<MarathonTown const*> stops;
        uint32 par = 0;      // seconds, the whole route
        std::vector<ObjectGuid> queue;
        std::vector<MarathonRunner> runners;
        uint32 goAt = 0;     // ms
        uint8 finishers = 0;
        uint32 reminded = 0; // the last reminder's minutes before the start
    };

    static std::mutex s_marathonMutex;
    static Marathon s_marathon;
    static uint32 s_marathonTicked = 0;
    static uint32 s_scheduleChecked = 0;

    static void CreateMarathons()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_marathon_results` ("
            "`starts` INT UNSIGNED NOT NULL COMMENT 'the marathon, unix time', "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'character low guid', "
            "`place` TINYINT UNSIGNED NOT NULL, "
            "`ms` INT UNSIGNED NOT NULL COMMENT 'from the go to the last stop', "
            "`tokens` SMALLINT UNSIGNED NOT NULL, "
            "PRIMARY KEY (`starts`, `guid`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
    }

    static float TownDistance(MarathonTown const& a, MarathonTown const& b)
    {
        return std::hypot(a.x - b.x, a.y - b.y);
    }

    // The route for a start on `map`: up to MARATHON_STOPS of its towns, picked by the start's
    // hash, the first of them first and each next the nearest left; the continents' towns differ,
    // and so does each start's route.
    static std::vector<MarathonTown const*> MarathonRoute(uint32 starts, uint32 map)
    {
        std::vector<MarathonTown const*> towns;
        for (MarathonTown const& t : MARATHON_TOWNS)
            if (t.map == map)
                towns.push_back(&t);
        uint32 h = Mix32(starts);
        for (size_t i = towns.size(); i > 1; --i)
        {
            h = Mix32(h + uint32(i));
            std::swap(towns[i - 1], towns[h % i]);
        }
        if (towns.size() > MARATHON_STOPS)
            towns.resize(MARATHON_STOPS);
        for (size_t i = 1; i + 1 < towns.size(); ++i)
        {
            size_t nearest = i;
            for (size_t j = i + 1; j < towns.size(); ++j)
                if (TownDistance(*towns[i - 1], *towns[j]) < TownDistance(*towns[i - 1], *towns[nearest]))
                    nearest = j;
            std::swap(towns[i], towns[nearest]);
        }
        return towns;
    }

    static std::string MarathonStops(Marathon const& m)
    {
        std::string out;
        for (MarathonTown const* t : m.stops)
            out += (out.empty() ? "" : ";") + std::string(t->name);
        return out;
    }

    static std::string MarathonRouteText(Marathon const& m)
    {
        std::string out;
        for (MarathonTown const* t : m.stops)
            out += (out.empty() ? "" : " -> ") + std::string(t->name);
        return out;
    }

    // A system line to every session in the world (World::SendGlobalMessage).
    static void AnnounceAll(std::string const& text)
    {
        WorldPacket data;
        ChatHandler::BuildChatPacket(data, CHAT_MSG_SYSTEM, text.c_str());
        sWorld.SendGlobalMessage(&data);
    }

    // An addon line to every session in the world, as SendLine is to one.
    static void SendLineToAll(std::string const& text)
    {
        WorldPacket data;
        ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, text.c_str(), LANG_ADDON, CHAT_TAG_NONE,
            ObjectGuid(), "");
        sWorld.SendGlobalMessage(&data);
    }

    static uint32 MarathonSecondsLeft(Marathon const& m)
    {
        uint32 now = uint32(std::time(nullptr));
        return m.starts > now ? m.starts - now : 0;
    }

    static void SendMarathonOpen(Player* player, Marathon const& m)
    {
        SendLine(player, "KARTCRAFT\tR\tMarOpen\t" + std::to_string(MarathonSecondsLeft(m)) + "\t" +
            MarathonStops(m));
        bool in = std::find(m.queue.begin(), m.queue.end(), player->GetObjectGuid()) != m.queue.end();
        SendLine(player, "KARTCRAFT\tR\tMarQ\t" + std::to_string(m.queue.size()) + "\t" + (in ? "1" : "0"));
    }

    static void SendMarathonLeg(Player* player, Marathon const& m, MarathonRunner const& r, uint32 now)
    {
        uint32 gone = m.phase == Marathon::RUNNING ? WorldTimer::getMSTimeDiff(m.goAt, now) / IN_MILLISECONDS : 0;
        SendLine(player, std::string("KARTCRAFT\tR\tMDel\t") + m.stops[r.stop]->name + "\t" +
            std::to_string(m.par) + "\t" + std::to_string(gone) + "\t" + std::to_string(r.stop) + "\t" +
            std::to_string(m.stops.size() - 1));
    }

    static MarathonRunner* FindRunner(Marathon& m, ObjectGuid guid)
    {
        for (MarathonRunner& r : m.runners)
            if (r.guid == guid)
                return &r;
        return nullptr;
    }

    static void GiveTokens(Player* player, uint32 count)
    {
        ItemPosCountVec dest;
        if (count && player->CanStoreNewItem(NULL_BAG, NULL_SLOT, dest, DELIVERY_TOKEN, count) == EQUIP_ERR_OK)
        {
            if (::Item* item = player->StoreNewItem(dest, DELIVERY_TOKEN, true))
                player->SendNewItem(item, count, true, false);
        }
        else if (count)
        {
            // Mailed, not lost: a marathon's prize and a cache's are recorded as paid.
            MailDraft draft("Kartcraft Tokens", "Your bags had no room for these.");
            ItemPrototype const* proto = sObjectMgr.GetItemPrototype(DELIVERY_TOKEN);
            uint32 const stack = proto && proto->GetMaxStackSize() ? proto->GetMaxStackSize() : count;
            for (uint32 left = count, n = 0; left && n < MAX_MAIL_ITEMS; left -= std::min(left, stack), ++n)
                if (::Item* item = ::Item::CreateItem(DELIVERY_TOKEN, std::min(left, stack), player->GetObjectGuid()))
                {
                    item->SaveToDB();
                    draft.AddItem(item);
                }
            draft.SendMailTo(MailReceiver(player), MailSender(player, MAIL_STATIONERY_GM), MAIL_CHECK_MASK_COPIED);
            ChatHandler(player->GetSession()).SendSysMessage(
                "Your bags had no room for the Kartcraft Tokens: they are in your mailbox.");
        }
    }

    // The package off a runner, if they have it.
    static void TakePackage(Player* player)
    {
        // From the bank too, as CarryingDelivery counts it there: one left behind would hold its
        // keeper to the by-kart rule for good.
        if (player->HasItemCount(DELIVERY_PACKAGE, 1, true))
            player->DestroyItemCount(DELIVERY_PACKAGE, 1, true, false, true);
    }

    // The marathon over: everyone still out told, the podium to the world, the start marked run.
    static void EndMarathon(Marathon& m)
    {
        for (MarathonRunner const& r : m.runners)
            if (Player* p = ObjectAccessor::FindPlayer(r.guid))
            {
                if (!r.place)
                    TakePackage(p);
                SendLine(p, "KARTCRAFT\tR\tDel-");
                SendLine(p, "KARTCRAFT\tR\tMarEnd");
            }
        if (m.phase == Marathon::OPEN)
            SendLineToAll("KARTCRAFT\tR\tMarEnd");
        std::vector<MarathonRunner const*> podium;
        for (MarathonRunner const& r : m.runners)
            if (r.place)
                podium.push_back(&r);
        std::sort(podium.begin(), podium.end(),
            [](MarathonRunner const* a, MarathonRunner const* b) { return a->place < b->place; });
        if (!m.runners.empty())
        {
            std::string text = "|cffffd100Marathon Delivery over.|r ";
            if (podium.empty())
                text += "Nobody made it to " + std::string(m.stops.back()->name) + ".";
            for (size_t i = 0; i < podium.size() && i < 3; ++i)
                text += (i ? ", " : "") + std::to_string(i + 1) + ". " + podium[i]->name + " (" +
                    Clock(podium[i]->ms) + ")";
            if (!podium.empty())
                text += "; " + std::to_string(podium.size()) + " of " + std::to_string(m.runners.size()) +
                    " finished.";
            AnnounceAll(text);
        }
        WorldDatabase.PExecute("UPDATE `kartcraft_marathons` SET `done` = 1 WHERE `starts` = %u", m.starts);
        m = Marathon();
    }

    // The route's par: each leg's, as a delivery's (pack.py's delivery_routes).
    // A leg's par is its delivery route's (the GPS way at the bot's pace, pack.py); a leg no
    // route covers, the straight line's.
    static void MarathonPar(Marathon& m)
    {
        m.par = 0;
        for (size_t i = 1; i < m.stops.size(); ++i)
        {
            MarathonTown const& a = *m.stops[i - 1];
            MarathonTown const& b = *m.stops[i];
            uint32 leg = 0;
            for (Delivery const& d : DELIVERIES)
                if (std::string(d.from) == a.name && std::string(d.to) == b.name)
                {
                    leg = d.par;
                    break;
                }
            if (!leg)
                leg = std::max<uint32>(120, uint32(std::ceil(TownDistance(a, b) * 1.5f / 32.f / 30.f)) * 30);
            m.par += leg;
        }
    }

    // Everyone told of the open queue as it is now: the popup, and world chat.
    static void AnnounceMarathonOpen(Marathon const& m)
    {
        SendLineToAll("KARTCRAFT\tR\tMarOpen\t" + std::to_string(MarathonSecondsLeft(m)) + "\t" + MarathonStops(m));
        AnnounceAll("|cffffd100A Marathon Delivery leaves in " + std::to_string((MarathonSecondsLeft(m) + 59) / 60) +
            " minutes:|r " + MarathonRouteText(m) + ". Join it from the popup, or /marathon. The first three in win!");
    }

    // The queue opened for a start at `starts` (unix time) on `map` over `stops`; false, and
    // nothing opened, with fewer than two stops.
    static bool OpenMarathon(Marathon& m, uint32 starts, uint32 map, std::vector<MarathonTown const*> stops)
    {
        if (stops.size() < 2)
            return false;
        m = Marathon();
        m.starts = starts;
        m.map = map;
        m.stops = std::move(stops);
        MarathonPar(m);
        m.phase = Marathon::OPEN;
        m.reminded = MarathonSecondsLeft(m) / MINUTE + 1;
        AnnounceMarathonOpen(m);
        return true;
    }

    // The next start not yet run, from the table, every MARATHON_SCHEDULE_RECHECK: one whose
    // queue's window is past without it having opened (a restart, a pause) is marked run.
    static void MarathonSchedule(Marathon& m, uint32 now)
    {
        if (m.phase != Marathon::NONE ||
            (s_scheduleChecked && WorldTimer::getMSTimeDiff(s_scheduleChecked, now) < MARATHON_SCHEDULE_RECHECK))
            return;
        s_scheduleChecked = now;
        uint32 const unixNow = uint32(std::time(nullptr));
        WorldDatabase.PExecute("UPDATE `kartcraft_marathons` SET `done` = 1 WHERE `done` = 0 AND `starts` < %u", unixNow);
        std::unique_ptr<QueryResult> result = WorldDatabase.PQuery(
            "SELECT `starts`, `map` FROM `kartcraft_marathons` WHERE `done` = 0 AND `starts` >= %u "
            "ORDER BY `starts` LIMIT 1", unixNow);
        if (!result)
            return;
        Field* f = result->Fetch();
        uint32 const starts = f[0].GetUInt32();
        uint32 const map = f[1].GetUInt32();
        if (starts > unixNow + MARATHON_OPEN)
            return;
        if (!OpenMarathon(m, starts, map, MarathonRoute(starts, map)))
            WorldDatabase.PExecute("UPDATE `kartcraft_marathons` SET `done` = 1 WHERE `starts` = %u", starts);
    }

    // The start: everyone queued who may go, to the first town with the package.
    static void StartMarathon(Marathon& m, uint32 now)
    {
        // The queue is shut for everyone; those going hear their countdown next ("R\tMarGo").
        SendLineToAll("KARTCRAFT\tR\tMarShut");
        MarathonTown const& first = *m.stops.front();
        for (ObjectGuid guid : m.queue)
        {
            Player* p = ObjectAccessor::FindPlayer(guid);
            if (!p)
                continue;
            if (InAnyRace(guid) || !CanBeRacedOff(p))
            {
                ChatHandler(p->GetSession()).SendSysMessage(
                    "The Marathon Delivery left without you: you were in a race, a fight, an instance or "
                    "carrying a delivery.");
                SendLine(p, "KARTCRAFT\tR\tMarEnd");
                continue;
            }
            // A Race Finder ready check up: its teleport would take them off the route.
            if (InPop(guid))
            {
                ChatHandler(p->GetSession()).SendSysMessage(
                    "The Marathon Delivery left without you: you were in a Race Finder ready check.");
                SendLine(p, "KARTCRAFT\tR\tMarEnd");
                continue;
            }
            ItemPosCountVec dest;
            if (p->CanStoreNewItem(NULL_BAG, NULL_SLOT, dest, DELIVERY_PACKAGE, 1) != EQUIP_ERR_OK)
            {
                ChatHandler(p->GetSession()).SendSysMessage("The Marathon Delivery left without you: no room for its package.");
                SendLine(p, "KARTCRAFT\tR\tMarEnd");
                continue;
            }
            if (::Item* item = p->StoreNewItem(dest, DELIVERY_PACKAGE, true))
                p->SendNewItem(item, 1, true, false);
            float const x = first.x + std::cos(first.o) * JOB_GAP, y = first.y + std::sin(first.o) * JOB_GAP;
            float facing = first.o + 3.14159265f;
            if (facing > 6.2831853f)
                facing -= 6.2831853f;
            bool const kart = p->HasAura(SPELL_SUMMON_KART);
            if (kart)
                p->RemoveAurasDueToSpell(SPELL_SUMMON_KART);
            p->TeleportTo(first.map, x, y, first.z + 0.5f, facing);
            if (kart)
                p->m_Events.AddEventAtOffset(new KartAgainEvent(p), KART_AGAIN_DELAY);
            MarathonRunner r;
            r.guid = guid;
            r.name = p->GetName();
            m.runners.push_back(r);
            SendLine(p, "KARTCRAFT\tR\tMarGo\t" + std::to_string(MARATHON_COUNTDOWN / IN_MILLISECONDS));
        }
        m.queue.clear();
        if (m.runners.empty())
        {
            SendLineToAll("KARTCRAFT\tR\tMarEnd");
            EndMarathon(m);
            return;
        }
        m.phase = Marathon::COUNTDOWN;
        m.goAt = now + MARATHON_COUNTDOWN;
        AnnounceAll("|cffffd100The Marathon Delivery is off|r with " + std::to_string(m.runners.size()) +
            " couriers: " + MarathonRouteText(m) + ".");
    }

    // A runner at their next stop: on to the one after, or in.
    static void MarathonArrive(Marathon& m, MarathonRunner& r, Player* p, uint32 now)
    {
        if (r.stop + 1 < m.stops.size())
        {
            ++r.stop;
            ChatHandler(p->GetSession()).PSendSysMessage("Marathon: %s reached. On to %s.",
                m.stops[r.stop - 1]->name, m.stops[r.stop]->name);
            SendMarathonLeg(p, m, r, now);
            return;
        }
        r.place = ++m.finishers;
        r.ms = WorldTimer::getMSTimeDiff(m.goAt, now);
        uint32 const tokens = r.place <= 3 ? MARATHON_PRIZES[r.place - 1] : MARATHON_FINISH;
        TakePackage(p);
        GiveTokens(p, tokens);
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_marathon_results` (`starts`, `guid`, `place`, `ms`, `tokens`) "
            "VALUES (%u, %u, %u, %u, %u)", m.starts, p->GetGUIDLow(), uint32(r.place), r.ms, tokens);
        SendLine(p, "KARTCRAFT\tR\tDel-");
        SendLine(p, "KARTCRAFT\tR\tMarDone\t" + std::to_string(r.place) + "\t" + std::to_string(r.ms) + "\t" +
            std::to_string(tokens));
        ChatHandler(p->GetSession()).PSendSysMessage("Marathon Delivery: in %u%s in %s, %u tokens!", uint32(r.place),
            r.place == 1 ? "st" : r.place == 2 ? "nd" : r.place == 3 ? "rd" : "th", Clock(r.ms).c_str(), tokens);
        if (r.place <= 3)
            AnnounceAll("|cffffd100Marathon Delivery:|r " + r.name + " is in " + std::to_string(r.place) +
                (r.place == 1 ? "st" : r.place == 2 ? "nd" : "rd") + ", in " + Clock(r.ms) + ".");
    }

    // GM orders (HandleMarathon's "MarGM"), run by the tick on the world thread, where a
    // marathon's teleports and packages belong.
    static std::vector<std::function<void(Marathon&, uint32)>> s_marathonOrders;

    static void MarathonTick(Marathon& m, uint32 now)
    {
        for (auto& order : std::exchange(s_marathonOrders, {}))
            order(m, now);
        MarathonSchedule(m, now);
        switch (m.phase)
        {
            case Marathon::NONE:
                return;
            case Marathon::OPEN:
            {
                uint32 left = MarathonSecondsLeft(m);
                if (!left)
                {
                    StartMarathon(m, now);
                    return;
                }
                // Reminders at five minutes and one.
                for (uint32 at : { 5u, 1u })
                    if (left <= at * MINUTE && m.reminded > at)
                    {
                        m.reminded = at;
                        SendLineToAll("KARTCRAFT\tR\tMarOpen\t" + std::to_string(left) + "\t" + MarathonStops(m));
                        AnnounceAll("|cffffd100The Marathon Delivery leaves in " + std::to_string(at) +
                            (at == 1 ? " minute" : " minutes") + "|r (" + std::to_string(m.queue.size()) +
                            " queued). /marathon to join.");
                    }
                return;
            }
            case Marathon::COUNTDOWN:
                if (int32(now - m.goAt) < 0)
                    return;
                m.phase = Marathon::RUNNING;
                for (MarathonRunner const& r : m.runners)
                    if (Player* p = ObjectAccessor::FindPlayer(r.guid))
                        SendMarathonLeg(p, m, r, now);
                return;
            case Marathon::RUNNING:
                break;
        }
        bool going = false;
        for (MarathonRunner& r : m.runners)
        {
            if (r.place || r.out)
                continue;
            Player* p = ObjectAccessor::FindPlayer(r.guid);
            if (!p)
            {
                // Logged out: their package and the marathon wait for them, MARATHON_OFFLINE at
                // most, so one gone for good does not hold everyone's podium to the limit. Their
                // login takes the package (MarathonLogin).
                if (!r.offlineSince)
                    r.offlineSince = now ? now : 1;
                if (WorldTimer::getMSTimeDiff(r.offlineSince, now) >= MARATHON_OFFLINE)
                    r.out = true;
                else
                    going = true;
                continue;
            }
            r.offlineSince = 0;
            if (!p->IsInWorld() || p->IsBeingTeleported())
            {
                going = true;
                continue;
            }
            if (!p->HasItemCount(DELIVERY_PACKAGE, 1, true))
            {
                r.out = true;
                ChatHandler(p->GetSession()).SendSysMessage("Marathon Delivery: the package is gone, and you with it.");
                SendLine(p, "KARTCRAFT\tR\tDel-");
                SendLine(p, "KARTCRAFT\tR\tMarEnd");
                continue;
            }
            going = true;
            MarathonTown const& next = *m.stops[r.stop];
            if (p->GetMapId() == next.map &&
                std::hypot(p->GetPositionX() - next.x, p->GetPositionY() - next.y) <= MARATHON_ARRIVE)
                MarathonArrive(m, r, p, now);
        }
        bool anyLeft = false;
        for (MarathonRunner const& r : m.runners)
            anyLeft |= !r.place && !r.out;
        if (!going || !anyLeft || WorldTimer::getMSTimeDiff(m.goAt, now) >= MARATHON_LIMIT)
            EndMarathon(m);
    }

    // Who is in the hub's sanctuary now, and their shields with it. On the world thread.
    static void HubTick(uint32 now)
    {
        std::set<ObjectGuid> here;
        if (Map* map = sMapMgr.FindMap(HUB_MAP, 0))
        {
            Map::PlayerList const& players = map->GetPlayers();
            for (auto itr = players.begin(); itr != players.end(); ++itr)
                if (Player* p = itr->getSource())
                    if (p->IsInWorld() && p->IsAlive() && p->GetDistance2d(HUB_X, HUB_Y) < HUB_RADIUS)
                        here.insert(p->GetObjectGuid());
        }
        std::lock_guard<std::mutex> lock(s_mutex);
        if (here == s_hubbed && here.empty())
            return;
        s_hubbed = here;
        TickShields(now);
    }

    // ── Race scale ─────────────────────────────────────────────────────────────────────────────
    //
    // A course's scale (Course::scale) sizes its racers: each one's body, as everyone sees it, is
    // its own scale times the course's while they are in a race on it, and its own again once they
    // are out of it (finished and gone, left, or their race over). The client sizes the kart, its
    // physics and the driver in its seat from the same scale. Every SCALE_TICK, under s_mutex.
    constexpr uint32 SCALE_TICK = 500;
    static uint32 s_scaleTicked = 0;

    static void TickRaceScales()
    {
        std::unordered_map<uint64, float> want;
        for (auto const& entry : s_races)
            if (entry.second.course && entry.second.course->scale != 1.f)
                for (Racer const& r : entry.second.racers)
                    want[r.guid.GetRawValue()] = entry.second.course->scale;
        // A test lap's, while its driver is still in the kart; out of it, it is over.
        for (auto it = s_testLapScale.begin(); it != s_testLapScale.end();)
        {
            Player* player = ObjectAccessor::FindPlayer(ObjectGuid(it->first));
            if (!player || !player->HasAura(SPELL_SUMMON_KART))
            {
                it = s_testLapScale.erase(it);
                continue;
            }
            want.emplace(it->first, it->second);
            ++it;
        }
        std::lock_guard<std::mutex> lock(s_raceScaleMutex);
        for (auto const& w : want)
        {
            Player* player = ObjectAccessor::FindPlayer(ObjectGuid(w.first));
            if (!player || !player->IsInWorld())
                continue;
            auto base = s_raceScaleBase.emplace(w.first, player->GetObjectScale()).first;
            float const target = base->second * w.second;
            if (std::fabs(player->GetObjectScale() - target) > 0.001f)
                player->SetObjectScale(target);
        }
        for (auto it = s_raceScaleBase.begin(); it != s_raceScaleBase.end();)
        {
            if (want.count(it->first))
            {
                ++it;
                continue;
            }
            if (Player* player = ObjectAccessor::FindPlayer(ObjectGuid(it->first)))
                player->SetObjectScale(it->second);
            it = s_raceScaleBase.erase(it);
        }
    }

    void Update(uint32 /*diff*/)
    {
        uint32 const now = WorldTimer::getMSTime();
        {
            std::lock_guard<std::mutex> races(s_mutex);
            SampleReplays(now);
            FinishTakes();
            if (!s_scaleTicked || WorldTimer::getMSTimeDiff(s_scaleTicked, now) >= SCALE_TICK)
            {
                s_scaleTicked = now;
                TickRaceScales();
            }
            // Today's and this week's courses chosen as the period turns, nobody racing or not,
            // so the website shows them from the first minute.
            ChallengesNow();
        }
        if (!s_replaySwept || WorldTimer::getMSTimeDiff(s_replaySwept, now) >= REPLAY_SWEEP)
        {
            s_replaySwept = now;
            SweepReplays();
        }
        if (!s_hubTicked || WorldTimer::getMSTimeDiff(s_hubTicked, now) >= HUB_TICK)
        {
            s_hubTicked = now;
            HubTick(now);
        }
        if (s_marathonTicked && WorldTimer::getMSTimeDiff(s_marathonTicked, now) < MARATHON_TICK)
            return;
        s_marathonTicked = now;
        std::lock_guard<std::mutex> races(s_mutex);
        std::lock_guard<std::mutex> lock(s_marathonMutex);
        MarathonTick(s_marathon, now);
    }

    // Whether `a` and `b` are the same word, any case.
    static bool SameWord(std::string const& a, std::string const& b)
    {
        return a.size() == b.size() && std::equal(a.begin(), a.end(), b.begin(), [](char x, char y)
        {
            return std::tolower(static_cast<unsigned char>(x)) == std::tolower(static_cast<unsigned char>(y));
        });
    }

    // A neutral town by name, any case.
    static MarathonTown const* FindMarathonTown(std::string const& name)
    {
        for (MarathonTown const& t : MARATHON_TOWNS)
            if (SameWord(t.name, name))
                return &t;
        return nullptr;
    }

    // "ek" or "kalimdor" (any case, or 0 / 1): the continent; false for neither.
    static bool MarathonMap(std::string const& word, uint32& map)
    {
        if (SameWord(word, "ek") || SameWord(word, "eastern") || word == "0")
            map = 0;
        else if (SameWord(word, "kalimdor") || SameWord(word, "kal") || word == "1")
            map = 1;
        else
            return false;
        return true;
    }

    // A word as minutes (0 to a day), else `fallback`.
    static uint32 MarathonMinutes(std::vector<std::string> const& f, size_t i, uint32 fallback)
    {
        if (f.size() <= i || f[i].empty() || !std::isdigit(static_cast<unsigned char>(f[i][0])))
            return fallback;
        return std::min<uint32>(uint32(std::strtoul(f[i].c_str(), nullptr, 10)), 24 * 60);
    }

    // "R\tMarGM\t<order>\t<args...>", a GM's (SEC_GAMEMASTER) hand on the marathon: status, open
    // [minutes] [ek|kalimdor], go, end, restart [minutes], route <town;town;...>, map
    // <ek|kalimdor>, delay <minutes>. Queued for the tick, which answers the GM.
    static void MarathonOrder(Player* player, std::vector<std::string> const& f)
    {
        ChatHandler chat(player->GetSession());
        if (player->GetSession()->GetSecurity() < SEC_GAMEMASTER)
        {
            chat.SendSysMessage("Only a GM can run the Marathon Delivery by hand.");
            return;
        }
        std::string const order = f.size() > 2 ? f[2] : "status";
        ObjectGuid const gm = player->GetObjectGuid();
        auto tell = [gm](std::string const& text)
        {
            if (Player* p = ObjectAccessor::FindPlayer(gm))
                ChatHandler(p->GetSession()).SendSysMessage(("Marathon: " + text).c_str());
        };
        uint32 const minutes = MarathonMinutes(f, 3, 10);
        if (order == "status")
            s_marathonOrders.push_back([tell](Marathon& m, uint32 /*now*/)
            {
                char const* phase[] = { "none", "queue open", "counting down", "running" };
                std::string text = std::string(phase[m.phase]);
                if (m.phase != Marathon::NONE)
                    text += ", " + MarathonRouteText(m) + " (par " + std::to_string(m.par / 60) + " min)";
                if (m.phase == Marathon::OPEN)
                    text += ", leaves in " + std::to_string(MarathonSecondsLeft(m)) + " s, " +
                        std::to_string(m.queue.size()) + " queued";
                if (m.phase == Marathon::RUNNING || m.phase == Marathon::COUNTDOWN)
                    text += ", " + std::to_string(m.runners.size()) + " couriers, " +
                        std::to_string(m.finishers) + " in";
                tell(text + ".");
            });
        else if (order == "open" || order == "restart")
        {
            bool const restart = order == "restart";
            uint32 map = 2;
            if (f.size() > 4)
                MarathonMap(f[4], map);
            else if (f.size() > 3 && !std::isdigit(static_cast<unsigned char>(f[3][0])))
                MarathonMap(f[3], map);
            s_marathonOrders.push_back([tell, restart, minutes, map](Marathon& m, uint32 /*now*/)
            {
                if (m.phase != Marathon::NONE && !restart)
                {
                    tell("one is on already: end or restart it first.");
                    return;
                }
                uint32 on = map < 2 ? map : m.phase != Marathon::NONE ? m.map : uint32(std::time(nullptr) / 60) % 2;
                if (m.phase != Marathon::NONE)
                    EndMarathon(m);
                uint32 const starts = uint32(std::time(nullptr)) + minutes * MINUTE;
                if (OpenMarathon(m, starts, on, MarathonRoute(starts, on)))
                    tell("opened: " + MarathonRouteText(m) + ", leaving in " + std::to_string(minutes) + " min.");
                else
                    tell("that continent has too few towns.");
            });
        }
        else if (order == "go")
            s_marathonOrders.push_back([tell](Marathon& m, uint32 /*now*/)
            {
                if (m.phase != Marathon::OPEN)
                {
                    tell("no queue is open.");
                    return;
                }
                m.starts = uint32(std::time(nullptr));
                tell("leaving now with " + std::to_string(m.queue.size()) + " queued.");
            });
        else if (order == "end")
            s_marathonOrders.push_back([tell](Marathon& m, uint32 /*now*/)
            {
                if (m.phase == Marathon::NONE)
                {
                    tell("none is on.");
                    return;
                }
                if (m.phase == Marathon::OPEN)
                    SendLineToAll("KARTCRAFT\tR\tMarEnd");
                EndMarathon(m);
                AnnounceAll("|cffffd100The Marathon Delivery was called off.|r");
                tell("ended.");
            });
        else if (order == "route" || order == "map" || order == "delay")
        {
            std::string args;
            for (size_t i = 3; i < f.size(); ++i)
                args += (i > 3 ? " " : "") + f[i];
            s_marathonOrders.push_back([tell, order, args, minutes](Marathon& m, uint32 now)
            {
                if (m.phase != Marathon::OPEN)
                {
                    tell("only an open queue's marathon can be changed: open one first.");
                    return;
                }
                if (order == "delay")
                    m.starts = uint32(std::time(nullptr)) + minutes * MINUTE;
                else if (order == "map")
                {
                    uint32 map;
                    if (!MarathonMap(args, map))
                    {
                        tell("which continent? ek or kalimdor.");
                        return;
                    }
                    std::vector<MarathonTown const*> stops = MarathonRoute(m.starts + now, map);
                    if (stops.size() < 2)
                    {
                        tell("that continent has too few towns.");
                        return;
                    }
                    m.map = map;
                    m.stops = std::move(stops);
                }
                else
                {
                    std::vector<MarathonTown const*> stops;
                    size_t from = 0;
                    while (from <= args.size())
                    {
                        size_t to = args.find(';', from);
                        std::string name = args.substr(from, to == std::string::npos ? std::string::npos : to - from);
                        while (!name.empty() && name.front() == ' ')
                            name.erase(name.begin());
                        while (!name.empty() && name.back() == ' ')
                            name.pop_back();
                        if (!name.empty())
                        {
                            MarathonTown const* t = FindMarathonTown(name);
                            if (!t || (!stops.empty() && t->map != stops.front()->map))
                            {
                                tell("no neutral town \"" + name + "\" on the route's continent.");
                                return;
                            }
                            stops.push_back(t);
                        }
                        if (to == std::string::npos)
                            break;
                        from = to + 1;
                    }
                    if (stops.size() < 2)
                    {
                        tell("a route needs two towns or more, separated by ';'.");
                        return;
                    }
                    m.map = stops.front()->map;
                    m.stops = std::move(stops);
                }
                MarathonPar(m);
                m.reminded = MarathonSecondsLeft(m) / MINUTE + 1;
                AnnounceMarathonOpen(m);
                tell("changed: " + MarathonRouteText(m) + ", leaving in " +
                    std::to_string((MarathonSecondsLeft(m) + 59) / 60) + " min.");
            });
        }
        else
            chat.SendSysMessage("Marathon orders: status, open [minutes] [ek|kalimdor], go, end, restart [minutes], "
                "route <town;town;...>, map <ek|kalimdor>, delay <minutes>.");
    }

    // "R\tMar+", "R\tMar-": the queue joined or left while it is open; "R\tMarGM": a GM's order.
    // False when the line is none of them. Under s_mutex (HandleLine).
    static bool HandleMarathon(Player* player, std::vector<std::string> const& f)
    {
        std::string const& verb = f[1];
        if (verb == "MarGM")
        {
            std::lock_guard<std::mutex> lock(s_marathonMutex);
            MarathonOrder(player, f);
            return true;
        }
        if (verb != "Mar+" && verb != "Mar-")
            return false;
        std::lock_guard<std::mutex> lock(s_marathonMutex);
        Marathon& m = s_marathon;
        if (m.phase != Marathon::OPEN)
        {
            ChatHandler(player->GetSession()).SendSysMessage("No Marathon Delivery is taking couriers now.");
            return true;
        }
        ObjectGuid const guid = player->GetObjectGuid();
        auto in = std::find(m.queue.begin(), m.queue.end(), guid);
        if (verb == "Mar+" && in == m.queue.end())
        {
            if (CarryingDelivery(player))
            {
                ChatHandler(player->GetSession()).SendSysMessage("Deliver the package you're carrying first.");
                return true;
            }
            m.queue.push_back(guid);
        }
        else if (verb == "Mar-" && in != m.queue.end())
            m.queue.erase(in);
        bool const queued = std::find(m.queue.begin(), m.queue.end(), guid) != m.queue.end();
        SendLine(player, "KARTCRAFT\tR\tMarQ\t" + std::to_string(m.queue.size()) + "\t" + (queued ? "1" : "0"));
        return true;
    }

    // At login: the open queue's popup, a runner's leg; and a package nothing explains taken (the
    // marathon ended, or the server restarted, while its carrier was away).
    static void MarathonLogin(Player* player)
    {
        std::lock_guard<std::mutex> lock(s_marathonMutex);
        Marathon& m = s_marathon;
        MarathonRunner const* r = FindRunner(m, player->GetObjectGuid());
        bool const running = r && !r->place && !r->out && m.phase != Marathon::NONE;
        if (m.phase == Marathon::OPEN)
            SendMarathonOpen(player, m);
        if (running && m.phase == Marathon::RUNNING)
            SendMarathonLeg(player, m, *r, WorldTimer::getMSTime());
        if (running || !player->HasItemCount(DELIVERY_PACKAGE, 1, true))
            return;
        // A courier delivery's own package: one of its quests in the log.
        for (Delivery const& d : DELIVERIES)
            if (player->GetQuestStatus(d.quest) != QUEST_STATUS_NONE)
                return;
        TakePackage(player);
    }

    // ── Dungeon teleporter ───────────────────────────────────────────────────────────────────────
    //
    // Any player, of any level, to any dungeon, raid or battleground, from the Kartcraft menu: to
    // build a track there, or race one. "R\tDL" asks for the list, answered
    // "R\tDLs\t<map>:<name>:<level>;..." (the entrance's level, its trigger's, as the client shows
    // it; a battleground's its template's); "R\tDT\t<map>" goes, to where its entrance puts a
    // player inside (its area trigger's spot, ObjectMgr::GetMapEntranceTrigger; a battleground's
    // start for the player's side, in its open copy; NO_ENTRANCE's for a map none leads into), in
    // their own copy or their party's
    // as an entrance would, race mode on (nobody can attack them, nor they anyone: a low level in
    // Stratholme stays alive); answered "R\tDTok\t<name>", or "R\tDTx\t<why>". "R\tDB" takes them
    // back to where the first trip took them from ("R\tDBok", or "R\tDTx\t<why>" with none to go
    // back from, a relog's client not knowing). No level check stands in the way:
    // the core asks one only at an entrance, not of a teleport (MapManager::CanPlayerEnter).
    constexpr uint32 DUNGEON_TRIP_COOLDOWN = 5 * IN_MILLISECONDS;
    static std::unordered_map<uint32, Origin> s_dungeonOrigins; // guid low -> where from
    static std::unordered_map<uint32, uint32> s_dungeonTrips;   // guid low -> last trip (ms)

    // The maps the teleporter goes to that no entrance leads into: where a trip lands (WoW x, y,
    // facing; on the ground there) and the level it is listed at. The Emerald Dream, 1.12's
    // unfinished raid: grass at its spot, in the 256 tiles of EmeraldDream.wdt.
    struct Unentered
    {
        uint32 map;
        float x, y, o;
        uint32 level;
    };
    static Unentered const NO_ENTRANCE[] = { { 169, 2732.93f, -3319.63f, 0.f, 60 } };

    // A battleground's template, its levels and its sides' starts; nullptr for another map.
    static BattleGround* BattlegroundOf(uint32 map)
    {
        BattleGroundTypeId const type = GetBattleGroundTypeIdByMapId(map);
        return type == BATTLEGROUND_TYPE_NONE ? nullptr : sBattleGroundMgr.GetBattleGroundTemplate(type);
    }

    // The level the teleporter lists `map` at; false for a map it does not go to.
    static bool TeleporterLevel(uint32 map, uint32& level)
    {
        MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(map);
        if (entry && entry->IsBattleGround())
        {
            BattleGround const* bg = BattlegroundOf(map);
            if (bg)
                level = bg->GetMinLevel();
            return bg != nullptr;
        }
        if (!entry || !entry->IsDungeon())
            return false;
        if (AreaTriggerTeleport const* at = sObjectMgr.GetMapEntranceTrigger(map))
        {
            level = at->requiredLevel;
            return true;
        }
        for (Unentered const& u : NO_ENTRANCE)
            if (u.map == map)
            {
                level = u.level;
                return true;
            }
        return false;
    }

    // Where `player`'s teleporter trip to `map` lands: its entrance trigger's spot, a
    // battleground's start for their side, else NO_ENTRANCE's on the ground; false with no ground
    // there.
    static bool TeleporterSpot(Player* player, uint32 map, WorldLocation& to)
    {
        if (BattleGround const* bg = BattlegroundOf(map))
        {
            to.mapId = map;
            bg->GetTeamStartLoc(player->GetTeam(), to.x, to.y, to.z, to.o);
            return true;
        }
        if (AreaTriggerTeleport const* at = sObjectMgr.GetMapEntranceTrigger(map))
        {
            to = at->destination;
            return true;
        }
        for (Unentered const& u : NO_ENTRANCE)
            if (u.map == map)
            {
                float const ground = sTerrainMgr.LoadTerrain(map)->GetHeightStatic(u.x, u.y, 500.f);
                if (ground <= INVALID_HEIGHT)
                    return false;
                to = WorldLocation(map, u.x, u.y, ground + 0.5f, u.o);
                return true;
            }
        return false;
    }

    // The dungeons, raids and battlegrounds the teleporter offers, by entrance level then name,
    // once.
    static std::string const& DungeonList()
    {
        static std::string const list = []
        {
            std::vector<std::tuple<uint32, std::string, uint32>> found;
            for (uint32 id = 2; id < 1000; ++id)
            {
                MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(id);
                uint32 level = 0;
                if (entry && entry->name && *entry->name && TeleporterLevel(id, level))
                    found.emplace_back(level, entry->name, id);
            }
            std::sort(found.begin(), found.end());
            std::string out;
            for (auto const& d : found)
            {
                std::string name = std::get<1>(d);
                std::replace(name.begin(), name.end(), ':', ' ');
                std::replace(name.begin(), name.end(), ';', ' ');
                out += (out.empty() ? "" : ";") + std::to_string(std::get<2>(d)) + ":" + name + ":" +
                    std::to_string(std::get<0>(d));
            }
            return out;
        }();
        return list;
    }

    static void ToDungeon(Player* player, uint32 map, uint32 now)
    {
        auto no = [player](char const* why) { SendLine(player, std::string("KARTCRAFT\tR\tDTx\t") + why); };
        MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(map);
        uint32 level = 0;
        WorldLocation to;
        if (!TeleporterLevel(map, level))
            return no("That is no place the teleporter goes to.");
        if (!TeleporterSpot(player, map, to))
            return no("There is no ground to land on there.");
        if (InAnyRace(player->GetObjectGuid()))
            return no("Leave the race first.");
        if (!Settled(player) || player->InBattleGround() || CarryingDelivery(player))
            return no("Not now: be out of combat, off any flight or boat, and not carrying a delivery.");
        uint32 const guid = player->GetGUIDLow();
        auto last = s_dungeonTrips.find(guid);
        if (last != s_dungeonTrips.end() && WorldTimer::getMSTimeDiff(last->second, now) < DUNGEON_TRIP_COOLDOWN)
            return no("Wait a few seconds between trips.");
        s_dungeonTrips[guid] = now;
        // The first trip's start is the way back: from one dungeon to another it stays.
        if (!s_dungeonOrigins.count(guid))
            s_dungeonOrigins[guid] = { player->GetMapId(), player->GetPositionX(), player->GetPositionY(),
                player->GetPositionZ(), player->GetOrientation(), now };
        RaceMode(player, true);
        ObjectGuid const g = player->GetObjectGuid();
        sWorld.GetMessager().AddMessage([g, to](World*)
        {
            if (Player* p = ObjectAccessor::FindPlayer(g))
                if (p->IsInWorld() && Settled(p))
                    p->TeleportTo(to);
        });
        SendLine(player, std::string("KARTCRAFT\tR\tDTok\t") + entry->name);
    }

    static void FromDungeon(Player* player)
    {
        auto found = s_dungeonOrigins.find(player->GetGUIDLow());
        if (found == s_dungeonOrigins.end())
            return SendLine(player, "KARTCRAFT\tR\tDTx\tNo teleporter trip to go back from.");
        if (InAnyRace(player->GetObjectGuid()))
            return SendLine(player, "KARTCRAFT\tR\tDTx\tLeave the race first.");
        Origin const to = found->second;
        s_dungeonOrigins.erase(found);
        ObjectGuid const g = player->GetObjectGuid();
        sWorld.GetMessager().AddMessage([g, to](World*)
        {
            Player* p = ObjectAccessor::FindPlayer(g);
            if (p && p->IsInWorld() && Settled(p))
                p->TeleportTo(to.map, to.x, to.y, to.z, to.o);
        });
        SendLine(player, "KARTCRAFT\tR\tDBok");
    }

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
            // Not from a dungeon, a raid or a battleground's match: a pop would teleport them out.
            // Told, or their client shows them queued until the heartbeat's answer.
            if (!InOpenWorld(player))
            {
                SendLine(player, "KARTCRAFT\tR\tQx\tworld");
                return true;
            }
            if (!AnyCourse(mask))
            {
                SendLine(player, "KARTCRAFT\tR\tQx\tcourse");
                return true;
            }
            // A tick while their ready check is up: the pop has them, and queueing again could
            // put them in a second one.
            if (InPop(guid))
                return true;
            // A race still waiting to start is left for the queue; one under way is not: a pop
            // would teleport them off the course, so they are told why instead.
            if (!LeaveWaitingRaces(player))
            {
                SendLine(player, "KARTCRAFT\tR\tQx\tracing");
                return true;
            }
            // In a party, the whole party queues: every member who can, for the same courses, as
            // one (Queued::party); each mate's client is told ("R\tQp\t<courses>\t<who>") so it
            // shows the queue and keeps its own place with the heartbeat.
            Group* group = player->GetGroup();
            if (group && group->GetMembersCount() > MAX_RACERS)
            {
                ChatHandler(player->GetSession()).SendSysMessage("A party larger than a race can't queue together.");
                return true;
            }
            uint32 const party = group ? group->GetId() : 0;
            auto enqueue = [&](Player* who)
            {
                ObjectGuid const g = who->GetObjectGuid();
                auto entry = std::find_if(s_queue.begin(), s_queue.end(), [g](Queued const& q) { return q.guid == g; });
                if (entry != s_queue.end())
                {
                    entry->courses = mask;
                    entry->party = party;
                }
                else
                    s_queue.push_back({ g, mask, now, now, party });
            };
            enqueue(player);
            if (group)
                for (GroupReference* itr = group->GetFirstMember(); itr != nullptr; itr = itr->next())
                {
                    Player* mate = itr->getSource();
                    if (!mate || mate == player || !mate->IsInWorld() || !InOpenWorld(mate) ||
                        !LeaveWaitingRaces(mate))
                        continue;
                    enqueue(mate);
                    SendLine(mate, "KARTCRAFT\tR\tQp\t" + f[2] + "\t" + player->GetName());
                }
        }
        else if (verb == "Lobby")
            SendLobby(player);
        else if (verb == "LJ" && f.size() > 2)
        {
            // A hosted race joined from the Race Finder's list, from anywhere: in the kart and on
            // the roster where they are, as a pop's racer; the host's start puts everyone on the
            // grid. Out of the queue and any other race still waiting.
            uint32 id = uint32(std::strtoul(f[2].c_str(), nullptr, 10));
            if (InPop(guid))
            {
                ChatHandler(player->GetSession()).SendSysMessage("Answer the Race Finder's ready check first.");
                return true;
            }
            auto race = s_races.find(id);
            if (race == s_races.end() || race->second.start || race->second.timeTrial ||
                race->second.instance || race->second.racers.size() >= MAX_RACERS || !CanBeRacedOff(player) ||
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
            LeaveQueue(guid);
            SummonKart(player);
            Race& r = race->second;
            r.racers.push_back({ guid, player->GetName() });
            r.racers.back().fromList = true;
            SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(race->first) + "\t" + Roster(r));
        }
        else if (verb == "TB" && f.size() > 2)
        {
            // A time trial from the track browser, from anywhere, as a Race Finder race: a
            // teleport to the course's grid (a far one settles longer) and, after it, the way
            // back ("R\tRet"); a dungeon course's in the player's own copy of the dungeon.
            Course const* course = FindCourse(f[2]);
            if (!course || !CanTrialOn(player, *course) || RefuseOff(player, *course))
                return true;
            // A dungeon course in the player's own copy of it (Race copies).
            uint32 const instance = TrialCopy(player, *course);
            if (PartyCourse(*course) && !instance)
                return true;
            Origin from = { player->GetMapId(), player->GetPositionX(), player->GetPositionY(),
                player->GetPositionZ(), player->GetOrientation(), now };
            uint32 raceId = OpenTimeTrial(player, *course, instance, now);
            if (TeleportToGrid(player, *course, 0, instance))
                s_origins[player->GetGUIDLow()] = from;
            // The countdown waits for them on the grid, however long the teleport takes.
            Race& r = s_races[raceId];
            r.awaiting = guid;
            r.awaitSince = now;
        }
        else if (verb == "TR" && f.size() > 2)
        {
            // Restart a time trial ("R\tTR\t<race>", the restart binding): their time trial
            // goes unrecorded (no retirement, no rating asked), a fresh one opens on its course
            // and its countdown runs once they are back on the grid in the kart, as a track
            // browser trial's does: the teleport takes the kart away for a few seconds, and the
            // lights are the kart's. The spot a track browser trial took them from stays theirs,
            // and nothing offers the way back now: that comes when they leave the last one.
            auto race = s_races.find(uint32(std::strtoul(f[2].c_str(), nullptr, 10)));
            if (race == s_races.end() || !race->second.timeTrial ||
                std::none_of(race->second.racers.begin(), race->second.racers.end(),
                    [guid](Racer const& m) { return m.guid == guid; }))
                return true;
            Course const& course = *race->second.course;
            uint32 const instance = race->second.instance;
            if (!CanRaceOn(player, course, instance) || RefuseOff(player, course))
                return true;
            // Its take too: a restart is no run, and each would stand in Replays unfinished.
            s_takes.erase(race->first);
            s_races.erase(race);
            uint32 raceId = OpenTimeTrial(player, course, instance, now);
            TeleportToGrid(player, course, 0, instance);
            Race& r = s_races[raceId];
            r.awaiting = guid;
            r.awaitSince = now;
        }
        else if (verb == "TA" && f.size() > 2)
        {
            // Another go at the time trial just over ("R\tTA\t<course>", the way out's Retry):
            // its grid again from wherever they are, as "TB" goes, inside its dungeon for a party
            // course. A spot a track browser trial took them from stays theirs; none is taken now.
            Course const* course = FindCourse(f[2]);
            if (!course || InAnyRace(guid) || !CanTrialOn(player, *course) || RefuseOff(player, *course))
                return true;
            // A dungeon course in the player's own copy of it (Race copies), from wherever they are.
            uint32 const instance = TrialCopy(player, *course);
            if (PartyCourse(*course) && !instance)
                return true;
            uint32 raceId = OpenTimeTrial(player, *course, instance, now);
            TeleportToGrid(player, *course, 0, instance);
            // The countdown waits for them on the grid, however long the teleport takes.
            Race& r = s_races[raceId];
            r.awaiting = guid;
            r.awaitSince = now;
        }
        else if (verb == "MT" && f.size() > 2)
        {
            if (Course const* course = FindCourse(f[2]))
                ToMarshal(player, *course, now);
        }
        else if (verb == "DL")
            SendLine(player, "KARTCRAFT\tR\tDLs\t" + DungeonList());
        else if (verb == "DT" && f.size() > 2)
            ToDungeon(player, uint32(std::strtoul(f[2].c_str(), nullptr, 10)), now);
        else if (verb == "DB")
            FromDungeon(player);
        else if (verb == "Sz" && f.size() > 2)
        {
            // "R\tSz\t<scale>": a track editor test lap's scale, the client's alone to run: the
            // body sized as a race on it would size it (TickRaceScales), 1 to give it back.
            float scale = 1.f;
            if (ParseFloat(f[2], scale))
            {
                scale = std::max(MIN_RACE_SCALE, std::min(MAX_RACE_SCALE, scale));
                if (scale == 1.f)
                    s_testLapScale.erase(guid.GetRawValue());
                else
                    s_testLapScale[guid.GetRawValue()] = scale;
            }
        }
        else if (verb == "TG" && f.size() > 4)
        {
            // An optional fifth field: the grid's height, the floor to land on in a dungeon.
            float x, y, o, z = 0.f;
            if (ParseFloat(f[2], x) && ParseFloat(f[3], y) && ParseFloat(f[4], o) &&
                (f.size() < 6 || f[5].empty() || ParseFloat(f[5], z)))
                ToEditorGrid(player, x, y, o, z, now);
        }
        else if (verb == "Q-")
            LeaveQueue(guid);
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
            // waits until they can (UpdateFinder), and they are told so.
            if (!CanBeRacedOff(player))
            {
                if (std::find(p.waiting.begin(), p.waiting.end(), guid) == p.waiting.end())
                    p.waiting.push_back(guid);
                ChatHandler(player->GetSession()).SendSysMessage(
                    "Accepted: the race takes you as soon as you can go (alive, out of the fight, off the "
                    "flight path, out of the dungeon).");
                return true;
            }
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
            // Out of everything (/leaverace, F6). Always answered, so a client sure it is racing
            // when the server is not lets go too; the shields come off with the next tick.
            QuitAll(guid, now);
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
            // choice is, and only for a player settled (alive, not fighting...) out of a
            // battleground's match: from a dungeon course's copy too, which they leave.
            sWorld.GetMessager().AddMessage([guid, to](World*)
            {
                Player* p = ObjectAccessor::FindPlayer(guid);
                if (p && p->IsInWorld() && Settled(p) && !p->InBattleGround() && !CarryingDelivery(p))
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
        // A spot to go back to does not outlive the session, nor another go at a time trial.
        s_origins.erase(guid.GetCounter());
        s_lastTrial.erase(guid.GetCounter());
        // Out of any race's layer, back with everyone.
        TickLayers();
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
        // A racer in a race under way is put back on its course, in a party race inside its copy
        // of the dungeon; anyone else only in the open world.
        Race const* racing = nullptr;
        for (auto const& entry : s_races)
            if (entry.second.start && int32(now - entry.second.start) >= 0)
                for (Racer const& x : entry.second.racers)
                    if (x.guid == player->GetObjectGuid() && !x.finished)
                        racing = &entry.second;
        if (racing ? !CanRaceOn(player, *racing->course, racing->instance) : !CanBeRacedOff(player))
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
                {
                    s_unstuck[player->GetGUIDLow()] = now;
                    CharacterDatabase.PExecute("INSERT INTO `kartcraft_unstuck` (`course`, `guid`, `race_at`, `at`) "
                        "VALUES ('%s', %u, %u, %u)", c.key, player->GetGUIDLow(), r.startedAt,
                        uint32(std::time(nullptr)));
                }
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
        if (TeleportToGrid(player, *nearest, 0, 0))
            s_unstuck[player->GetGUIDLow()] = now;
    }

    // `marshal`: the line comes from a Race Marshal's menu (MarshalSelect), which the player used
    // standing at the start: the start's distance and the kart are not asked for.
    // ── Player tracks ────────────────────────────────────────────────────────────────────────
    //
    // Tracks players make in the client's track editor and publish (`kartcraft_user_tracks`), for
    // every track browser to list and to test-lap on its own client: no races, records or rewards
    // on them. "R\tPub\t<name>\t<map>\t<laps>\t<zone>\t<grid>\t<points>[\t<z>]" publishes one
    // (grid "x,y,o" or empty; points "x,y[,o[,w]];...", the start first, a turned gate's facing and
    // a gate's half-width; `z` its floor height; then optionally its objects,
    // "key,x,y,z,yaw,pitch,roll[,l,w,h];..." the editor's placed objects, and its scale, 0.2 to 3), in
    // the open world or a dungeon, our own of that name replaced: "R\tPubOk\t<id>\t<name>" or
    // "R\tPubNo\t<why>". "R\tPubL" lists them, newest first: "R\tPTB", a "R\tPT\t<id>\t<name>\t<author>
    // \t<map>\t<laps>\t<zone>\t<grid>\t<points>\t<good>\t<bad>\t<z>\t<objects>\t<scale>" each (its
    // votes, its height, objects and scale or empty), then "R\tPTE"; a race on one sends its line
    // so, the votes empty. "R\tPubD\t<id>" takes one of ours down (a GM's, anyone's): "R\tPubGone\t<id>";
    // "R\tPubDN\t<name>" ours of that name, if any. A publish longer than one addon message (the
    // client cuts one at 2047 bytes) comes in pieces, "R\tPubP\t<i>\t<n>\t<text>", its line after
    // "R\t" cut anywhere, 1 to n in order; the last puts it together (PublishPiece).
    constexpr size_t USER_TRACKS_MAX = 2000;    // on the server
    constexpr size_t USER_TRACKS_EACH = 20;     // by one character
    constexpr size_t USER_TRACK_GATES = 60;     // checkpoints on one
    constexpr size_t USER_TRACK_OBJECTS = 250;  // placed objects on one
    constexpr size_t PUBLISH_PIECES = 16;       // pieces of one publish, at most
    // A build piece's size each way (yards): the client's bounds (kart/pieces.rs MIN_SIZE, MAX_SIZE).
    constexpr float PIECE_MIN = 0.5f;
    constexpr float PIECE_MAX = 200.f;
    constexpr size_t USER_TRACKS_LISTED = 100;  // newest, in a list
    constexpr uint32 PUBLISH_COOLDOWN = 20 * IN_MILLISECONDS;
    constexpr uint32 USER_TRACKS_ASK_COOLDOWN = 3 * IN_MILLISECONDS;
    static uint32 s_userTrackNext = 1;
    static std::unordered_map<uint32, uint32> s_published;      // guid low -> last publish (ms)
    static std::unordered_map<uint32, uint32> s_userTracksAsked; // guid low -> last list (ms)

    // A publish coming in pieces: how many, how many so far, and their text.
    struct PublishParts
    {
        size_t n = 0;
        size_t got = 0;
        std::string text;
    };
    static std::unordered_map<uint32, PublishParts> s_publishParts; // guid low -> so far

    // One piece of a publish ("<i>\t<n>\t<text>"): true with `whole` its line once the last is
    // in. A piece out of order, or past PUBLISH_PIECES, drops what came so far.
    static bool PublishPiece(Player* player, std::string const& rest, std::string& whole)
    {
        uint32 const guid = player->GetGUIDLow();
        size_t const a = rest.find('\t');
        size_t const b = a == std::string::npos ? std::string::npos : rest.find('\t', a + 1);
        if (b == std::string::npos)
            return false;
        size_t const i = std::strtoul(rest.substr(0, a).c_str(), nullptr, 10);
        size_t const n = std::strtoul(rest.substr(a + 1, b - a - 1).c_str(), nullptr, 10);
        PublishParts& p = s_publishParts[guid];
        if (i == 1)
            p = PublishParts();
        if (n < 2 || n > PUBLISH_PIECES || i != p.got + 1 || (i > 1 && n != p.n))
        {
            s_publishParts.erase(guid);
            return false;
        }
        p.n = n;
        p.got = i;
        p.text += rest.substr(b + 1);
        if (i < n)
            return false;
        whole = "R\t" + p.text;
        s_publishParts.erase(guid);
        return whole.compare(0, 6, "R\tPub\t") == 0;
    }

    static void LoadUserTracks()
    {
        CharacterDatabase.DirectExecute(
            "CREATE TABLE IF NOT EXISTS `kartcraft_user_tracks` ("
            "`id` INT UNSIGNED NOT NULL, "
            "`guid` INT UNSIGNED NOT NULL COMMENT 'the author, character low guid', "
            "`author` VARCHAR(12) NOT NULL COMMENT 'character name when published', "
            "`name` VARCHAR(40) NOT NULL, "
            "`map` INT UNSIGNED NOT NULL, "
            "`laps` TINYINT UNSIGNED NOT NULL COMMENT '0 a sprint', "
            "`zone` VARCHAR(40) NOT NULL DEFAULT '', "
            "`grid` VARCHAR(64) NOT NULL DEFAULT '' COMMENT 'x,y,o, or empty for none', "
            "`points` TEXT NOT NULL COMMENT 'x,y[,o[,w]];... the start first', "
            "`at` INT UNSIGNED NOT NULL COMMENT 'when published, unix time', "
            "PRIMARY KEY (`id`), UNIQUE KEY `guid_name` (`guid`, `name`)"
            ") ENGINE=InnoDB DEFAULT CHARSET=utf8");
        // A dungeon track's height (TeleportOnto), added to a table made before it.
        CharacterDatabase.DirectExecute("ALTER TABLE `kartcraft_user_tracks` ADD COLUMN IF NOT EXISTS `z` FLOAT "
            "NOT NULL DEFAULT 0 COMMENT 'the track''s floor height, 0 for none'");
        // Its placed objects (the track editor's models), added to a table made before them.
        CharacterDatabase.DirectExecute("ALTER TABLE `kartcraft_user_tracks` ADD COLUMN IF NOT EXISTS "
            "`objects` TEXT NOT NULL DEFAULT '' COMMENT 'key,x,y,z,yaw,pitch,roll;... or empty'");
        // Its scale (Course::scale), added to a table made before it.
        CharacterDatabase.DirectExecute("ALTER TABLE `kartcraft_user_tracks` ADD COLUMN IF NOT EXISTS "
            "`scale` FLOAT NOT NULL DEFAULT 1 COMMENT 'the karts'' and racers'' size, 1 their own'");
        std::unique_ptr<QueryResult> result = CharacterDatabase.Query(
            "SELECT `id`, `guid`, `author`, `name`, `map`, `laps`, `zone`, `grid`, `points`, `z`, `objects`, `scale` "
            "FROM `kartcraft_user_tracks` ORDER BY `at`, `id`");
        if (!result)
            return;
        do
        {
            Field* f = result->Fetch();
            UserTrack t;
            t.id = f[0].GetUInt32();
            t.guid = f[1].GetUInt32();
            t.author = f[2].GetCppString();
            t.name = f[3].GetCppString();
            t.map = f[4].GetUInt32();
            t.laps = f[5].GetUInt32();
            t.zone = f[6].GetCppString();
            t.grid = f[7].GetCppString();
            t.points = f[8].GetCppString();
            t.z = f[9].GetFloat();
            t.objects = f[10].GetCppString();
            t.scale = f[11].GetFloat();
            s_userTrackNext = std::max(s_userTrackNext, t.id + 1);
            MakeUserCourse(t);
            s_userTracks.push_back(t);
        } while (result->NextRow());
        sLog.Out(LOG_BASIC, LOG_LVL_MINIMAL, "Kartcraft: %u player tracks.", uint32(s_userTracks.size()));
    }

    // A name or zone as a list line carries it: printable ASCII, no tabs, quotes, backslashes or
    // the client's `|` escapes, trimmed and cut to `max`.
    static std::string CleanTrackText(std::string const& text, size_t max)
    {
        std::string out;
        for (char c : text)
            if (c >= 0x20 && c < 0x7f && c != '"' && c != '\\' && c != '|' && out.size() < max)
                out += c;
        size_t const first = out.find_first_not_of(' ');
        if (first == std::string::npos)
            return std::string();
        return out.substr(first, out.find_last_not_of(' ') - first + 1);
    }

    // "x,y[,o[,w[,z,pitch,roll]]];..." checked and written back at the wire's decimals (a ring
    // checkpoint all seven: its centre's height and its tilt, which the client hangs it by) (`o` a turned gate's facing,
    // empty where only `w` follows; `w` the gate's half-width, which the client draws and passes
    // by: within CHECKPOINT_RADIUS, the reach a report is taken at); false if any spot is not
    // numbers within the world, or `facing` (the grid's: x,y,o alone) is wanted and missing.
    // `count` its spots.
    static bool CleanTrackSpots(std::string const& text, bool facing, std::string& out, size_t& count)
    {
        out.clear();
        count = 0;
        for (size_t at = 0, end; at < text.size(); at = end + 1)
        {
            end = text.find(';', at);
            if (end == std::string::npos)
                end = text.size();
            std::string const spot = text.substr(at, end - at);
            if (spot.empty())
                continue;
            std::vector<std::string> v;
            for (size_t p = 0, c; p <= spot.size(); p = c + 1)
            {
                c = spot.find(',', p);
                if (c == std::string::npos)
                    c = spot.size();
                v.push_back(spot.substr(p, c - p));
            }
            float x, y, o = 0.f, w = 0.f, z = 0.f, pitch = 0.f, roll = 0.f;
            // A ring checkpoint has all seven: its facing, radius, centre height, pitch and roll.
            bool const ring = !facing && v.size() == 7;
            bool const has_o = v.size() >= 3 && !v[2].empty();
            bool const has_w = (v.size() == 4 || ring) && !v[3].empty();
            if (v.size() < 2 || (v.size() > (facing ? 3u : 4u) && !ring) || !ParseFloat(v[0], x) ||
                !ParseFloat(v[1], y) || std::fabs(x) > 20000.f || std::fabs(y) > 20000.f ||
                (has_o && !ParseFloat(v[2], o)) ||
                (has_w && (!ParseFloat(v[3], w) || w < 1.f || w > CHECKPOINT_RADIUS)) || (facing && !has_o))
                return false;
            if (ring && (!has_o || !has_w || !ParseFloat(v[4], z) || std::fabs(z) > 20000.f ||
                !ParseFloat(v[5], pitch) || std::fabs(pitch) > 100.f || !ParseFloat(v[6], roll) ||
                std::fabs(roll) > 100.f))
                return false;
            // A facing within one turn: the editor's turning is not wrapped, and a huge one would
            // run the spot's text past the buffer it is written back into.
            o = std::fmod(o, 6.2831853f);
            if (o < 0.f)
                o += 6.2831853f;
            char buf[80];
            int n = std::snprintf(buf, sizeof(buf), "%.1f,%.1f", x, y);
            if (has_o)
                n += std::snprintf(buf + n, sizeof(buf) - size_t(n), ",%.3f", o);
            else if (has_w)
                n += std::snprintf(buf + n, sizeof(buf) - size_t(n), ",");
            if (has_w)
                n += std::snprintf(buf + n, sizeof(buf) - size_t(n), ",%.1f", w);
            if (ring)
                std::snprintf(buf + n, sizeof(buf) - size_t(n), ",%.1f,%.3f,%.3f", z, pitch, roll);
            out += (count ? ";" : "") + std::string(buf);
            ++count;
        }
        return true;
    }

    // "key,x,y,z,yaw,pitch,roll[,l,w,h];..." checked and written back at the wire's decimals: a key of
    // a-z, 0-9, '-' and '_' (CTR's props: ctr-crate_fruit) (the client knows its models by it; one it does not know it skips), a place
    // within the world and a turn in radians; false if any object is not so. `count` its objects.
    static bool CleanTrackObjects(std::string const& text, std::string& out, size_t& count)
    {
        out.clear();
        count = 0;
        for (size_t at = 0, end; at < text.size(); at = end + 1)
        {
            end = text.find(';', at);
            if (end == std::string::npos)
                end = text.size();
            std::string const spot = text.substr(at, end - at);
            if (spot.empty())
                continue;
            std::vector<std::string> v;
            for (size_t p = 0, c; p <= spot.size(); p = c + 1)
            {
                c = spot.find(',', p);
                if (c == std::string::npos)
                    c = spot.size();
                v.push_back(spot.substr(p, c - p));
            }
            // A place and a turn, and a build piece's size after them (the editor's ramps and
            // blocks: length, width, height, within PIECE_MIN to PIECE_MAX yards).
            if ((v.size() != 7 && v.size() != 10) || v[0].empty() || v[0].size() > 32)
                return false;
            for (char ch : v[0])
                if (!((ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9') || ch == '-' || ch == '_'))
                    return false;
            float n[9];
            for (size_t k = 0; k + 1 < v.size(); ++k)
            {
                float const most = k < 3 ? 20000.f : k < 6 ? 100.f : PIECE_MAX;
                if (!ParseFloat(v[k + 1], n[k]) || std::fabs(n[k]) > most || (k >= 6 && n[k] < PIECE_MIN))
                    return false;
            }
            char buf[200];
            int len = std::snprintf(buf, sizeof(buf), "%s,%.1f,%.1f,%.1f,%.3f,%.3f,%.3f", v[0].c_str(), n[0],
                n[1], n[2], n[3], n[4], n[5]);
            if (v.size() == 10)
                std::snprintf(buf + len, sizeof(buf) - size_t(len), ",%.1f,%.1f,%.1f", n[6], n[7], n[8]);
            out += (count ? ";" : "") + std::string(buf);
            ++count;
        }
        return true;
    }

    static void ListUserTracks(Player* player, uint32 now, bool now_please = false);

    static void PublishTrack(Player* player, std::vector<std::string> const& f, uint32 now)
    {
        auto no = [player](char const* why) { SendLine(player, std::string("KARTCRAFT\tR\tPubNo\t") + why); };
        if (f.size() < 8)
            return no("That track did not arrive whole.");
        uint32 const guid = player->GetGUIDLow();
        auto last = s_published.find(guid);
        if (last != s_published.end() && WorldTimer::getMSTimeDiff(last->second, now) < PUBLISH_COOLDOWN)
            return no("Wait a few seconds between publishing tracks.");
        UserTrack t;
        t.guid = guid;
        t.author = player->GetName();
        t.name = CleanTrackText(f[2], 40);
        t.map = uint32(std::strtoul(f[3].c_str(), nullptr, 10));
        t.laps = uint32(std::strtoul(f[4].c_str(), nullptr, 10));
        t.zone = CleanTrackText(f[5], 40);
        size_t gates = 0, grids = 0;
        if (t.name.empty())
            return no("The track needs a name.");
        // The open world, a battleground's open copy, or a dungeon or a raid, whose races run in
        // copies of their own (Race copies).
        MapEntry const* entry = sMapStorage.LookupEntry<MapEntry>(t.map);
        if (t.map > 1 && (!entry || !(entry->IsDungeon() || entry->IsBattleGround())))
            return no("Tracks can be published in the open world, dungeons, raids and battlegrounds.");
        // The track's floor height, an optional ninth field: under a roof, in a dungeon or the
        // open world's (the Caverns of Time's way down), the grid's trip lands on it.
        if (f.size() > 8 && !f[8].empty() && (!ParseFloat(f[8], t.z) || std::fabs(t.z) > 20000.f))
            return no("The track's height is not right.");
        if (t.laps > 9)
            return no("A track has at most 9 laps.");
        if (!CleanTrackSpots(f[7], false, t.points, gates) || gates < 2 || gates > USER_TRACK_GATES ||
            (t.laps > 0 && gates < 3))
            return no("The track's checkpoints are not right: 2 to 60 of them (3 for laps).");
        if (!CleanTrackSpots(f[6], true, t.grid, grids) || grids > 1)
            return no("The track's grid is not right.");
        // Its placed objects, an optional tenth field.
        size_t objects = 0;
        if (f.size() > 9 && !CleanTrackObjects(f[9], t.objects, objects))
            return no("The track's objects are not right.");
        if (objects > USER_TRACK_OBJECTS)
            return no("A track has at most 250 objects.");
        // Its scale, an optional eleventh field: the karts' and racers' size on it.
        if (f.size() > 10 && !f[10].empty() &&
            (!ParseFloat(f[10], t.scale) || t.scale < MIN_RACE_SCALE || t.scale > MAX_RACE_SCALE))
            return no("The track's scale is not right: 0.2 to 3.");
        auto same = std::find_if(s_userTracks.begin(), s_userTracks.end(),
            [&t](UserTrack const& u) { return u.guid == t.guid && u.name == t.name; });
        if (same != s_userTracks.end())
        {
            t.id = same->id;
            s_userTracks.erase(same);
        }
        else
        {
            size_t const mine = std::count_if(s_userTracks.begin(), s_userTracks.end(),
                [guid](UserTrack const& u) { return u.guid == guid; });
            if (mine >= USER_TRACKS_EACH)
                return no("You have 20 tracks published: take one down first.");
            if (s_userTracks.size() >= USER_TRACKS_MAX)
                return no("The server holds all the tracks it can.");
            t.id = s_userTrackNext++;
        }
        MakeUserCourse(t);
        s_userTracks.push_back(t);
        s_published[guid] = now;
        std::string author = t.author, name = t.name, zone = t.zone;
        CharacterDatabase.escape_string(author);
        CharacterDatabase.escape_string(name);
        CharacterDatabase.escape_string(zone);
        CharacterDatabase.PExecute("REPLACE INTO `kartcraft_user_tracks` (`id`, `guid`, `author`, `name`, "
            "`map`, `laps`, `zone`, `grid`, `points`, `z`, `objects`, `scale`, `at`) VALUES (%u, %u, '%s', '%s', "
            "%u, %u, '%s', '%s', '%s', %.1f, '%s', %.2f, %u)",
            t.id, t.guid, author.c_str(), name.c_str(), t.map, t.laps, zone.c_str(), t.grid.c_str(),
            t.points.c_str(), t.z, t.objects.c_str(), t.scale, uint32(std::time(nullptr)));
        SendLine(player, "KARTCRAFT\tR\tPubOk\t" + std::to_string(t.id) + "\t" + t.name);
        ListUserTracks(player, now, true);
    }

    // The player tracks to `player`, at most once per USER_TRACKS_ASK_COOLDOWN unless `now_please`
    // (after their own publish or take-down, so their lists show it at once).
    static void ListUserTracks(Player* player, uint32 now, bool now_please)
    {
        uint32 const guid = player->GetGUIDLow();
        auto last = s_userTracksAsked.find(guid);
        if (!now_please && last != s_userTracksAsked.end() &&
            WorldTimer::getMSTimeDiff(last->second, now) < USER_TRACKS_ASK_COOLDOWN)
            return;
        s_userTracksAsked[guid] = now;
        // Each player track's good and bad votes (kartcraft_rating), for the Race Finder's list.
        // ponytail: counted from the table at each list, once per USER_TRACKS_ASK_COOLDOWN a
        // player; counts kept in memory by Rate if the list is asked for much more.
        std::unordered_map<std::string, std::pair<uint32, uint32>> votes;
        std::unique_ptr<QueryResult> v = CharacterDatabase.Query(
            "SELECT `course`, SUM(`vote` > 0), SUM(`vote` < 0) FROM `kartcraft_rating` "
            "WHERE `course` LIKE 'pt-%' GROUP BY `course`");
        if (v)
            do
            {
                Field* f = v->Fetch();
                votes[f[0].GetCppString()] = std::make_pair(f[1].GetUInt32(), f[2].GetUInt32());
            } while (v->NextRow());
        SendLine(player, "KARTCRAFT\tR\tPTB");
        size_t sent = 0;
        for (auto t = s_userTracks.rbegin(); t != s_userTracks.rend() && sent < USER_TRACKS_LISTED; ++t, ++sent)
        {
            std::pair<uint32, uint32> const vote = votes[s_userCourses[t->slot].key];
            // Its height last, after the votes, so an older client reads the line as before.
            char z[16];
            std::snprintf(z, sizeof(z), "%.1f", t->z);
            SendLine(player, UserTrackLine(*t) + "\t" + std::to_string(vote.first) + "\t" +
                std::to_string(vote.second) + "\t" + (t->z != 0.f ? z : "") + "\t" + t->objects + "\t" +
                ScaleField(t->scale));
        }
        SendLine(player, "KARTCRAFT\tR\tPTE");
    }

    static void UnpublishTrack(Player* player, uint32 id)
    {
        auto t = std::find_if(s_userTracks.begin(), s_userTracks.end(), [id](UserTrack const& u) { return u.id == id; });
        if (t == s_userTracks.end())
            return SendLine(player, "KARTCRAFT\tR\tPubGone\t" + std::to_string(id));
        if (t->guid != player->GetGUIDLow() && player->GetSession()->GetSecurity() < SEC_GAMEMASTER)
            return;
        s_userTracks.erase(t);
        CharacterDatabase.PExecute("DELETE FROM `kartcraft_user_tracks` WHERE `id` = %u", id);
        SendLine(player, "KARTCRAFT\tR\tPubGone\t" + std::to_string(id));
        ListUserTracks(player, WorldTimer::getMSTime(), true);
    }

    // Ours called `name` taken down, if one is: the client deleting its saved track, which knows
    // no id (a publish replaces ours of the same name, so the name is the track).
    static void UnpublishOurTrack(Player* player, std::string const& name)
    {
        std::string const clean = CleanTrackText(name, 40);
        uint32 const guid = player->GetGUIDLow();
        auto t = std::find_if(s_userTracks.begin(), s_userTracks.end(),
            [&](UserTrack const& u) { return u.guid == guid && u.name == clean; });
        if (t != s_userTracks.end())
            UnpublishTrack(player, t->id);
    }

    // A race's close: everyone not in is out of time, recorded and told to all.
    static void CloseRace(uint32 id, Race& r)
    {
        if (!r.start)
            return;
        for (Racer& x : r.racers)
            if (!x.finished)
            {
                x.finished = true;
                x.place = 0;
                RecordResult(r, x, 0, 0);
                SendRace(r, "KARTCRAFT\tR\tDNF\t" + std::to_string(id) + "\t" + x.name);
            }
    }

    static void HandleRace(Player* player, std::string const& body, bool marshal)
    {
        uint32 now = WorldTimer::getMSTime();
        // RACE_LIFETIME from the start once it has gone, as a lobby may wait long; closed as a
        // late race is, so its racers are told and recorded.
        for (auto it = s_races.begin(); it != s_races.end();)
        {
            Race& r = it->second;
            // Counting down: started, its lifetime not yet begun.
            bool const counting = r.start && int32(now - r.start) < 0;
            uint32 const since = r.start ? r.start : r.created;
            if (counting || WorldTimer::getMSTimeDiff(since, now) <= RACE_LIFETIME)
            {
                it = std::next(it);
                continue;
            }
            CloseRace(it->first, r);
            it = s_races.erase(it);
        }
        UpdateFinder(now);

        // A publish's pieces: nothing until the last, then its whole line.
        std::string whole;
        if (body.compare(0, 7, "R\tPubP\t") == 0 && !PublishPiece(player, body.substr(7), whole))
            return;
        std::string const& line = whole.empty() ? body : whole;
        std::vector<std::string> f;
        for (size_t at = 0, tab; at <= line.size(); at = tab + 1)
        {
            tab = line.find('\t', at);
            if (tab == std::string::npos)
                tab = line.size();
            f.push_back(line.substr(at, tab - at));
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
        if (f.size() >= 2 && f[1] == "CH")
        {
            SendChallenges(player);
            return;
        }
        // Player tracks: published, listed and taken down, in the kart or out of it.
        if (f.size() >= 2 && f[1] == "Pub")
        {
            PublishTrack(player, f, now);
            return;
        }
        if (f.size() >= 2 && f[1] == "PubL")
        {
            ListUserTracks(player, now);
            return;
        }
        if (f.size() >= 3 && f[1] == "PubD")
        {
            UnpublishTrack(player, uint32(std::strtoul(f[2].c_str(), nullptr, 10)));
            return;
        }
        if (f.size() >= 3 && f[1] == "PubDN")
        {
            UnpublishOurTrack(player, f[2]);
            return;
        }
        // A courier's package goes by kart: none of the verbs that teleport (or queue for one).
        if (f.size() >= 2 && f[1] == "JL")
        {
            SendJobs(player);
            return;
        }
        static std::set<std::string> const TELEPORTS = { "TB", "TR", "TA", "TG", "MT", "LJ", "Q+", "Y", "Back", "JT", "Hub", "DT", "DB" };
        if (f.size() >= 2 && TELEPORTS.count(f[1]) && CarryingDelivery(player))
        {
            ChatHandler(player->GetSession()).SendSysMessage(
                "You're carrying a delivery: it goes by kart, so no teleports until it's delivered.");
            return;
        }
        if (f.size() >= 3 && f[1] == "JT")
        {
            TakeJob(player, uint32(std::strtoul(f[2].c_str(), nullptr, 10)), uint32(WorldTimer::getMSTime()));
            return;
        }
        if (f.size() >= 2 && f[1] == "CO")
        {
            SendCosmetics(player);
            return;
        }
        if (f.size() >= 4 && f[1] == "CE" && (f[2] == "P" || f[2] == "T"))
        {
            EquipCosmetic(player, f[2], f[3]);
            return;
        }
        if (f.size() >= 2 && (f[1] == "CrateOdds" || f[1] == "CrateBuy"))
        {
            ObjectGuid const guid = player->GetObjectGuid();
            std::string const verb = f[1];
            sWorld.GetMessager().AddMessage([guid, verb](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    HandleCrate(p, verb);
            });
            return;
        }
        if (f.size() >= 4 && f[1] == "Hop")
        {
            ObjectGuid const guid = player->GetObjectGuid();
            std::string const key = f[2];
            uint32 const ms = uint32(std::strtoul(f[3].c_str(), nullptr, 10));
            sWorld.GetMessager().AddMessage([guid, key, ms](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    HandleHop(p, key, ms);
            });
            return;
        }
        if (f.size() >= 2 && (f[1] == "RList" || f[1] == "RRec" || f[1] == "RPin"))
        {
            ObjectGuid const guid = player->GetObjectGuid();
            std::vector<std::string> const args = f;
            sWorld.GetMessager().AddMessage([guid, args](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    HandleReplays(p, args);
            });
            return;
        }
        if (f.size() >= 2 && f[1] == "Hub")
        {
            ObjectGuid const guid = player->GetObjectGuid();
            sWorld.GetMessager().AddMessage([guid](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    HubTeleport(p);
            });
            return;
        }
        if (f.size() >= 3 && f[1] == "AC")
        {
            ObjectGuid const guid = player->GetObjectGuid();
            uint64 const cache = std::strtoull(f[2].c_str(), nullptr, 10);
            sWorld.GetMessager().AddMessage([guid, cache](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    CacheTouched(p, cache);
            });
            return;
        }
        if (f.size() >= 2 && (f[1] == "Goods" || f[1] == "GoodBuy" || f[1] == "FlagWear"))
        {
            ObjectGuid const guid = player->GetObjectGuid();
            std::string const verb = f[1], key = f.size() > 2 ? f[2] : std::string();
            sWorld.GetMessager().AddMessage([guid, verb, key](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    HandleGoods(p, verb, key);
            });
            return;
        }
        if (f.size() >= 2 && (f[1] == "Skins" || f[1] == "SkinBuy" || f[1] == "SkinWear"))
        {
            ObjectGuid const guid = player->GetObjectGuid();
            std::string const verb = f[1], key = f.size() > 2 ? f[2] : std::string();
            sWorld.GetMessager().AddMessage([guid, verb, key](World*)
            {
                if (Player* p = ObjectAccessor::FindPlayer(guid))
                    HandleSkins(p, verb, key);
            });
            return;
        }
        if (f.size() >= 4 && f[1] == "Rate")
        {
            Rate(player, f[2], f[3], f.size() >= 5 ? f[4] : std::string());
            return;
        }
        if (f.size() >= 2 && f[1] == "Off")
        {
            SendCourseOff(player);
            return;
        }
        // The showcase's verb works on foot, as a GM's teleport does.
        if (f.size() >= 2 && HandleShow(player, f))
            return;
        // The finder's verbs work on foot; the rest need the kart, but for a marshal's and for
        // leaving: a racer the marshal put in on foot may leave before the kart's aura lands.
        if (f.size() >= 2 && HandleMarathon(player, f))
            return;
        if (f.size() < 2 || HandleFinder(player, f, now) ||
            (!marshal && f[1] != "Q" && !player->HasAura(SPELL_SUMMON_KART)))
            return;
        std::string const& verb = f[1];
        uint32 id = f.size() > 2 ? uint32(std::strtoul(f[2].c_str(), nullptr, 10)) : 0;
        auto race = s_races.find(id);

        if (verb == "H" && f.size() > 2)
        {
            // "R\tH\t<key>\t1" hosts with ground types on; a race already waiting keeps its own.
            Course const* course = FindCourse(f[2]);
            if (!course || player->GetMapId() != course->map || (!marshal && DistanceTo(player, course->points[0]) > START_RADIUS))
                return;
            if (RefuseOff(player, *course))
                return;
            // A party course is hosted inside its dungeon, by the party's leader.
            uint32 const instance = PartyCourse(*course) ? player->GetInstanceId() : 0;
            Group* group = player->GetGroup();
            if (instance && group && !group->IsLeader(player->GetObjectGuid()))
            {
                ChatHandler(player->GetSession()).SendSysMessage("Only your party's leader can host a race here.");
                return;
            }
            // A race already waiting on this course takes them in: who arrives after the offer
            // (sent once, to those near) still races with the host rather than alone. A party
            // race only if it is theirs to join.
            uint32 joinId = 0;
            for (auto& entry : s_races)
            {
                Race& open = entry.second;
                bool in = std::any_of(open.racers.begin(), open.racers.end(),
                    [player](Racer const& x) { return x.guid == player->GetObjectGuid(); });
                if (open.course == course && !open.start && !open.timeTrial && open.instance == instance &&
                    MayJoin(player, open) && (in || open.racers.size() < MAX_RACERS))
                {
                    if (in)
                    {
                        SendRace(open, "KARTCRAFT\tR\tL\t" + std::to_string(entry.first) + "\t" + Roster(open));
                        return;
                    }
                    joinId = entry.first;
                    break;
                }
            }
            // Hosting or joining here leaves the queue and every other race still waiting; one
            // under way stays, and refuses it, as a ready check up does.
            if (InPop(player->GetObjectGuid()))
            {
                ChatHandler(player->GetSession()).SendSysMessage("Answer the Race Finder's ready check first.");
                return;
            }
            if (!LeaveWaitingRaces(player))
                return;
            LeaveQueue(player->GetObjectGuid());
            auto join = s_races.find(joinId);
            if (joinId && join != s_races.end())
            {
                join->second.racers.push_back({ player->GetObjectGuid(), player->GetName() });
                SendRace(join->second, "KARTCRAFT\tR\tL\t" + std::to_string(joinId) + "\t" + Roster(join->second));
                return;
            }
            uint32 raceId = s_nextRace++;
            Race& r = s_races[raceId];
            r.course = course;
            r.host = player->GetObjectGuid();
            r.created = now;
            r.groundTypes = f.size() > 3 && f[3] == "1";
            r.instance = instance;
            r.racers.push_back({ player->GetObjectGuid(), player->GetName() });
            WorldPacket data;
            std::string open = "KARTCRAFT\tR\tO\t" + std::to_string(raceId) + "\t" + course->key + "\t" +
                course->name + "\t" + player->GetName();
            ChatHandler::BuildChatPacket(data, CHAT_MSG_PARTY, open.c_str(), LANG_ADDON, CHAT_TAG_NONE,
                player->GetObjectGuid(), player->GetName());
            player->SendMessageToSetInRange(&data, START_RADIUS * 2.f, true);
            SendRace(r, "KARTCRAFT\tR\tL\t" + std::to_string(raceId) + "\t" + Roster(r));
        }
        else if (verb == "T" && f.size() > 2)
        {
            // A time trial at the course's start, the Race Marshal's (OpenTimeTrial): alone on
            // the grid's first slot, the usual countdown, then the laps, no crates or items.
            Course const* course = FindCourse(f[2]);
            uint32 const instance = course && PartyCourse(*course) ? player->GetInstanceId() : 0;
            if (!course || player->GetMapId() != course->map || !CanRaceOn(player, *course, instance) ||
                (!marshal && DistanceTo(player, course->points[0]) > START_RADIUS) || RefuseOff(player, *course))
                return;
            uint32 raceId = OpenTimeTrial(player, *course, instance, now);
            TeleportToGrid(player, *course, 0, instance);
            StartRace(raceId, s_races[raceId], now, GRID_COUNTDOWN);
        }
        else if (verb == "J" && race != s_races.end())
        {
            Race& r = race->second;
            if (r.start || r.timeTrial || r.racers.size() >= MAX_RACERS || player->GetMapId() != r.course->map ||
                !MayJoin(player, r) || (!marshal && DistanceTo(player, r.course->points[0]) > START_RADIUS))
                return;
            for (Racer const& x : r.racers)
                if (x.guid == player->GetObjectGuid())
                    return;
            // As hosting: out of the queue and every other race still waiting.
            if (InPop(player->GetObjectGuid()))
            {
                ChatHandler(player->GetSession()).SendSysMessage("Answer the Race Finder's ready check first.");
                return;
            }
            if (!LeaveWaitingRaces(player))
                return;
            LeaveQueue(player->GetObjectGuid());
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
                    Origin from = { p->GetMapId(), p->GetPositionX(), p->GetPositionY(), p->GetPositionZ(),
                        p->GetOrientation(), now };
                    if (TeleportToGrid(p, *r.course, slot++, r.instance) && x.fromList)
                        s_origins[p->GetGUIDLow()] = from;
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
            if (!me || me->finished || (r.instance && player->GetInstanceId() != r.instance))
                return;
            Course const& c = *r.course;
            uint32 n = uint32(c.points.size());
            uint32 index = uint32(std::strtoul(f[3].c_str(), nullptr, 10));
            // The next checkpoint only, and only once the server has seen the racer there: now, at
            // a teleport's landing, or on any earlier look since the last pass (a report lost or
            // overtaken, sent again by the client).
            uint32 passes = Passes(*me, n);
            uint32 expected = (passes + 1) % n;
            if (index != expected || index >= n || passes >= FinalPass(c))
                return;
            Witness(*me, c, player);
            if (me->seen <= passes)
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
                if (!r.firstFinish)
                    r.firstFinish = now ? now : 1;
                uint32 ms = WorldTimer::getMSTimeDiff(r.start, now);
                me->ms = ms;
                ReplayFinish(id, me->guid, me->place, ms, now);
                RecordResult(r, *me, place, ms);
                if (r.timeTrial)
                {
                    ChallengeFinish(player->GetGUIDLow(), me->name, c.key, ms, &r);
                    s_lastTrial[player->GetGUIDLow()] = c.key;
                }
                std::string const best = RecordFinish(BoardOf(r), player->GetGUIDLow(), me->name, c, ms);
                // The take keeps a new best, and a new record (Replays).
                if (!best.empty())
                {
                    char const* board = r.timeTrial ? "T" : "R";
                    ReplayKeep(r, (std::string("p") + board).c_str(), player->GetGUIDLow(), c.key, player->GetGUIDLow());
                    if (best == "r")
                        ReplayKeep(r, (std::string("r") + board).c_str(), 0, c.key, player->GetGUIDLow());
                }
                SendRace(r, "KARTCRAFT\tR\tF\t" + std::to_string(id) + "\t" + me->name + "\t" +
                    std::to_string(place) + "\t" + std::to_string(ms) + "\t" + best);
                if (!best.empty())
                    SendRecords(id, r);
                Reward(player, place, uint32(r.racers.size()));
                // Over when everyone still in has finished: a finisher who left counts in
                // `finishers` but no longer in `racers`, so the counts would close it a racer early.
                if (std::all_of(r.racers.begin(), r.racers.end(), [](Racer const& x) { return x.finished; }))
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
    // Every line is a look at where its sender is, for each race under way they are still in.
    static void WitnessSender(Player* player, uint32 now)
    {
        for (auto& entry : s_races)
        {
            Race& r = entry.second;
            if (!r.start || int32(now - r.start) < 0)
                continue;
            for (Racer& x : r.racers)
                if (x.guid == player->GetObjectGuid() && !x.finished)
                    Witness(x, *r.course, player);
        }
    }

    // DNF_WAIT after its first finish a race closes: everyone not in is out of time, told to all.
    // Run with every line from anyone, as the Race Finder's matching is: racers in the kart send
    // their state many times a second, so the close comes on time.
    static void CloseLateRaces(uint32 now)
    {
        for (auto it = s_races.begin(); it != s_races.end();)
        {
            Race& r = it->second;
            if (!r.firstFinish || WorldTimer::getMSTimeDiff(r.firstFinish, now) < DNF_WAIT)
            {
                it = std::next(it);
                continue;
            }
            CloseRace(it->first, r);
            it = s_races.erase(it);
        }
    }

    // A track browser time trial waiting for its racer: their kart state from the course's grid
    // (the teleport and any loading screen behind them) starts the countdown; ARRIVE_WAIT
    // starts it regardless. On the grid is near its first slot, where the trial sent them (a
    // player track's own grid may stand further than START_RADIUS from its start), or near
    // the start.
    static void TickArrival(Player* player, bool state, uint32 now)
    {
        for (auto& entry : s_races)
        {
            Race& r = entry.second;
            if (r.start || !r.awaiting)
                continue;
            bool arrived = false;
            if (state && player->GetObjectGuid() == r.awaiting && player->GetMapId() == r.course->map)
            {
                auto near = [player](float x, float y) {
                    float dx = player->GetPositionX() - x, dy = player->GetPositionY() - y;
                    return dx * dx + dy * dy <= START_RADIUS * START_RADIUS;
                };
                float gx, gy, go;
                GridSpot(*r.course, 0, gx, gy, go);
                arrived = near(gx, gy) || near(r.course->points[0].first, r.course->points[0].second);
            }
            if (arrived || WorldTimer::getMSTimeDiff(r.awaitSince, now) >= ARRIVE_WAIT)
            {
                r.awaiting = ObjectGuid();
                StartRace(entry.first, r, now, ARRIVED_COUNTDOWN);
            }
        }
    }

    // How often (ms) a kart state line, the many-a-second one, runs the races' time-based
    // housekeeping; every other line runs it at once.
    constexpr uint32 STATE_HOUSEKEEPING = 50;
    static uint32 s_housekept = 0; // under s_mutex

    static void HandleLine(Player* player, std::string const& body, bool marshal)
    {
        bool const state = body.compare(0, 2, "S\t") == 0;
        // Relayed before the race lock: it reads no race, and every driver's lines wait on it.
        if (state)
            RelayState(player, PREFIX + body);
        std::lock_guard<std::mutex> lock(s_mutex);
        uint32 const now = WorldTimer::getMSTime();
        WitnessSender(player, now);
        // A state line says nothing of a race: its time-based checks at most every
        // STATE_HOUSEKEEPING, with someone's lines still coming many times a second.
        bool const housekeep = !state || !s_housekept || WorldTimer::getMSTimeDiff(s_housekept, now) >= STATE_HOUSEKEEPING;
        if (housekeep)
        {
            s_housekept = now;
            CloseLateRaces(now);
            TickDoors();
            TickWeapons(now);
        }
        if (body.compare(0, 2, "L\t") == 0)
        {
            float points = float(std::atof(body.c_str() + 2));
            if (points > 0.f)
                LandLine(player, points);
        }
        else if (body.compare(0, 2, "R\t") == 0)
            HandleRace(player, body, marshal);
        else if (body.compare(0, 2, "W\t") == 0)
            HandleWeapon(player, body);
        TickArrival(player, state, now);
        // After the line: a finish or a leave in it frees the racer at once.
        if (housekeep)
        {
            TickShields(WorldTimer::getMSTime());
            TickLayers();
        }
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
    constexpr uint32 MARSHAL_TRIALS_TEXT = 190003;  // npc_text: over the time trial leaderboard
    constexpr uint32 MARSHAL_HOST_TEXT = 190004;    // npc_text: over hosting's ground types choice
    // The leaderboard's length.
    constexpr size_t TRIAL_BOARD_TOP = 10;
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
        MARSHAL_TRIAL,  // action: the course's index in COURSES
        MARSHAL_TRIALS, // action: the course's index in COURSES, or COURSES.size() for the list
        MARSHAL_HOST_PLAIN,  // action: the course's index in COURSES; ground types off
        MARSHAL_HOST_GROUND, // action: the course's index in COURSES; ground types on
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
        // Room for the lines a menu ends with: leave, finder, tracks, records, leaderboard, help.
        size_t room = GOSSIP_MAX_MENU_ITEMS - 6;
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
                // Another copy's party race is not this marshal's to offer.
                if (r.start || r.timeTrial || WorldTimer::getMSTimeDiff(r.created, now) > RACE_LIFETIME ||
                    !AtMarshal(here, r.course) || (r.instance && r.instance != player->GetInstanceId()))
                    continue;
                waiting.push_back(r.course);
                if (r.host == guid && room)
                {
                    menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Start the race on " + std::string(r.course->name) + " (" +
                        std::to_string(r.racers.size()) + (r.racers.size() == 1 ? " racer)" : " racers)"),
                        MARSHAL_START, entry.first, "", false);
                    --room;
                }
                else if (!in && r.racers.size() < MAX_RACERS && MayJoin(player, r))
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
                    std::to_string(r.racers.size()) + (r.racers.size() == 1 ? " racer" : " racers") +
                    (r.groundTypes ? ", ground types on)" : ")"),
                    MARSHAL_JOIN, j.first, "", false);
                --room;
            }
            // A course with a race waiting is joined, not hosted again (as "R\tH" itself does); a
            // time trial is the player's own, on any course here.
            for (size_t i : here)
            {
                Course const& c = COURSES[i];
                if (!room)
                    break;
                std::string const kind = c.laps ? std::to_string(c.laps) + (c.laps == 1 ? " lap)" : " laps)")
                                                : std::string("sprint)");
                if (std::find(waiting.begin(), waiting.end(), &c) == waiting.end())
                {
                    menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Host a race: " + std::string(c.name) + " (" + kind,
                        MARSHAL_HOST, uint32(i), "", false);
                    if (!--room)
                        break;
                }
                menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Time trial: " + std::string(c.name) + " (" + kind,
                    MARSHAL_TRIAL, uint32(i), "", false);
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
        {
            menu.AddMenuItem(GOSSIP_ICON_CHAT, "Course records", MARSHAL_RECORDS, 0, "", false);
            // One course here: its leaderboard at once; several: the list to pick from.
            menu.AddMenuItem(GOSSIP_ICON_CHAT, "Time trial leaderboard", MARSHAL_TRIALS,
                here.size() == 1 ? uint32(here.front()) : uint32(COURSES.size()), "", false);
        }
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
                auto record = s_raceBoard.records.find(c.key);
                std::string line = std::string(c.name) + ": ";
                if (record == s_raceBoard.records.end() || !record->second.ms)
                    line += "no record yet";
                else
                    line += Clock(record->second.ms) + " by " + record->second.name;
                if (uint32 best = BestOf(s_raceBoard, guid, c.key))
                    line += " (yours " + Clock(best) + ")";
                menu.AddMenuItem(GOSSIP_ICON_DOT, line, MARSHAL_RECORDS, 0, "", false);
            }
        }
        menu.AddMenuItem(GOSSIP_ICON_CHAT, "Back", MARSHAL_BACK, 0, "", false);
        player->PlayerTalkClass->SendGossipMenu(MARSHAL_RECORDS_TEXT, marshal->GetObjectGuid());
    }

    // The time trial leaderboard of `course` (an index in COURSES) when it starts here: its top
    // TRIAL_BOARD_TOP times with their holders, and the player's own best and rank. Else (the
    // list's action, COURSES.size()) the courses here, a line each, to pick from. A line chosen
    // shows its page again.
    static void MarshalTrials(Player* player, Creature* marshal, uint32 course)
    {
        std::vector<size_t> const here = MarshalCourses(marshal);
        uint32 const guid = player->GetGUIDLow();
        GossipMenu& menu = player->PlayerTalkClass->GetGossipMenu();
        menu.ClearMenu();
        bool const one = course < COURSES.size() && AtMarshal(here, &COURSES[course]);
        if (!one)
            for (size_t i = 0; i < here.size() && i + 1 < GOSSIP_MAX_MENU_ITEMS; ++i)
                menu.AddMenuItem(GOSSIP_ICON_DOT, "Time trials: " + std::string(COURSES[here[i]].name), MARSHAL_TRIALS,
                    uint32(here[i]), "", false);
        else
        {
            Course const& c = COURSES[course];
            std::lock_guard<std::mutex> lock(s_mutex);
            std::vector<std::pair<uint32, Record>> const ranking = Ranking(s_trialBoard, c.key);
            if (ranking.empty())
                menu.AddMenuItem(GOSSIP_ICON_DOT, std::string(c.name) + ": no time trials yet", MARSHAL_TRIALS,
                    course, "", false);
            for (size_t i = 0; i < ranking.size() && i < TRIAL_BOARD_TOP && i + 3 < GOSSIP_MAX_MENU_ITEMS; ++i)
                menu.AddMenuItem(GOSSIP_ICON_DOT, std::to_string(i + 1) + ". " + Clock(ranking[i].second.ms) + "  " +
                    ranking[i].second.name, MARSHAL_TRIALS, course, "", false);
            auto mine = std::find_if(ranking.begin(), ranking.end(),
                [guid](std::pair<uint32, Record> const& e) { return e.first == guid; });
            menu.AddMenuItem(GOSSIP_ICON_CHAT, mine == ranking.end()
                ? std::string("You have no time trial on ") + c.name + " yet"
                : "Your best: " + Clock(mine->second.ms) + " (rank " + std::to_string(mine - ranking.begin() + 1) +
                    " of " + std::to_string(ranking.size()) + ")",
                MARSHAL_TRIALS, course, "", false);
        }
        menu.AddMenuItem(GOSSIP_ICON_CHAT, "Back", MARSHAL_BACK, 0, "", false);
        player->PlayerTalkClass->SendGossipMenu(MARSHAL_TRIALS_TEXT, marshal->GetObjectGuid());
    }

    // Hosting `course` (an index in COURSES): with CTR's ground types or without, for every racer.
    static void MarshalHost(Player* player, Creature* marshal, uint32 course)
    {
        GossipMenu& menu = player->PlayerTalkClass->GetGossipMenu();
        menu.ClearMenu();
        std::string const name = COURSES[course].name;
        menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Host " + name + ": ground types off", MARSHAL_HOST_PLAIN, course, "", false);
        menu.AddMenuItem(GOSSIP_ICON_BATTLE, "Host " + name + ": ground types on (off-road slows)", MARSHAL_HOST_GROUND,
            course, "", false);
        menu.AddMenuItem(GOSSIP_ICON_CHAT, "Back", MARSHAL_BACK, 0, "", false);
        player->PlayerTalkClass->SendGossipMenu(MARSHAL_HOST_TEXT, marshal->GetObjectGuid());
    }

    static bool MarshalSelect(Player* player, Creature* marshal, uint32 sender, uint32 action)
    {
        std::string line;
        switch (sender)
        {
            case MARSHAL_HOST:
                if (action < COURSES.size() && AtMarshal(MarshalCourses(marshal), &COURSES[action]))
                {
                    MarshalHost(player, marshal, action);
                    return true;
                }
                break;
            case MARSHAL_HOST_PLAIN:
            case MARSHAL_HOST_GROUND:
                if (action < COURSES.size() && AtMarshal(MarshalCourses(marshal), &COURSES[action]))
                    line = std::string("R\tH\t") + COURSES[action].key + (sender == MARSHAL_HOST_GROUND ? "\t1" : "");
                break;
            case MARSHAL_TRIAL:
                if (action < COURSES.size() && AtMarshal(MarshalCourses(marshal), &COURSES[action]))
                    line = std::string("R\tT\t") + COURSES[action].key;
                break;
            case MARSHAL_TRIALS:
                MarshalTrials(player, marshal, action);
                return true;
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
            // Hosting, joining or a time trial on foot: into the kart first, as a Race Finder pop
            // does; in a dungeon too, where a marshal has only party courses.
            bool const enters = line.compare(0, 4, "R\tH\t") == 0 || line.compare(0, 4, "R\tJ\t") == 0 ||
                line.compare(0, 4, "R\tT\t") == 0;
            if (enters && (CanBeRacedOff(p) || (p->GetMap()->IsDungeon() && Settled(p))))
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
        // The hub's Quartermaster (sql/kartcraft.sql).
        Script* hub = new Script;
        hub->Name = "npc_kartcraft_hub";
        hub->pGossipHello = &HubHello;
        hub->pGossipSelect = &HubSelect;
        hub->RegisterSelf(true);
        // The caches' chests (sql/kartcraft_caches.sql).
        Script* cache = new Script;
        cache->Name = "go_kartcraft_cache";
        cache->pGOHello = &CacheHello;
        cache->RegisterSelf(true);
        CreateCacheClaims();
        CreateHops();
        CreateCrates();
        CreateReplays();
        CreateResults();
        CreateChallenges();
        CreateCosmetics();
        CreateRatings();
        CreateDeliveries();
        CreateMarathons();
        CreateSkins();
        CreateGoods();
        LoadLadder();
        LoadRecords(s_raceBoard, "race");
        LoadRecords(s_trialBoard, "time trial");
        LoadUserTracks();
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
        // A courier's package goes by kart: no hearthstone, no teleport of oneself.
        if (Player const* courier = caster->ToPlayer())
            if (CarryingDelivery(courier))
                for (uint32 i = 0; i < MAX_EFFECT_INDEX; ++i)
                    if (spell->Effect[i] == SPELL_EFFECT_TELEPORT_UNITS)
                    {
                        ChatHandler(courier->GetSession()).SendSysMessage(
                            "You're carrying a delivery: it goes by kart, so no teleports until it's delivered.");
                        return SPELL_FAILED_DONT_REPORT;
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
                if (InRace(caster))
                    return SPELL_FAILED_NOT_HERE;
                break;
        }
        return SPELL_CAST_OK;
    }
}
