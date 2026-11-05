using System.Collections.Generic;
using DOL.GS.Realm;

namespace DOL.GS.PlayerClass
{
    // Native client slot 64 (Midgard); took over the slot from the retired Stormforged.
    // Originally built on the daoc-offline-linux-server-bots project (class 65 there), migrated
    // to this server first as class 69 (to avoid colliding with PendragonSovereign), then moved
    // here to 64 once Stormforged was retired off the slot.
    [CharacterClass((int)eCharacterClass.Drengr, "Drengr", "Viking")]
    public class ClassDrengr : ClassWarrior
    {
        // Drengr trains Spear/Staff/Parry, not ClassWarrior's Axe/Hammer/Sword (none of which are
        // even in her spec list), so the inherited autotrain list gave her no baseline competency
        // in either weapon she can actually use.
        private static readonly string[] AutotrainableSkills = new[] { Specs.Spear, Specs.Staff };

        public ClassDrengr()
            : base()
        {
            m_manaStat = eStat.PIE;
        }

        public override eClassType ClassType => eClassType.Hybrid;

        public override IList<string> GetAutotrainableSkills()
        {
            return AutotrainableSkills;
        }

        public override List<PlayerRace> EligibleRaces => new List<PlayerRace>()
        {
            PlayerRace.Dwarf,
            PlayerRace.Kobold,
            PlayerRace.Norseman,
            PlayerRace.Troll,
        };
    }
}
