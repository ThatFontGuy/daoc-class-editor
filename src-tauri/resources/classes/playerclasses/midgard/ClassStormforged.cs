using System.Collections.Generic;
using DOL.GS.Realm;

namespace DOL.GS.PlayerClass
{
    [CharacterClass((int)eCharacterClass.Stormforged, "Storm Sovereign", "Viking")]
    public class ClassStormforged : ClassThane
    {
        public ClassStormforged()
        {
            m_specializationMultiplier = 80;
            m_primaryStat = eStat.STR;
            m_secondaryStat = eStat.PIE;
            m_tertiaryStat = eStat.CON;
        }

        public override List<PlayerRace> EligibleRaces => new List<PlayerRace>()
        {
            PlayerRace.Norseman, PlayerRace.Troll, PlayerRace.Dwarf, PlayerRace.Kobold,
            PlayerRace.Valkyn, PlayerRace.Frostalf, PlayerRace.Deifrang
        };
    }
}
