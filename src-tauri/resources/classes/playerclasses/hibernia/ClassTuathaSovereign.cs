using System.Collections.Generic;
using DOL.GS.Realm;

namespace DOL.GS.PlayerClass
{
    [CharacterClass((int)eCharacterClass.TuathaSovereign, "Tuatha Sovereign", "Guardian")]
    public class ClassTuathaSovereign : ClassHero
    {
        public ClassTuathaSovereign()
        {
            m_specializationMultiplier = 80;
            m_primaryStat = eStat.STR;
            m_secondaryStat = eStat.PIE;
            m_tertiaryStat = eStat.CON;
            // Hero has no power stat; the royal spells need Piety-based power like the other Sovereigns.
            m_manaStat = eStat.PIE;
            // Match Storm Sovereign's underlying HP and weapon skill curves.
            m_wsbase = 360;
            m_baseHP = 720;
        }
        // A royal spellcaster like the other Sovereigns, not Hero's pure tank.
        public override eClassType ClassType => eClassType.Hybrid;
        public override IList<string> GetAutotrainableSkills() => new List<string>();
        public override List<PlayerRace> EligibleRaces => new()
        {
            PlayerRace.Celt, PlayerRace.Firbolg, PlayerRace.Elf, PlayerRace.Lurikeen,
            PlayerRace.Sylvan, PlayerRace.Shar, PlayerRace.Graoch
        };
    }
}
