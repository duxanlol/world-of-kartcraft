/*
 * World of Kartcraft: Karting, a secondary profession levelled by chaining kart turbos.
 *
 * The benilla client drives CTR's kart model and reports each turbo chain to us as an addon
 * message ("KARTCRAFT\tL\t<points>"). We raise the Karting skill, give character XP and
 * teach the kart abilities as the skill grows. The skill line and the spells live in code, so
 * the server needs no edited DBC files or database rows.
 */

#ifndef KARTCRAFT_H
#define KARTCRAFT_H

#include "Common.h"
#include "SpellDefines.h"

class Player;
class Unit;
class WorldObject;
class SpellEntry;
class Quest;

namespace Kartcraft
{
    constexpr uint32 SKILL_KARTING = 760;
    constexpr uint32 SKILL_MAX = 300;

    // Spell ids stay below 65536: SMSG_INITIAL_SPELLS carries them as uint16.
    constexpr uint32 SPELL_SUMMON_KART = 60001; // the ride itself, an aura while in the kart
    constexpr uint32 SPELL_WUMPA_JUICE       = 60002;
    constexpr uint32 SPELL_TURBO_BOOST      = 60003;
    constexpr uint32 SPELL_SUPER_ENGINE       = 60004;
    constexpr uint32 SPELL_AKU_AKU         = 60005;

    // After the DBC stores load: the Karting skill line and its race/class rule.
    void LoadSkillLine();
    // After spell_template loads: the kart spells.
    void LoadSpells();
    // On login: everyone can drive; abilities match the skill.
    void OnLogin(Player* player);
    // The kart's aura on or off (Aura::HandleAuraMounted): a driver is friendly to everyone.
    void OnMounted(Unit* target, uint32 spell, bool apply);
    // Race layers: false when a and b are players in different races' layers (each race under
    // way is its own instance); creatures and everything else are in every layer. Player's
    // visibility check and the player message deliverers ask it.
    bool SameLayer(WorldObject const* a, WorldObject const* b);
    // An addon message from the client; true when it was ours.
    bool HandleAddonMessage(Player* player, std::string const& message);
    // Extra cast checks for the kart spells. SPELL_CAST_OK otherwise.
    SpellCastResult CheckCast(Unit const* caster, SpellEntry const* spell);
    // After ScriptMgr::Initialize: the Race Marshal's gossip (npc_kartcraft_marshal).
    void RegisterScripts();
    // Whether a player is in the kart: a driver springs no dungeon's traps nor its scripted
    // ambushes (Stratholme's rat-trap gates and The Unforgiven, instance_stratholme.cpp).
    bool InKart(Player const* player);
    // Courier deliveries (kartcraft_deliveries.sql): whether `quest`, if one of them, is offered
    // to the player now (today's, not yet delivered today, no package already carried), asked
    // by Player::CanSeeStartQuest and CanTakeQuest; `msg` says why not.
    bool DeliveryOffered(Player const* player, Quest const* quest, bool msg);
    // Player::AddQuest: a delivery's clock starts.
    void OnQuestAccepted(Player* player, Quest const* quest);
    // Player::RewardQuest: a delivery made, its bonus paid when within par.
    void OnQuestRewarded(Player* player, Quest const* quest);
    // Whether the player carries a courier's package: no flight path takes them
    // (Player::ActivateTaxiPathTo), nor any teleport.
    bool CarryingDelivery(Player const* player);
}

#endif
