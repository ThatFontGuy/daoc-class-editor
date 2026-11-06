using System;
using System.Collections.Generic;
using System.Linq;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    public abstract class ModernGroupRealmAbility : TimedRealmAbility, IModernRealmAbility
    {
        protected ModernGroupRealmAbility(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.ActiveCost(currentLevel);
        public override int GetReUseDelay(int rank) => 600;
        public static IEnumerable<GameLiving> Members(GameLiving caster, int range, bool includeCaster = true)
        {
            var members = caster.Group?.GetMembersInTheGroup() ?? new List<GameLiving> { caster };
            return members.Where(target => target != null && (includeCaster || target != caster) && target.IsAlive &&
                target.CurrentRegion == caster.CurrentRegion && caster.IsWithinRadius(target, range)).ToArray();
        }
        protected bool CannotUse(GameLiving living) => Level < 1 || Level > MaxLevel ||
            CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED);
    }

    public enum ModernRealmBuff { Fortitude, Aura, Barricade, EmptyMind }

    public class ModernRealmBuffEffect : ECSGameAbilityEffect
    {
        private readonly ModernRealmBuff _kind;
        private readonly string _name;
        private readonly ushort _icon;
        private readonly int _amount;
        private bool _applied;
        public static readonly eProperty[] MagicResists = { eProperty.Resist_Body, eProperty.Resist_Cold, eProperty.Resist_Energy,
            eProperty.Resist_Heat, eProperty.Resist_Matter, eProperty.Resist_Spirit };
        public static readonly eProperty[] PhysicalResists = { eProperty.Resist_Crush, eProperty.Resist_Slash, eProperty.Resist_Thrust };
        public const string BarrierLock = BarrierOfFortitudeAbility.BofBaSb;
        public ModernRealmBuffEffect(in ECSGameEffectInitParams init, ModernRealmBuff kind, string name, ushort icon) : base(init)
        {
            _kind = kind; _name = name; _icon = icon; _amount = (int)init.Effectiveness;
            EffectType = kind == ModernRealmBuff.EmptyMind ? eEffect.ModernRealmMind : eEffect.ModernRealmBarrier;
        }
        public override string Name => _name;
        public override ushort Icon => _icon;
        public override bool HasPositiveEffect => true;
        public static void Modify(GameLiving owner, ModernRealmBuff kind, int amount)
        {
            if (kind == ModernRealmBuff.Fortitude)
                owner.AbilityBonus[eProperty.PhysicalAbsorption] += amount;
            else
            {
                // Damage-only secondary resists must not shorten crowd control.
                foreach (eProperty property in MagicResists)
                    owner.OtherBonus[property] += amount;
                if (kind == ModernRealmBuff.Barricade)
                    foreach (eProperty property in PhysicalResists)
                        owner.OtherBonus[property] += amount;
            }
        }
        public override void OnStartEffect()
        {
            lock (Owner.TempProperties)
            {
                if (_applied || (_kind != ModernRealmBuff.EmptyMind && Owner.TempProperties.GetProperty<bool>(BarrierLock)))
                    return;
                _applied = true;
                if (_kind != ModernRealmBuff.EmptyMind)
                    Owner.TempProperties.SetProperty(BarrierLock, true);
                Modify(Owner, _kind, _amount);
            }
            Refresh();
        }
        public override void OnStopEffect()
        {
            lock (Owner.TempProperties)
            {
                if (!_applied)
                    return;
                Modify(Owner, _kind, -_amount);
                _applied = false;
                if (_kind != ModernRealmBuff.EmptyMind)
                    Owner.TempProperties.RemoveProperty(BarrierLock);
            }
            Refresh();
        }
        private void Refresh() => Owner.effectListComponent.RequestPlayerUpdate(EffectHelper.PlayerUpdate.Resists | EffectHelper.PlayerUpdate.WeaponArmor);
    }

    public abstract class ModernRealmBarrier : ModernGroupRealmAbility
    {
        protected ModernRealmBarrier(DbAbility ability, int level) : base(ability, level) { }
        protected abstract ModernRealmBuff Kind { get; }
        public virtual int Amount(int rank) => ModernRealmRanks.Value(rank, 10, 15, 20, 30, 40);
        protected virtual ushort EffectIcon => 7132;
        protected virtual int Duration => 30000;
        protected virtual IEnumerable<GameLiving> Targets(GameLiving living) => Members(living, 1500);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            bool used = false;
            foreach (GameLiving target in Targets(living))
            {
                if (Kind != ModernRealmBuff.EmptyMind && (target.TempProperties.GetProperty<bool>(ModernRealmBuffEffect.BarrierLock) ||
                    target.effectListComponent.ContainsEffectForEffectType(eEffect.ModernRealmBarrier))) continue;
                if (Kind == ModernRealmBuff.EmptyMind && target.effectListComponent.ContainsEffectForEffectType(eEffect.ModernRealmMind)) continue;
                ECSGameEffectFactory.Create(new(target, Duration, Amount(Level)),
                    (in ECSGameEffectInitParams init) => new ModernRealmBuffEffect(init, Kind, m_name, EffectIcon));
                used = true;
            }
            if (used) DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add($"Duration: {Duration / 1000} seconds.");
            for (int rank = 1; rank <= MaxLevel; rank++) list.Add($"Rank {rank}: {Amount(rank)}% damage protection.");
            if (Kind != ModernRealmBuff.EmptyMind) list.Add("Does not stack with Barrier of Fortitude, Bedazzling Aura or Soldier's Barricade.");
        }
    }
    public class ModernBunkerOfFaith : ModernRealmBarrier
    {
        public ModernBunkerOfFaith(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernRealmBuff Kind => ModernRealmBuff.Fortitude;
    }
    public class ModernBrilliantAura : ModernRealmBarrier
    {
        public ModernBrilliantAura(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernRealmBuff Kind => ModernRealmBuff.Aura;
        protected override ushort EffectIcon => 7030;
    }
    public class ModernSoldiersBarricade : ModernRealmBarrier
    {
        public ModernSoldiersBarricade(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernRealmBuff Kind => ModernRealmBuff.Barricade;
        public override int Amount(int rank) => ModernRealmRanks.Value(rank, 5, 10, 15, 20, 30);
        protected override ushort EffectIcon => 7015;
    }
    public class ModernEmptyMind : ModernRealmBarrier
    {
        public ModernEmptyMind(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernRealmBuff Kind => ModernRealmBuff.EmptyMind;
        public override int Amount(int rank) => ModernRealmRanks.Value(rank, 10, 15, 20, 25, 30);
        protected override int Duration => 45000;
        protected override ushort EffectIcon => 7122;
        protected override IEnumerable<GameLiving> Targets(GameLiving living) => new[] { living };
    }

    public class ModernVehementRenewal : ModernGroupRealmAbility
    {
        public ModernVehementRenewal(DbAbility ability, int level) : base(ability, level) { }
        public int Amount(int rank) => ModernRealmRanks.Value(rank, 375, 525, 750, 1125, 1500);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            bool used = false;
            foreach (GameLiving target in Members(living, 2000, false))
                used |= target.ChangeHealth(living, eHealthChangeType.Spell, Amount(Level)) > 0;
            if (used) { SendCasterSpellEffectAndCastMessage(living, 7017, true); DisableSkill(living); }
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Heals other living group members within 2000 units for 375/525/750/1125/1500 health.");
    }
    public class ModernAmelioratingMelodies : ModernGroupRealmAbility
    {
        public ModernAmelioratingMelodies(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => 900;
        public int Amount(int rank) => ModernRealmRanks.Value(rank, 100, 175, 250, 325, 400);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || !Members(living, 1500, false).Any() ||
                living.effectListComponent.ContainsEffectForEffectType(eEffect.ModernRealmMelodies)) return;
            ECSGameEffectFactory.Create(new(living, 30000, Amount(Level)), static (in i) => new ModernRealmMelodiesEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Heals other living group members within 1500 units ten times over 30 seconds: 100/175/250/325/400 per tick.");
    }
    public class ModernRealmMelodiesEffect : ECSGameAbilityEffect
    {
        private int _ticks;
        public ModernRealmMelodiesEffect(in ECSGameEffectInitParams init) : base(init)
        {
            EffectType = eEffect.ModernRealmMelodies; PulseFreq = 3000; NextTick = StartTick;
        }
        public override string Name => "Ameliorating Melodies";
        public override ushort Icon => 4250;
        public override bool HasPositiveEffect => true;
        public override void OnEffectPulse()
        {
            if (!Owner.IsAlive || _ticks >= 10) return;
            _ticks++;
            foreach (GameLiving target in ModernGroupRealmAbility.Members(Owner, 1500, false))
                target.ChangeHealth(Owner, eHealthChangeType.Regenerate, (int)Effectiveness);
        }
    }

    public class ModernAngerOfTheGods : ModernGroupRealmAbility
    {
        public ModernAngerOfTheGods(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 600, 525, 450, 375, 300);
        public static double Damage(int rank) => rank <= 0 ? 0 : new[] { 15.0, 22.5, 30.0, 37.5, 45.0 }[Math.Min(rank, 5) - 1];
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            var spell = new Spell(new DbSpell { Name = "Anger of the Gods", Type = "DamageAdd", Target = "Realm", Range = 1000, Duration = 30,
                Damage = Damage(Level), EffectGroup = 99999, ClientEffect = 7141, Icon = 7141 }, 0);
            foreach (GameLiving target in Members(living, 1000))
                new ModernAngerDamageAdd(living, spell, GlobalSpellsLines.RealmSpellsSpellLine).ApplyEffectOnTarget(target);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Group damage add for 30 seconds: 15/22.5/30/37.5/45 DPS. Stacks with ordinary damage adds.");
    }

    public class ModernAngerDamageAdd : DamageAddSpellHandler
    {
        public ModernAngerDamageAdd(GameLiving caster, Spell spell, SpellLine line) : base(caster, spell, line) { }
        public override bool BuffReceivesSpecBonus => false;
        protected override int CalculateEffectDuration(GameLiving target) => 30000;
        public static double SwingDamage(double dps, int interval) => dps * Math.Max(0, interval) * 0.001;
        public override void Handle(AttackData hit, double effectiveness)
        {
            if (!AreArgumentsValid(hit, out GameLiving attacker, out GameLiving target)) return;
            AttackData extra = CreateAttackData(SwingDamage(Spell.Damage, hit.Interval), attacker, target);
            target.OnAttackedByEnemy(extra);
            attacker.DealDamage(extra);
        }
    }
    public class ModernFuryOfTheGods : ModernAngerOfTheGods
    {
        public ModernFuryOfTheGods(DbAbility ability, int level) : base(ability, level) { }
    }
}
