using System.Collections.Generic;
using DOL.Database;

namespace DOL.GS.RealmAbilities
{
    public class ModernFirstAid : AtlasOF_FirstAid, IModernRealmAbility
    {
        public ModernFirstAid(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.ActiveCost(currentLevel);
        public override int GetReUseDelay(int level) => ModernRealmRanks.Value(level, 600, 450, 300, 240, 180);
        // Basis points preserve the half-percent fourth rank without rounding it to 42%.
        public static int HealAmount(int maxHealth, int rank) => (int)((long)maxHealth * ModernRealmRanks.Value(rank, 250, 300, 350, 425, 500) / 1000);
        public override void Execute(GameLiving living)
        {
            if (Level < 1 || Level > MaxLevel || CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED | INCOMBAT))
                return;
            int healed = living.ChangeHealth(living, eHealthChangeType.Spell, HealAmount(living.MaxHealth, Level));
            SendCasterSpellEffectAndCastMessage(living, 7001, healed > 0);
            if (healed > 0)
                DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Out-of-combat self heal. Cannot be used while sitting or controlled.");
            list.Add("Ranks 1–5 restore 25%, 30%, 35%, 42.5%, 50% of maximum health.");
        }
    }
}
