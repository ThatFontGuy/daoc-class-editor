using System;
using System.Collections.Generic;
using System.Linq;
using DOL.Database;
using DOL.GS.Effects;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    public class ModernDualThreat : RAPropertyEnhancer, IModernRealmAbility
    {
        public ModernDualThreat(DbAbility ability, int level) : base(ability, level,
            new[] { eProperty.CriticalMeleeHitChance, eProperty.CriticalSpellHitChance }) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int current) => ModernRealmRanks.ActiveCost(current);
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Value(rank, 5, 7, 10, 15, 20);
        protected override string ValueUnit => "%";
    }
    public class ModernViper : ViperAbility, IModernRealmAbility
    {
        public ModernViper(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int current) => ModernRealmRanks.ActiveCost(current);
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Value(rank, 25, 35, 50, 75, 100);
        public static double PoisonMultiplier(GameLiving owner, bool poison) => poison ? 1 + (owner.GetAbility<ModernViper>()?.Amount ?? 0) * 0.01 : 1;
    }
    public class ModernReflexAttack : XReflexAttackAbility, IModernRealmAbility
    {
        public ModernReflexAttack(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int current) => ModernRealmRanks.ActiveCost(current);
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Value(rank, 10, 20, 30, 40, 50);
        public static bool Eligible(AttackData hit) => hit.IsMeleeAttack &&
            hit.AttackResult is eAttackResult.HitStyle or eAttackResult.HitUnstyled && hit.Damage + hit.CriticalDamage > 0;
        public static bool ShouldCounter(GameLiving owner, AttackData hit) => owner.IsAlive && Eligible(hit) &&
            owner.GetAbility<ModernReflexAttack>() is { Amount: > 0 } ability && owner.Chance(RandomDeckEvent.OffensiveProcChance, ability.Amount);
    }
    public class ModernMasteryOfConcentration : ModernGroupRealmAbility
    {
        public ModernMasteryOfConcentration(DbAbility ability, int level) : base(ability, level) { }
        public static int Amount(int rank) => ModernRealmRanks.Value(rank, 25, 35, 50, 60, 75);
        public static double Factor(GameLiving caster)
        {
            GameLiving owner = caster is NecromancerPet pet ? pet.Owner : caster;
            if (owner?.effectListComponent == null) return 1;
            return owner.effectListComponent.GetAbilityEffects(eEffect.MasteryOfConcentration)
                .OfType<ModernConcentrationEffect>().FirstOrDefault(e => e.IsActive && !e.IsEnding)?.Effectiveness ?? 1;
        }
        public static bool IsControl(Spell spell) => EffectHelper.GetEffectFromSpell(spell) is
            eEffect.Mez or eEffect.Stun or eEffect.MovementSpeedDebuff or eEffect.Nearsight;
        public static double SpellFactor(GameLiving caster, SpellLine line) => line?.KeyName is
            GlobalSpellsLines.Item_Effects or GlobalSpellsLines.Potions_Effects or GlobalSpellsLines.Mundane_Poisons or
            GlobalSpellsLines.Combat_Styles_Effect or GlobalSpellsLines.Realm_Spells ? 1 : Factor(caster);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.effectListComponent.ContainsEffectForEffectType(eEffect.MasteryOfConcentration) ||
                living.EffectList.GetOfType<ShieldOfImmunityEffect>() != null) return;
            ECSGameEffectFactory.Create(new(living, 30000, Amount(Level) * 0.01), static (in i) => new ModernConcentrationEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("30 seconds of uninterrupted casting at 25/35/50/60/75% spell effectiveness. Necromancer pets inherit the effect.");
    }
    public class ModernConcentrationEffect : MasteryOfConcentrationECSEffect
    {
        public ModernConcentrationEffect(in ECSGameEffectInitParams init) : base(init) { }
    }
    public class ModernJuggernaut : ModernGroupRealmAbility
    {
        public ModernJuggernaut(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => 900;
        public static int Amount(int rank) => ModernRealmRanks.Value(rank, 10, 15, 20, 25, 30);
        public static bool HasSummonMastery(GameLiving pet) => SpellHandler.FindEffectOnTarget(pet, "SummonMastery") != null ||
            pet.effectListComponent.GetSpellEffects(eEffect.MasterLevel).Any(effect => !effect.IsEnding && !effect.IsDisabled &&
                effect.SpellHandler is MasterLevelAbilityHandler { Ability.Path: "Convoker", Ability.Level: 9 });
        public override void Execute(GameLiving living)
        {
            GameNPC pet = living.ControlledBrain?.Body;
            if (CannotUse(living) || pet == null || !pet.IsAlive || pet.CurrentRegion != living.CurrentRegion || !living.IsWithinRadius(pet, 1500) ||
                pet.effectListComponent.ContainsEffectForEffectType(eEffect.Juggernaut) || HasSummonMastery(pet)) return;
            ECSGameEffectFactory.Create(new(pet, 60000, Amount(Level)), static (in i) => new ModernJuggernautEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Raises your pet by 10/15/20/25/30 levels for 60 seconds, capped at level 70. Cannot stack with Summon Mastery.");
    }
    public class ModernJuggernautEffect : ECSGameAbilityEffect
    {
        private byte _level, _size;
        private bool _applied;
        public ModernJuggernautEffect(in ECSGameEffectInitParams init) : base(init) { EffectType = eEffect.Juggernaut; }
        public override string Name => "Juggernaut";
        public override ushort Icon => 3030;
        public override bool HasPositiveEffect => true;
        public override void OnStartEffect()
        {
            if (_applied || Owner is not GameNPC pet) return;
            _level = pet.Level; _size = pet.Size; _applied = true;
            // A pet already above the cap must never lose levels from this ability.
            pet.Level = (byte)Math.Max(_level, Math.Min(70, _level + (int)Effectiveness));
            pet.Size = (byte)Math.Min(255, _size + 15);
        }
        public override void OnStopEffect()
        {
            if (!_applied || Owner is not GameNPC pet) return;
            pet.Level = _level; pet.Size = _size; _applied = false;
        }
    }
    public class ModernStrikePrediction : ModernGroupRealmAbility
    {
        public ModernStrikePrediction(DbAbility ability, int level) : base(ability, level) { }
        public static int Amount(int rank) => ModernRealmRanks.Value(rank, 5, 7, 10, 15, 20);
        public static int MinimumChance(GameLiving living, int normal) => living.effectListComponent == null ? normal : Math.Max(normal,
            (int)(living.effectListComponent.GetAbilityEffects(eEffect.ModernStrikePrediction)
                .OfType<ModernStrikePredictionEffect>().FirstOrDefault(e => e.IsActive && !e.IsEnding)?.Effectiveness ?? 0) * 10);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            bool used = false;
            foreach (GameLiving target in Members(living, 2000))
            {
                if (target.effectListComponent.ContainsEffectForEffectType(eEffect.ModernStrikePrediction)) continue;
                ECSGameEffectFactory.Create(new(target, 30000, Amount(Level)), static (in i) => new ModernStrikePredictionEffect(i)); used = true;
            }
            if (used) DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("30-second group minimum evade chance: 5/7/10/15/20%. Does not add to a higher existing evade chance.");
    }
    public class ModernStrikePredictionEffect : ECSGameAbilityEffect
    {
        public ModernStrikePredictionEffect(in ECSGameEffectInitParams init) : base(init) { EffectType = eEffect.ModernStrikePrediction; }
        public override string Name => "Strike Prediction";
        public override ushort Icon => 7037;
        public override bool HasPositiveEffect => true;
    }
}
