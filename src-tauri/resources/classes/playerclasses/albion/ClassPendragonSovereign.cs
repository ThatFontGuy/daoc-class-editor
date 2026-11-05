using System.Collections.Generic;
using DOL.GS.Realm;

namespace DOL.GS.PlayerClass
{
    [CharacterClass((int)eCharacterClass.PendragonSovereign, "Pendragon Sovereign", "Fighter")]
    public class ClassPendragonSovereign : ClassPaladin
    {
        public ClassPendragonSovereign()
        {
            m_specializationMultiplier = 80;
            m_primaryStat = eStat.STR;
            m_secondaryStat = eStat.PIE;
            m_tertiaryStat = eStat.CON;
            // Match Storm Sovereign's underlying HP and weapon skill curves.
            m_wsbase = 360;
            m_baseHP = 720;
        }
        public override IList<string> GetAutotrainableSkills() => new List<string>();
        public override List<PlayerRace> EligibleRaces => new()
        {
            PlayerRace.Briton, PlayerRace.Avalonian, PlayerRace.Highlander, PlayerRace.Saracen,
            PlayerRace.Inconnu, PlayerRace.HalfOgre, PlayerRace.Korazh
        };
    }
}
