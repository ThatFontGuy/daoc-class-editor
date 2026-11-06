using System;
using System.Collections.Generic;
using System.Linq;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    public enum ModernStormKind { Maelstrom, Thornweed, Tempest }

    public static class ModernRealmCast
    {
        private const string Pending = "ModernRealmAbilityCast";
        public static bool CanContinue(GameLiving caster, Region region, Point3D origin) => caster.IsAlive &&
            caster.ObjectState == GameObject.eObjectState.Active && caster.CurrentRegion == region && !caster.IsMoving &&
            !caster.IsSitting && !caster.IsCrowdControlled && caster.X == origin.X && caster.Y == origin.Y && caster.Z == origin.Z;
        public static bool Begin(GameLiving caster, int milliseconds, ushort animation, Func<bool> valid, Action complete)
        {
            var player = caster as GamePlayer;
            if (caster.IsMoving || caster.CurrentRegion == null || player?.RealmAbilityCastTimer?.IsAlive == true ||
                caster.TempProperties.GetProperty<ECSGameTimer>(Pending)?.IsAlive == true) return false;
            var region = caster.CurrentRegion;
            var origin = new Point3D(caster.X, caster.Y, caster.Z);
            long finish = GameLoop.GameLoopTime + milliseconds;
            caster.StopCurrentSpellcast(); caster.attackComponent.StopAttack(); caster.Stealth(false);
            foreach (GamePlayer viewer in caster.GetPlayersInRadius(WorldMgr.VISIBILITY_DISTANCE))
                viewer.Out.SendSpellCastAnimation(caster, animation, (ushort)(milliseconds / 100));
            var cast = new ECSGameTimer(caster);
            cast.Callback = timer =>
            {
                bool owns = player == null || player.RealmAbilityCastTimer == timer;
                bool ready = owns && CanContinue(caster, region, origin) && valid();
                if (ready && GameLoop.GameLoopTime < finish) return 100;
                if (player != null && owns) player.RealmAbilityCastTimer = null;
                if (caster.TempProperties.GetProperty<ECSGameTimer>(Pending) == timer) caster.TempProperties.RemoveProperty(Pending);
                if (ready) complete();
                return 0;
            };
            caster.TempProperties.SetProperty(Pending, cast);
            if (player != null) player.RealmAbilityCastTimer = cast;
            cast.Start(100);
            return true;
        }
    }

    public class ModernRealmRoot : SpeedDecreaseSpellHandler
    {
        public ModernRealmRoot(GameLiving caster, Spell spell) : base(caster, spell, GlobalSpellsLines.RealmSpellsSpellLine) { }
        protected override int CalculateEffectDuration(GameLiving target) => Spell.Duration;
        protected override double GetDebuffEffectivenessCriticalModifier() => 1;
        public override void ApplyEffectOnTarget(GameLiving target)
        {
            if (target == null || !target.IsAlive || target.effectListComponent.ContainsEffectForEffectType(eEffect.Charge) ||
                target.effectListComponent.ContainsEffectForEffectType(eEffect.Shade) || ChampionAbilities.IsControlImmune(target, Spell)) return;
            base.ApplyEffectOnTarget(target);
        }
        public override ECSGameSpellEffect CreateECSEffect(in ECSGameEffectInitParams init)
        {
            var effect = ECSGameEffectFactory.Create(init, static (in i) => new ModernRealmRootEffect(i));
            // Thornweed's short snare must keep refreshing without producing root immunity.
            if (Spell.Value < 99) effect.TriggersImmunity = false;
            return effect;
        }
    }
    public class ModernRealmRootEffect : StatDebuffECSEffect
    {
        public ModernRealmRootEffect(in ECSGameEffectInitParams init) : base(init)
        {
            // Ordinary snares decay after half their duration. Ichor's fixed root
            // and Thornweed's refreshed field snare keep their stated strength.
            PulseFreq = 0;
            NextTick = 0;
        }
        public override void TryApplyImmunity()
        {
            // Ichor grants normal root immunity on every affected actor, including
            // NPC-backed bots. The generic spell implementation only handles players.
            if (TriggersImmunity && SpellHandler.Spell.Value >= 99)
                ECSGameEffectFactory.Create(new(Owner, ImmunityDuration, 1, SpellHandler),
                    static (in i) => new ECSImmunityEffect(i, 0));
        }
    }
    public class ModernTempestStun : UnresistableStunSpellHandler
    {
        public ModernTempestStun(GameLiving caster, Spell spell) : base(caster, spell, GlobalSpellsLines.RealmSpellsSpellLine) { }
        protected override bool CheckSpellResist(GameLiving target) => target.HasAbility(Abilities.CCImmunity) ||
            target.HasAbility(Abilities.StunImmunity) || target.IsStunned || ChampionAbilities.IsControlImmune(target, Spell) ||
            target.effectListComponent.ContainsEffectForEffectType(eEffect.Charge) ||
            target.effectListComponent.ContainsEffectForEffectType(eEffect.SpeedOfSound) ||
            ModernRr5.IsControlImmune(target) ||
            target.effectListComponent.ContainsEffectForEffectType(eEffect.Shade);
    }

    public class ModernRealmStorm : GameStaticItem
    {
        private readonly GameLiving _caster;
        private readonly ModernStormKind _kind;
        private readonly int _rank;
        private readonly long _ends;
        private int _pulseNumber;
        private ECSGameTimer _pulse;
        public int Radius => _kind == ModernStormKind.Maelstrom ? 350 : _kind == ModernStormKind.Thornweed ? 500 : 360;
        public int PulseMilliseconds => _kind == ModernStormKind.Thornweed ? 3000 : 5000;
        public int LifetimeSeconds => _kind == ModernStormKind.Maelstrom ? 30 :
            _kind == ModernStormKind.Thornweed ? ModernThornweedField.Duration(_rank) : ModernStaticTempest.Duration(_rank);
        public ModernRealmStorm(GameLiving caster, Point3D point, ModernStormKind kind, int rank)
        {
            _caster = caster; _kind = kind; _rank = rank;
            Name = kind == ModernStormKind.Maelstrom ? "Negative Maelstrom" : kind == ModernStormKind.Thornweed ? "Thornweed Field" : "Static Tempest";
            Model = (ushort)(kind == ModernStormKind.Maelstrom ? 3738 : kind == ModernStormKind.Thornweed ? 2653 : 2654);
            X = point.X; Y = point.Y; Z = point.Z; CurrentRegionID = caster.CurrentRegionID; Realm = caster.Realm; Level = caster.Level;
            _ends = GameLoop.GameLoopTime + LifetimeSeconds * 1000;
        }
        public override bool AddToWorld()
        {
            if (!base.AddToWorld()) return false;
            _pulse = new ECSGameTimer(this, OnPulse, 1);
            return true;
        }
        public bool ShouldExpire(long now) => now >= _ends || _caster.ObjectState != eObjectState.Active ||
            _caster.CurrentRegion != CurrentRegion;
        public override bool RemoveFromWorld() { _pulse?.Stop(); return base.RemoveFromWorld(); }
        private int OnPulse(ECSGameTimer timer)
        {
            if (ShouldExpire(GameLoop.GameLoopTime)) { RemoveFromWorld(); return 0; }
            PulseTargets(GetPlayersInRadius((ushort)Radius).Cast<GameLiving>().Concat(GetNPCsInRadius((ushort)Radius)));
            // End precisely at expiry even when duration is not a multiple of pulse frequency.
            return (int)Math.Min(PulseMilliseconds, Math.Max(1, _ends - GameLoop.GameLoopTime));
        }
        protected virtual bool Allowed(GameLiving target) => GameServer.ServerRules.IsAllowedToAttack(_caster, target, true);
        public void PulseTargets(IEnumerable<GameLiving> candidates)
        {
            _pulseNumber++;
            foreach (GameLiving target in candidates.Distinct().Where(t => t != null && t != _caster && t.IsAlive &&
                t.CurrentRegion == CurrentRegion && IsWithinRadius(t, Radius) && Allowed(t))) Hit(target, _pulseNumber);
        }
        protected virtual void Hit(GameLiving target, int pulse)
        {
            if (_kind == ModernStormKind.Tempest)
            {
                var stun = new Spell(new DbSpell { Name = Name, Type = "UnresistableStun", Target = "Enemy",
                    Duration = 3, ClientEffect = 7032, Icon = 7032 }, 50);
                new ModernTempestStun(_caster, stun).StartSpell(target);
                return;
            }
            double amount = _kind == ModernStormKind.Maelstrom ? ModernNegativeMaelstrom.PulseDamage(_rank, pulse) : ModernThornweedField.Damage(_rank);
            var spell = new Spell(new DbSpell { Name = Name, Type = "DirectDamage", Target = "Enemy", Damage = amount,
                DamageType = (int)(_kind == ModernStormKind.Maelstrom ? eDamageType.Cold : eDamageType.Natural),
                ClientEffect = _kind == ModernStormKind.Maelstrom ? 7027 : 7028, Icon = _kind == ModernStormKind.Maelstrom ? 7027 : 7028 }, 50);
            if (_kind == ModernStormKind.Thornweed) new ModernThornweedDamage(_caster, spell).StartSpell(target);
            else new ModernRealmDamage(_caster, spell).StartSpell(target);
        }
    }
    public class ModernThornweedDamage : ModernRealmDamage
    {
        public ModernThornweedDamage(GameLiving caster, Spell spell) : base(caster, spell) { }
        public override void OnDirectEffect(GameLiving target)
        {
            base.OnDirectEffect(target);
            if (target.IsAlive) new ModernRealmRoot(Caster, new Spell(new DbSpell { Name = "Thornweed Field", Type = "SpeedDecrease",
                Target = "Enemy", Value = 50, Duration = 5, ClientEffect = 7028, Icon = 7028 }, 50)).ApplyEffectOnTarget(target);
        }
    }
    public abstract class ModernStormAbility : ModernGroupRealmAbility
    {
        protected ModernStormAbility(DbAbility ability, int level) : base(ability, level) { }
        protected abstract ModernStormKind Kind { get; }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.CurrentRegion == null) return;
            Point3D point;
            if (Kind == ModernStormKind.Tempest)
            {
                if (living.TargetObject is not GameLiving target || !target.IsAlive || target.CurrentRegion != living.CurrentRegion ||
                    !living.IsWithinRadius(target, 1500) || !GameServer.ServerRules.IsAllowedToAttack(living, target, true) ||
                    living is GamePlayer { TargetInView: false }) return;
                point = new Point3D(target.X, target.Y, target.Z);
            }
            else
            {
                if (!living.GroundTarget.IsValid || !living.IsWithinRadius(living.GroundTarget, 1500)) return;
                point = new Point3D(living.GroundTarget.X, living.GroundTarget.Y, living.GroundTarget.Z);
            }
            int rank = Level;
            void Spawn()
            {
                if (!living.IsWithinRadius(point, 1500)) return;
                var storm = new ModernRealmStorm(living, point, Kind, rank);
                if (storm.AddToWorld()) living.DisableSkill(this, GetReUseDelay(rank) * 1000);
            }
            if (Kind == ModernStormKind.Tempest) { living.Stealth(false); Spawn(); }
            else ModernRealmCast.Begin(living, 2000, (ushort)(Kind == ModernStormKind.Maelstrom ? 7027 : 7028),
                () => living.IsWithinRadius(point, 1500), Spawn);
        }
    }
    public class ModernNegativeMaelstrom : ModernStormAbility
    {
        public ModernNegativeMaelstrom(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernStormKind Kind => ModernStormKind.Maelstrom;
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 900, 780, 675, 560, 450);
        public static double Damage(int rank) => rank <= 0 ? 0 : new[] { 262.5, 390, 525, 650, 775 }[Math.Min(rank, 5) - 1];
        public static double PulseDamage(int rank, int pulse) => Damage(rank) * Math.Clamp(pulse, 0, 6) / 6;
        public override void AddEffectsInfo(IList<string> list) => list.Add("2-second cast. 30-second cold ground storm, 350 radius. Damage ramps over six pulses; peak damage: 262.5/390/525/650/775.");
    }
    public class ModernThornweedField : ModernStormAbility
    {
        public ModernThornweedField(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernStormKind Kind => ModernStormKind.Thornweed;
        public static int Damage(int rank) => ModernRealmRanks.Value(rank, 30, 60, 120, 210, 300);
        public static int Duration(int rank) => ModernRealmRanks.Value(rank, 10, 15, 20, 25, 30);
        public override void AddEffectsInfo(IList<string> list) => list.Add("2-second cast. 500-radius ground field pulses damage and a 50% snare every 3 seconds. Damage: 30/60/120/210/300; duration: 10/15/20/25/30 seconds.");
    }
    public class ModernStaticTempest : ModernStormAbility
    {
        public ModernStaticTempest(DbAbility ability, int level) : base(ability, level) { }
        protected override ModernStormKind Kind => ModernStormKind.Tempest;
        public static int Duration(int rank) => ModernRealmRanks.Value(rank, 15, 20, 25, 30, 35);
        public override void AddEffectsInfo(IList<string> list) => list.Add("Instant targeted stationary storm, 360 radius. A 3-second unresistible stun every 5 seconds for 15/20/25/30/35 seconds.");
    }
    public class ModernIchorDamage : ModernRealmDamage
    {
        private readonly int _rank;
        public ModernIchorDamage(GameLiving caster, Spell spell, int rank) : base(caster, spell) { _rank = rank; }
        public override void OnDirectEffect(GameLiving target)
        {
            base.OnDirectEffect(target);
            if (target.IsAlive) new ModernRealmRoot(Caster, ModernIchor.RootSpell(_rank)).ApplyEffectOnTarget(target);
        }
    }
    public class ModernIchor : ModernGroupRealmAbility
    {
        public ModernIchor(DbAbility ability, int level) : base(ability, level) { }
        public static double Damage(int rank) => rank <= 0 ? 0 : new[] { 185.0, 343.8, 500, 612.5, 725 }[Math.Min(rank, 5) - 1];
        public static int RootSeconds(int rank) => ModernRealmRanks.Value(rank, 5, 7, 9, 12, 15);
        public static Spell RootSpell(int rank) => new(new DbSpell { Name = "Ichor of the Deep", Type = "SpeedDecrease", Target = "Enemy",
            Value = 99, Duration = RootSeconds(rank), DamageType = (int)eDamageType.Spirit, ClientEffect = 7029, Icon = 7029 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.TargetObject is not GameLiving target || !target.IsAlive || target.CurrentRegion != living.CurrentRegion ||
                !living.IsWithinRadius(target, 1875) || !GameServer.ServerRules.IsAllowedToAttack(living, target, true) ||
                living is GamePlayer { TargetInView: false }) return;
            int rank = Level;
            ModernRealmCast.Begin(living, 2000, 7029, () => target.IsAlive && target.CurrentRegion == living.CurrentRegion &&
                living.IsWithinRadius(target, 1875) && GameServer.ServerRules.IsAllowedToAttack(living, target, true), () =>
            {
                var spell = new Spell(new DbSpell { Name = "Ichor of the Deep", Type = "DirectDamage", Target = "Enemy", Range = 1875,
                    Radius = 500, Damage = Damage(rank), DamageType = (int)eDamageType.Spirit, ClientEffect = 7029, Icon = 7029 }, 50);
                new ModernIchorDamage(living, spell, rank).StartSpell(target);
                living.DisableSkill(this, GetReUseDelay(rank) * 1000);
            });
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("2-second cast; spirit damage and fixed 5/7/9/12/15-second root. Root duration ignores resists, Determination and duration gear. Range 1875, radius 500.");
    }
}
