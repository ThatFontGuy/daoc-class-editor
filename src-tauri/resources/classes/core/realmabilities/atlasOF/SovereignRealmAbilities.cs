using System;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    /// <summary>Purchased Sovereign passives; three ranks, incremental costs 2/4/6.</summary>
    public abstract class SovereignPassive : RAPropertyEnhancer
    {
        protected SovereignPassive(DbAbility ability, int level, params eProperty[] properties)
            : base(ability, level, properties) { }
        public override int MaxLevel => 3;
        public override int CostForUpgrade(int currentLevel) => currentLevel is >= 0 and < 3 ? (currentLevel + 1) * 2 : 1000;
        public override bool CheckRequirement(GamePlayer player) => player.CharacterClass != null && PendragonRules.IsSovereign(player.CharacterClass.ID);
        protected override string ValueUnit => "%";
        protected override void SendUpdates(GameLiving target)
        {
            if (target.Health > target.MaxHealth)
                target.Health = target.MaxHealth;
            MLMod.Refresh(target);
        }
    }

    public sealed class SovereignRoyalFortitude : SovereignPassive
    {
        public SovereignRoyalFortitude(DbAbility ability, int level) : base(ability, level, eProperty.Of_Toughness) { }
        public override int GetAmountForLevel(int level) => Math.Clamp(level, 0, 3) * 3;
    }

    public sealed class SovereignStormMastery : SovereignPassive
    {
        public SovereignStormMastery(DbAbility ability, int level) : base(ability, level, eProperty.MeleeDamage, eProperty.SpellDamage) { }
        public override int GetAmountForLevel(int level) => Math.Clamp(level, 0, 3) * 2;
    }

    public sealed class SovereignCrownsResolve : SovereignPassive
    {
        public SovereignCrownsResolve(DbAbility ability, int level) : base(ability, level, MLMod.AllResists) { }
        public override int GetAmountForLevel(int level) => Math.Clamp(level, 0, 3) * 2;
    }

    /// <summary>The existing Realm Abilities career automatically grants this free at RR5.</summary>
    public sealed class SovereignWrathOfTheCrown : RR5RealmAbility
    {
        public SovereignWrathOfTheCrown(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 300;
        public override bool CheckRequirement(GamePlayer player) =>
            player.CharacterClass != null && PendragonRules.IsSovereign(player.CharacterClass.ID) && base.CheckRequirement(player);

        public override void Execute(GameLiving living)
        {
            if (living is not GamePlayer player || !CheckRequirement(player) ||
                living.GetSkillDisabledDuration(this) > 0 || CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED))
                return;
            Spell spell = SkillBase.GetSpellByID(StormSovereignAbilities.RR5SpellId + PendragonRules.RealmOffset(player));
            if (spell == null)
                return;
            // Starting the registered effect directly avoids adding a second spell cooldown.
            DisableSkill(living);
            living.Stealth(false);
            new StormSovereignHandler(living, spell, GlobalSpellsLines.RealmSpellsSpellLine).StartSpell(living);
        }
    }
}
