using System;
using System.Collections.Generic;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    // Keep spell damage in the normal attack pipeline: resistances, ablatives,
    // damage immunities and combat notifications must apply to an RA too.
    public class ModernRealmDamage : DirectDamageSpellHandler
    {
        public ModernRealmDamage(GameLiving caster, Spell spell) : base(caster, spell, GlobalSpellsLines.RealmSpellsSpellLine) { }
        public override double CalculateDamageBase(GameLiving target) => Spell.Damage;
        public override void CalculateDamageVariance(GameLiving target, out double min, out double max) { min = 1; max = 1; }
        protected override double CalculateDamageEffectiveness() => 1;
        public override double DamageCap(double effectiveness) => Spell.Damage;
        public override AttackData CalculateDamageToTarget(GameLiving target)
        {
            AttackData damage = base.CalculateDamageToTarget(target);
            damage.CriticalChance = 0; damage.CriticalDamage = 0;
            return damage;
        }
    }
    public class ModernWrathOfTheChampion : ModernGroupRealmAbility
    {
        public ModernWrathOfTheChampion(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 600, 525, 450, 375, 300);
        // The final 1.128 notes explicitly list 522.5 for rank two.
        public static double Damage(int rank) => rank <= 0 ? 0 : new[] { 300.0, 522.5, 750.0, 937.5, 1125.0 }[Math.Min(rank, 5) - 1];
        public Spell BuildSpell(int rank) => new(new DbSpell { Name = "Wrath of Champions", Type = "DirectDamage", Target = "Enemy",
            Range = 0, Radius = 150, Damage = Damage(rank), DamageType = (int)eDamageType.Spirit, Icon = 4468, ClientEffect = 4468 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            living.Stealth(false);
            new ModernRealmDamage(living, BuildSpell(Level)).StartSpell(living);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Instant spirit PBAoE, 150 radius. Damage: 300/522.5/750/937.5/1125 before resists.");
    }
    public class ModernVolcanicPillar : ModernGroupRealmAbility
    {
        public ModernVolcanicPillar(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 900, 780, 675, 560, 450);
        public static int Damage(int rank) => ModernRealmRanks.Value(rank, 400, 700, 1000, 1250, 1500);
        public Spell BuildSpell(int rank) => new(new DbSpell { Name = "Volcanic Pillar", Type = "DirectDamage", Target = "Enemy",
            Range = 1500, Radius = 500, Damage = Damage(rank), DamageType = (int)eDamageType.Heat, Icon = 7025, ClientEffect = 7025 }, 50);
        public static bool CanFinish(GameLiving caster, GameLiving target, Region region, Point3D origin) =>
            caster.IsAlive && !caster.IsMoving && !caster.IsSitting && !caster.IsCrowdControlled && caster.CurrentRegion == region &&
            caster.X == origin.X && caster.Y == origin.Y && caster.Z == origin.Z && target != null && target.IsAlive &&
            target.CurrentRegion == region && caster.IsWithinRadius(target, 1500);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.IsMoving || living.CurrentRegion == null ||
                living.TargetObject is not GameLiving target || !target.IsAlive || target.CurrentRegion != living.CurrentRegion ||
                !living.IsWithinRadius(target, 1500) || !GameServer.ServerRules.IsAllowedToAttack(living, target, true) ||
                living is GamePlayer { TargetInView: false }) return;
            GamePlayer player = living as GamePlayer;
            if (player?.RealmAbilityCastTimer?.IsAlive == true) return;
            Region region = living.CurrentRegion;
            Point3D origin = new(living.X, living.Y, living.Z);
            int rank = Level;
            living.StopCurrentSpellcast();
            living.attackComponent.StopAttack();
            living.Stealth(false);
            foreach (GamePlayer viewer in living.GetPlayersInRadius(WorldMgr.VISIBILITY_DISTANCE))
                viewer.Out.SendSpellCastAnimation(living, 7025, 20);
            ECSGameTimer cast = new(living);
            long completes = GameLoop.GameLoopTime + 2000;
            cast.Callback = timer =>
            {
                if (player != null && player.RealmAbilityCastTimer != timer) return 0;
                if (!CanFinish(living, target, region, origin))
                {
                    if (player != null) player.RealmAbilityCastTimer = null;
                    return 0;
                }
                if (GameLoop.GameLoopTime < completes) return 100;
                if (player != null) player.RealmAbilityCastTimer = null;
                if (GameServer.ServerRules.IsAllowedToAttack(living, target, true))
                {
                    // Capture the original target and rank: switching targets during the cast cannot redirect the blast.
                    new ModernRealmDamage(living, BuildSpell(rank)).StartSpell(target);
                    living.DisableSkill(this, GetReUseDelay(rank) * 1000);
                }
                return 0;
            };
            if (player != null) player.RealmAbilityCastTimer = cast;
            cast.Start(100);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("2-second uninterruptible heat AoE; movement or crowd control cancels. Range 1500, radius 500. Damage: 400/700/1000/1250/1500 before resists.");
    }
}
