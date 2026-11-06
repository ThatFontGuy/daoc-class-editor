using System;
using System.Collections.Generic;
using System.Linq;
using DOL.AI.Brain;
using DOL.Database;
using DOL.GS.PacketHandler;
using DOL.GS.PropertyCalc;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    // Current live class RR5s that replaced the 1.108 set (sources in docs/LIVE-RA-MODERNIZATION.md).
    public abstract class ModernRr5Ability : RR5RealmAbility, IModernRealmAbility
    {
        protected ModernRr5Ability(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 600;
        protected bool CannotUse(GameLiving living) => CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED);
        protected static bool Has(GameLiving living, eEffect type) =>
            living?.effectListComponent != null && living.effectListComponent.ContainsEffectForEffectType(type);
        protected static ECSGameEffect Self(GameLiving living, int ms, double value, Func<ECSGameEffectInitParams, ECSGameEffect> make) =>
            ECSGameEffectFactory.Create(new(living, ms, value), (in ECSGameEffectInitParams i) => make(i));
    }

    // A self or group buff whose behaviour lives in the ModernRr5 combat hooks.
    public class ModernRr5Effect : ECSGameAbilityEffect
    {
        private readonly string _name;
        private readonly ushort _icon;
        private readonly Action<ModernRr5Effect> _start, _stop;
        private bool _applied;
        public ModernRr5Effect(in ECSGameEffectInitParams init, eEffect type, string name, ushort icon,
            Action<ModernRr5Effect> start = null, Action<ModernRr5Effect> stop = null) : base(init)
        {
            EffectType = type; _name = name; _icon = icon; _start = start; _stop = stop;
        }
        public override string Name => _name;
        public override ushort Icon => _icon;
        public override bool HasPositiveEffect => EffectType is not eEffect.ModernNaturesMadnessSlow and not eEffect.ModernNaturesMadnessScare;
        public int Remaining;
        public override void OnStartEffect()
        {
            if (_applied) return;
            _applied = true;
            _start?.Invoke(this);
        }
        public override void OnStopEffect()
        {
            if (!_applied) return;
            _applied = false;
            _stop?.Invoke(this);
        }
    }

    public static class ModernRr5
    {
        public static ModernRr5Effect Get(GameLiving living, eEffect type) => living?.effectListComponent == null ? null :
            living.effectListComponent.GetAbilityEffects(type).OfType<ModernRr5Effect>().FirstOrDefault(e => e.IsActive && !e.IsEnding);

        // Base weapon damage multiplier (Dissolute Swings: the Mercenary deals 25% more damage).
        public static double OutgoingFactor(GameLiving attacker) => Get(attacker, eEffect.ModernDissoluteSwings) != null ? 1 + ModernDissoluteSwings.DamageBonus : 1;

        // Runs for every attack received, hit or not.
        public static void OnAttacked(GameLiving target, AttackData ad)
        {
            if (ad == null || target?.effectListComponent == null) return;
            ModernRetributionOfTheFaithful.Retaliate(target, ad);
        }

        // Runs before damage is dealt and before ablatives.
        public static void ModifyIncoming(GameLiving target, AttackData ad)
        {
            if (ad == null || !ad.IsHit || ad.Damage + ad.CriticalDamage <= 0 || target?.effectListComponent == null) return;
            if (Get(target, eEffect.ModernSputinsImmunity) != null) { ad.Damage = 0; ad.CriticalDamage = 0; return; }
            if (Get(target, eEffect.ModernNaturesWomb) != null)
            {
                target.ChangeHealth(target, eHealthChangeType.Spell, ad.Damage + ad.CriticalDamage);
                ad.Damage = 0; ad.CriticalDamage = 0;
                return;
            }
            if (Get(target, eEffect.ModernDissoluteSwings) != null)
                Scale(ad, 1 + ModernDissoluteSwings.DamageTakenPenalty);
            if (Get(target, eEffect.ModernFueledByRage) != null)
            {
                int before = ad.Damage + ad.CriticalDamage;
                Scale(ad, 1 - ModernFueledByRage.Reduction);
                int prevented = before - (ad.Damage + ad.CriticalDamage);
                if (prevented > 1) target.ChangeHealth(target, eHealthChangeType.Spell, prevented / 2);
            }
            if (ad.AttackType == AttackData.eAttackType.Spell && Get(target, eEffect.ModernMjollnirsProtection) is { } shield && shield.Remaining > 0)
            {
                int total = ad.Damage + ad.CriticalDamage;
                int absorbed = Math.Min(shield.Remaining, total / 2);
                shield.Remaining -= absorbed;
                int fromCrit = Math.Min(ad.CriticalDamage, absorbed / 2);
                ad.CriticalDamage -= fromCrit;
                ad.Damage = Math.Max(0, ad.Damage - (absorbed - fromCrit));
                if (shield.Remaining <= 0) shield.End();
            }
            ModernSputinsLegacy.CheatDeath(target, ad);
            DOL.GS.Spells.ArtifactAbilities.CheatDeath(target, ad);
        }
        private static void Scale(AttackData ad, double factor)
        {
            ad.Damage = (int)(ad.Damage * factor);
            ad.CriticalDamage = (int)(ad.CriticalDamage * factor);
        }

        // Runs after a weapon hit has dealt its damage.
        public static void OnWeaponHit(GameLiving attacker, AttackData ad)
        {
            if (ad?.Target is not GameLiving target || !target.IsAlive || attacker?.effectListComponent == null) return;
            if (ad.AttackType == AttackData.eAttackType.Ranged && Get(attacker, eEffect.ModernFarSight) != null &&
                attacker.rangeAttackComponent?.RangedAttackType is eRangedAttackType.Normal)
                Apply(attacker, target, ModernFarSight.NearsightSpell);
            if (ad.IsMeleeAttack && Get(attacker, eEffect.ModernBestialHeart) != null)
                Apply(attacker, target, ModernBestialHeart.DiseaseSpell);
            if (Get(attacker, eEffect.ModernFuryOfNature) != null && ad.Damage + ad.CriticalDamage > 0)
                ModernFuryOfNature.SpreadHeal(attacker, ad.Damage + ad.CriticalDamage);
        }
        public static double StyleFactor(GameLiving attacker) => Get(attacker, eEffect.ModernFuryOfNature) != null ? 2 : 1;
        private static void Apply(GameLiving caster, GameLiving target, Spell spell) =>
            (ScriptMgr.CreateSpellHandler(caster, spell, GlobalSpellsLines.RealmSpellsSpellLine) as SpellHandler)?.ApplyEffectOnTarget(target);

        public static int CastTime(GameLiving caster, int castTime)
        {
            double time = castTime;
            if (Get(caster, eEffect.ModernFrenzyOfTheUnderhill) != null) time /= 2; // +100% casting speed
            if (Get(caster, eEffect.ModernNaturesMadnessSlow) != null) time /= 1 - ModernNaturesMadness.Slow;
            return (int)time;
        }
        public static int SpellRange(GameLiving caster, int range) =>
            Get(caster, eEffect.ModernFarSight) != null ? (int)(range * (1 + ModernFarSight.RangeBonus * 0.01)) : range;

        public static bool IsControlImmune(GameLiving target) => Get(target, eEffect.ModernVoiceOfSkadi) != null ||
            Get(target, eEffect.ModernResoluteMinion) != null || Get(target, eEffect.ModernSputinsImmunity) != null;
        public static bool IsNearsightImmune(GameLiving target) => Get(target, eEffect.ModernFarSightImmunity) != null;
        public static bool BlocksBow(GameLiving living) => Get(living, eEffect.ModernBestialHeart) != null;
        public static bool IsStatDebuff(ISpellHandler handler) => handler is SingleStatDebuff debuff &&
            (debuff.Property1 is >= eProperty.Stat_First and <= eProperty.Stat_Last || debuff.Property1 == eProperty.Acuity);
        public static bool BlocksStatDebuff(GameLiving target, ISpellHandler handler) =>
            IsStatDebuff(handler) && Get(target, eEffect.ModernDissoluteSwings) != null;
    }

    public class ModernDissoluteSwings : ModernRr5Ability
    {
        public const double DamageBonus = 0.25, DamageTakenPenalty = 0.25;
        public ModernDissoluteSwings(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || Has(living, eEffect.ModernDissoluteSwings)) return;
            SendCasterSpellEffectAndCastMessage(living, 7072, true);
            Self(living, 30000, 1, i => new ModernRr5Effect(i, eEffect.ModernDissoluteSwings, "Dissolute Swings", 3039, e => ClearStatDebuffs(e.Owner)));
            DisableSkill(living);
        }
        public static void ClearStatDebuffs(GameLiving owner)
        {
            foreach (ECSGameSpellEffect effect in owner.effectListComponent.GetEffects().OfType<ECSGameSpellEffect>()
                .Where(e => !e.IsEnding && ModernRr5.IsStatDebuff(e.SpellHandler)).ToArray())
                effect.End();
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("30 seconds: you deal 25% more damage and take 25% more damage from all sources. Clears stat debuffs and blocks new ones.");
    }

    public class ModernFueledByRage : ModernRr5Ability
    {
        public const double Reduction = 0.20;
        public ModernFueledByRage(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || Has(living, eEffect.ModernFueledByRage)) return;
            SendCasterSpellEffectAndCastMessage(living, 7068, true);
            Self(living, 30000, 1, i => new ModernRr5Effect(i, eEffect.ModernFueledByRage, "Fueled by Rage", 3068));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("30 seconds: 20% less damage taken; half of the prevented damage heals you.");
    }

    public class ModernFrenzyOfTheUnderhill : ModernRr5Ability
    {
        public const int Haste = 50; // NPC melee speed buff halving the swing interval: +100% attack speed.
        public ModernFrenzyOfTheUnderhill(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 300;
        public override void Execute(GameLiving living)
        {
            GameNPC pet = living.ControlledBrain?.Body;
            if (CannotUse(living) || pet == null || !pet.IsAlive || Has(pet, eEffect.ModernFrenzyOfTheUnderhill)) return;
            SendCasterSpellEffectAndCastMessage(living, 7051, true);
            Self(pet, 30000, 1, i => new ModernRr5Effect(i, eEffect.ModernFrenzyOfTheUnderhill, "Frenzy of the Underhill", 3063,
                e => e.Owner.BaseBuffBonusCategory[eProperty.MeleeSpeed] += Haste,
                e => e.Owner.BaseBuffBonusCategory[eProperty.MeleeSpeed] -= Haste));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("30 seconds: your pet attacks and casts 100% faster.");
    }

    public class ModernRestorativeMend : ModernRr5Ability
    {
        public const int Percent = 15, Ticks = 10, Range = 2000;
        public ModernRestorativeMend(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7076, true);
            foreach (GameLiving target in ModernGroupRealmAbility.Members(living, Range))
            {
                target.effectListComponent.GetAbilityEffects(eEffect.ModernRestorativeMend).FirstOrDefault()?.End();
                ECSGameEffectFactory.Create(new(target, Ticks * 2000, Percent), static (in i) => new ModernRestorativeMendEffect(i));
            }
            DisableSkill(living);
        }
        public static void Tick(GameLiving target)
        {
            target.ChangeHealth(target, eHealthChangeType.Regenerate, target.MaxHealth * Percent / 100);
            target.ChangeMana(target, eManaChangeType.Regenerate, target.MaxMana * Percent / 100);
            target.ChangeEndurance(target, eEnduranceChangeType.Regenerate, target.MaxEndurance * Percent / 100);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Group within 2000: restores 15% health, power and endurance every 2 seconds for 20 seconds, in or out of combat.");
    }

    public class ModernRestorativeMendEffect : ECSGameAbilityEffect
    {
        private int _ticks;
        public ModernRestorativeMendEffect(in ECSGameEffectInitParams init) : base(init)
        {
            EffectType = eEffect.ModernRestorativeMend; PulseFreq = 2000; NextTick = StartTick;
        }
        public override string Name => "Restorative Mend";
        public override ushort Icon => 3070;
        public override bool HasPositiveEffect => true;
        public override void OnEffectPulse()
        {
            if (!Owner.IsAlive || _ticks >= ModernRestorativeMend.Ticks) return;
            _ticks++;
            ModernRestorativeMend.Tick(Owner);
        }
    }

    public class ModernFarSight : ModernRr5Ability
    {
        public const int RangeBonus = 20;
        public static readonly Spell NearsightSpell = new(new DbSpell { Name = "Far Sight", Type = eSpellType.Nearsight.ToString(),
            Target = eSpellTarget.ENEMY.ToString(), Value = 50, Duration = 20, ClientEffect = 7049, Icon = 7049 }, 50);
        public ModernFarSight(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            living.Stealth(false);
            SendCasterSpellEffectAndCastMessage(living, 7049, true);
            foreach (ECSGameEffect nearsight in living.effectListComponent.GetEffects(eEffect.Nearsight).ToArray())
                nearsight.End();
            Self(living, 60000, 1, i => new ModernRr5Effect(i, eEffect.ModernFarSightImmunity, "Far Sight Immunity", 3045));
            Self(living, 30000, 1, i => new ModernRr5Effect(i, eEffect.ModernFarSight, "Far Sight", 3045,
                e => e.Owner.AbilityBonus[eProperty.ArcheryRange] += RangeBonus,
                e => e.Owner.AbilityBonus[eProperty.ArcheryRange] -= RangeBonus));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Clears nearsight and grants 60 seconds of nearsight immunity.");
            list.Add("30 seconds: +20% archery and spell range, and standard shots apply a 20-second 50% nearsight. Removes stealth.");
        }
    }

    public class ModernBestialHeart : ModernRr5Ability
    {
        public const ushort WolfModel = 649;
        public const int MeleeDamage = 15, Celerity = 40;
        public const double RunSpeed = 2.15;
        public static readonly Spell DiseaseSpell = new(new DbSpell { Name = "Bestial Heart", Type = eSpellType.Disease.ToString(),
            Target = eSpellTarget.ENEMY.ToString(), Duration = 20, ClientEffect = 4375, Icon = 4375 }, 50);
        public ModernBestialHeart(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || Has(living, eEffect.ModernBestialHeart)) return;
            living.Stealth(false);
            if (living.ActiveWeaponSlot == eActiveWeaponSlot.Distance) living.attackComponent.StopAttack();
            SendCasterSpellEffectAndCastMessage(living, 7059, true);
            Self(living, 30000, 1, i => new ModernRr5Effect(i, eEffect.ModernBestialHeart, "Bestial Heart", 3071, Start, Stop));
            DisableSkill(living);
        }
        private static void Start(ModernRr5Effect e)
        {
            GameLiving owner = e.Owner;
            owner.AbilityBonus[eProperty.MeleeDamage] += MeleeDamage;
            owner.BaseBuffBonusCategory[eProperty.MeleeSpeed] += Celerity;
            owner.BuffBonusMultCategory1.Set((int)eProperty.MaxSpeed, e, RunSpeed);
            if (owner is GamePlayer player) { player.Model = WolfModel; player.Out.SendUpdateMaxSpeed(); }
        }
        private static void Stop(ModernRr5Effect e)
        {
            GameLiving owner = e.Owner;
            owner.AbilityBonus[eProperty.MeleeDamage] -= MeleeDamage;
            owner.BaseBuffBonusCategory[eProperty.MeleeSpeed] -= Celerity;
            owner.BuffBonusMultCategory1.Remove((int)eProperty.MaxSpeed, e);
            if (owner is GamePlayer player) { player.Model = (ushort)player.CreationModel; player.Out.SendUpdateMaxSpeed(); }
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("30 seconds as a Dire Wolf: +15% melee damage, +40% attack speed, +115% run speed, melee hits inflict disease. You cannot use your bow. Removes stealth.");
    }

    public class ModernNaturesMadness : ModernRr5Ability
    {
        public const int Radius = 500, Duration = 30000, Pulse = 2000;
        public const double Slow = 0.30;
        public ModernNaturesMadness(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.CurrentRegion == null) return;
            living.Stealth(false);
            SendCasterSpellEffectAndCastMessage(living, 7024, true);
            var storm = new ModernNaturesMadnessStorm(living);
            if (storm.AddToWorld()) storm.Begin();
            DisableSkill(living);
        }
        public static bool IsEnemyPet(GameLiving caster, GameLiving target) => target is GameNPC { Brain: IControlledBrain };
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("A 30-second storm at your feet: enemies within 500 attack and cast 30% slower, and enemy pets inside flee in terror. Removes stealth.");
    }

    public class ModernNaturesMadnessStorm : GameStaticItem
    {
        private readonly GameLiving _caster;
        private readonly long _ends;
        private ECSGameTimer _timer;
        public ModernNaturesMadnessStorm(GameLiving caster)
        {
            _caster = caster; _ends = GameLoop.GameLoopTime + ModernNaturesMadness.Duration;
            Name = "Nature's Madness"; Model = 3457; Realm = caster.Realm; CurrentRegion = caster.CurrentRegion;
            X = caster.X; Y = caster.Y; Z = caster.Z; Heading = caster.Heading;
        }
        public void Begin()
        {
            _timer = new ECSGameTimer(this, _ => PulseOnce(), 1);
        }
        private int PulseOnce()
        {
            if (GameLoop.GameLoopTime >= _ends || _caster.ObjectState != eObjectState.Active || _caster.CurrentRegion != CurrentRegion)
            {
                Delete();
                return 0;
            }
            foreach (GamePlayer player in GetPlayersInRadius(ModernNaturesMadness.Radius))
                Affect(player);
            foreach (GameNPC npc in GetNPCsInRadius(ModernNaturesMadness.Radius))
                Affect(npc);
            return ModernNaturesMadness.Pulse;
        }
        private void Affect(GameLiving target)
        {
            if (!target.IsAlive || !GameServer.ServerRules.IsAllowedToAttack(_caster, target, true)) return;
            Refresh(target, eEffect.ModernNaturesMadnessSlow, "Nature's Madness",
                e => e.Owner.DebuffCategory[eProperty.MeleeSpeed] += (int)(ModernNaturesMadness.Slow * 100),
                e => e.Owner.DebuffCategory[eProperty.MeleeSpeed] -= (int)(ModernNaturesMadness.Slow * 100));
            if (ModernNaturesMadness.IsEnemyPet(_caster, target) && target is GameNPC pet)
            {
                Refresh(target, eEffect.ModernNaturesMadnessScare, "Nature's Madness Terror", null, null);
                pet.attackComponent.StopAttack();
                pet.StopFollowing();
                double angle = Math.Atan2(pet.Y - Y, pet.X - X);
                pet.WalkTo(new Point3D(X + (int)(Math.Cos(angle) * 900), Y + (int)(Math.Sin(angle) * 900), pet.Z), pet.MaxSpeed);
            }
        }
        // Each pulse renews a short effect, so leaving the storm ends it quickly.
        private static void Refresh(GameLiving target, eEffect type, string name, Action<ModernRr5Effect> start, Action<ModernRr5Effect> stop)
        {
            if (ModernRr5.Get(target, type) is { } existing) existing.End();
            ECSGameEffectFactory.Create(new(target, ModernNaturesMadness.Pulse + 1000, 1),
                (in ECSGameEffectInitParams i) => new ModernRr5Effect(i, type, name, 3060, start, stop));
        }
        public override void Delete()
        {
            _timer?.Stop();
            _timer = null;
            base.Delete();
        }
    }

    public class ModernVoiceOfSkadi : ModernRr5Ability
    {
        public ModernVoiceOfSkadi(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 300;
        public override void Execute(GameLiving living)
        {
            if (Has(living, eEffect.ModernVoiceOfSkadi) || CheckPreconditions(living, DEAD | SITTING)) return;
            SendCasterSpellEffectAndCastMessage(living, 7066, true);
            Self(living, 15000, 1, i => new ModernRr5Effect(i, eEffect.ModernVoiceOfSkadi, "Voice of Skadi", 3065));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("15 seconds of immunity to crowd control.");
    }

    public class ModernMjollnirsProtection : ModernRr5Ability
    {
        public const int Pool = 1500, Range = 2000;
        public ModernMjollnirsProtection(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7045, true);
            foreach (GameLiving target in ModernGroupRealmAbility.Members(living, Range))
            {
                ModernRr5.Get(target, eEffect.ModernMjollnirsProtection)?.End();
                ECSGameEffectFactory.Create(new(target, 60000, 1),
                    static (in ECSGameEffectInitParams i) => new ModernRr5Effect(i, eEffect.ModernMjollnirsProtection, "Mjollnir's Protection", 3066,
                        e => e.Remaining = Pool));
            }
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Group within 2000: for 60 seconds, absorbs 50% of spell damage taken, up to 1500 damage each.");
    }
}
