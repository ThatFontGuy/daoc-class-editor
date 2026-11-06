using System.Collections.Generic;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    /// <summary>
    /// Death Knight RR5 Call of Darkness (live delve): 30 seconds as a Demon Lord, with the benefits of Chthonic and
    /// Decrepit Form increased by 10%. It does not return the Death Knight to a form when it ends. 10-minute reuse.
    /// The classic Necromancer keeps its own Call of Darkness ability.
    /// </summary>
    public class ModernDeathKnightDarkness : ModernRr5Ability
    {
        /// <summary>The Darkness form row is the first spell of the Death Knight kit.</summary>
        public const int SpellId = 71000;

        public ModernDeathKnightDarkness(DbAbility ability, int level) : base(ability, level) { }

        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || !DeathKnight.Is(living))
                return;
            Spell spell = SkillBase.GetSpellByID(SpellId);
            if (spell == null)
                return;
            SendCasterSpellEffectAndCastMessage(living, 7051, true);
            new DeathKnightFormHandler(living, spell, SkillBase.GetSpellLine(GlobalSpellsLines.Character_Abilities)).ApplyEffectOnTarget(living);
            DisableSkill(living);
        }

        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Become a Demon Lord for 30 seconds: the benefits of Chthonic and Decrepit Form, increased by 10%. You do not return to a form afterwards.");
    }
}
