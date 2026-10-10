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
class SpellEntry;

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
    // An addon message from the client; true when it was ours.
    bool HandleAddonMessage(Player* player, std::string const& message);
    // Extra cast checks for the kart spells. SPELL_CAST_OK otherwise.
    SpellCastResult CheckCast(Unit const* caster, SpellEntry const* spell);
}

#endif
