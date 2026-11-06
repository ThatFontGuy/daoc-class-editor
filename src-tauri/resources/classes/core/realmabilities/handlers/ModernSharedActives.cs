using System.Collections.Generic;
using System.Linq;
using DOL.AI;
using DOL.Database;
using DOL.GS.Effects;
using DOL.GS.PacketHandler;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    public static class ModernRealmControl
    {
        // Charge and Speed of Sound make the target immune to stuns and mezzes; damage still lands.
        public static bool IsMovingTooFast(GameLiving target) => target.effectListComponent != null &&
            (target.effectListComponent.ContainsEffectForEffectType(eEffect.Charge) ||
            target.effectListComponent.ContainsEffectForEffectType(eEffect.SpeedOfSound));
    }

    public class ModernChargeEffect : ChargeECSGameEffect
    {
        private bool _applied;
        public ModernChargeEffect(in ECSGameEffectInitParams init) : base(init) { }
        public override void OnStartEffect()
        {
            base.OnStartEffect();
            if (_applied) return;
            _applied = true;
            Owner.BuffBonusMultCategory1.Set((int)eProperty.MaxSpeed, this, PropertyCalc.MaxSpeedCalculator.SPEED3);
            OwnerPlayer?.Out.SendUpdateMaxSpeed();
        }
        public override void OnStopEffect()
        {
            base.OnStopEffect();
            if (!_applied) return;
            _applied = false;
            Owner.BuffBonusMultCategory1.Remove((int)eProperty.MaxSpeed, this);
            OwnerPlayer?.Out.SendUpdateMaxSpeed();
        }
    }

    public class ModernSpeedOfSound : ModernGroupRealmAbility
    {
        public ModernSpeedOfSound(DbAbility ability, int level) : base(ability, level) { }
        public static int Duration(int rank) => ModernRealmRanks.Value(rank, 10, 20, 30, 45, 60);
        public override ushort Icon => 3020;
        public static IEnumerable<GameLiving> Targets(GameLiving living)
        {
            foreach (GameLiving member in Members(living, 2000))
            {
                yield return member;
                if (member.ControlledBrain?.Body is GameNPC { IsAlive: true } pet && pet.CurrentRegion == living.CurrentRegion &&
                    living.IsWithinRadius(pet, 2000))
                    yield return pet;
            }
        }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.effectListComponent.ContainsEffectForEffectType(eEffect.SpeedOfSound)) return;
            int duration = Duration(Level) * 1000;
            foreach (GameLiving target in Targets(living).Distinct().ToArray())
            {
                if (target.effectListComponent.ContainsEffectForEffectType(eEffect.SpeedOfSound)) continue;
                foreach (GamePlayer viewer in target.GetPlayersInRadius(WorldMgr.VISIBILITY_DISTANCE))
                    viewer.Out.SendSpellEffectAnimation(living, target, 7021, 0, false, 1);
                ECSGameEffectFactory.Create(new(target, duration, 1), static (in i) => new ModernSpeedOfSoundEffect(i));
            }
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Group and pets within 2000 run at speed 4 for 10/20/30/45/60 seconds.");
            list.Add("Immune to stun, mez, snare and root while it lasts; damage still applies.");
            list.Add("Each member's effect ends when that member attacks or casts a harmful spell. Healing does not break it.");
        }
    }

    // The Atlas effect only moves players; this one also speeds up companions, gamebots and pets.
    public class ModernSpeedOfSoundEffect : SpeedOfSoundECSEffect
    {
        private bool _applied;
        public ModernSpeedOfSoundEffect(in ECSGameEffectInitParams init) : base(init) { }
        public static readonly HashSet<string> PassiveLines = new()
        {
            GlobalSpellsLines.Item_Effects, GlobalSpellsLines.Potions_Effects, GlobalSpellsLines.Mundane_Poisons,
            GlobalSpellsLines.Combat_Styles_Effect, GlobalSpellsLines.Realm_Spells
        };
        public static bool BreaksOn(Spell spell, SpellLine line) => spell != null && spell.IsHarmful && !spell.IsHealing &&
            !PassiveLines.Contains(line?.KeyName ?? string.Empty);
        public static void BreakOnAction(GameLiving living)
        {
            if (living?.effectListComponent == null) return;
            foreach (var effect in living.effectListComponent.GetAbilityEffects(eEffect.SpeedOfSound).OfType<ModernSpeedOfSoundEffect>().ToArray())
                if (!effect.IsEnding) effect.End();
        }
        public override void OnStartEffect()
        {
            if (_applied) return;
            _applied = true;
            if (OwnerPlayer != null) { base.OnStartEffect(); return; }
            foreach (ECSGameSpellEffect slow in Owner.effectListComponent.GetSpellEffects(eEffect.MovementSpeedDebuff))
                slow.Disable();
            Owner.BuffBonusMultCategory1.Set((int)eProperty.MaxSpeed, this, PropertyCalc.MaxSpeedCalculator.SPEED4);
        }
        public override void OnStopEffect()
        {
            if (!_applied) return;
            _applied = false;
            if (OwnerPlayer != null) { base.OnStopEffect(); return; }
            Owner.BuffBonusMultCategory1.Remove((int)eProperty.MaxSpeed, this);
            foreach (ECSGameSpellEffect slow in Owner.effectListComponent.GetSpellEffects(eEffect.MovementSpeedDebuff))
                slow.Enable();
        }
    }

    public class ModernPerfectRecovery : AtlasOF_PerfectRecovery, ISpellCastingAbilityHandler, IModernRealmAbility
    {
        private Spell _spell;
        public ModernPerfectRecovery(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int current) => ModernRealmRanks.ActiveCost(current);
        public override int GetReUseDelay(int rank) => 300;
        public static int Restore(int rank) => ModernRealmRanks.Value(rank, 10, 25, 50, 75, 100);
        public new Spell Spell => _spell;
        public new SpellLine SpellLine => GlobalSpellsLines.RealmSpellsSpellLine;
        public new Ability Ability => this;
        public Spell BuildSpell(int rank, int casterLevel) => new(new DbSpell
        {
            Name = "Perfect Recovery Rez", ClientEffect = 7019, Target = eSpellTarget.CORPSE.ToString(), Type = eSpellType.Resurrect.ToString(),
            Range = 1500, RecastDelay = GetReUseDelay(rank), ResurrectHealth = Restore(rank), ResurrectMana = Restore(rank)
        }, casterLevel);
        public override void Execute(GameLiving living)
        {
            if (Level < 1 || Level > MaxLevel || CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED)) return;
            if (living is not GamePlayer caster || caster.castingComponent == null || caster.TargetObject is not GameLiving) return;
            // The realm spell line keeps the resurrection free of rez sickness; the spell code disables the ability.
            _spell = BuildSpell(Level, caster.Level);
            caster.CastSpell(_spell, SpellLine, this);
            _spell = null;
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Instantly resurrects a dead realm mate at 10/25/50/75/100% health and power, with no resurrection illness. Range 1500.");
    }

    public class ModernDecimationTrap : ModernGroupRealmAbility
    {
        public const int Lifetime = 600000;
        public ModernDecimationTrap(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => 900;
        public static int Damage(int rank) => ModernRealmRanks.Value(rank, 300, 450, 600, 750, 900);
        public Spell BuildSpell(int rank) => new(new DbSpell { Name = "Decimation Trap", Type = eSpellType.DirectDamage.ToString(),
            Target = eSpellTarget.ENEMY.ToString(), Range = 350, Radius = 350, Damage = Damage(rank), DamageType = (int)eDamageType.Energy,
            Icon = 4254, ClientEffect = 7153 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.IsCasting) return;
            int rank = Level;
            ModernRealmCast.Begin(living, 2000, 7153, () => true, () =>
            {
                var trap = new ModernDecimationTrapMine(BuildSpell(rank), GameLoop.GameLoopTime + Lifetime)
                {
                    Name = "Decimation Trap", GuildName = living.Name, Realm = living.Realm, Size = 50, Level = living.Level,
                    CurrentRegion = living.CurrentRegion, X = living.X, Y = living.Y, Z = living.Z, Owner = living
                };
                if (!trap.AddToWorld()) return;
                (living as GamePlayer)?.Out.SendMessage("You set a Decimation Trap!", eChatType.CT_Spell, eChatLoc.CL_SystemWindow);
                living.DisableSkill(this, GetReUseDelay(rank) * 1000);
            });
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("2-second uninterruptible cast; moving or crowd control cancels it.");
            list.Add("Places a trap that detonates on the first enemy within 350, dealing 300/450/600/750/900 energy damage in a 350 radius.");
            list.Add("The trap lasts 10 minutes or until it detonates.");
        }
    }

    public class ModernDecimationTrapMine : GameMine
    {
        public override bool IsVisibleToPlayers => true;
        public Spell Spell { get; }
        public ModernDecimationTrapMine(Spell spell, long expires) : base(new ModernDecimationTrapBrain(expires)) { Spell = spell; }
    }

    public class ModernDecimationTrapBrain : ABrain
    {
        private readonly long _expires;
        public ModernDecimationTrapBrain(long expires) { _expires = expires; }
        public override int ThinkInterval => 500;
        public static GameLiving FindTrigger(GameLiving caster, GameObject trap, int radius)
        {
            foreach (GamePlayer player in trap.GetPlayersInRadius((ushort)radius))
                if (player.IsAlive && GameServer.ServerRules.IsAllowedToAttack(caster, player, true)) return player;
            foreach (GameNPC npc in trap.GetNPCsInRadius((ushort)radius))
                if (npc is not GameMine && npc.IsAlive && GameServer.ServerRules.IsAllowedToAttack(caster, npc, true)) return npc;
            return null;
        }
        public override void Think()
        {
            if (Body is not ModernDecimationTrapMine trap || GameServiceUtils.ShouldTick(_expires) || trap.Owner == null ||
                trap.Owner.ObjectState == GameObject.eObjectState.Deleted || trap.Owner.CurrentRegion != trap.CurrentRegion)
            {
                Body?.Die(Body);
                return;
            }
            GameLiving target = FindTrigger(trap.Owner, trap, (int)trap.Spell.Radius);
            if (target == null) return;
            // Damage is centered on the enemy that sprang the trap, through the normal resist and combat pipeline.
            new ModernRealmDamage(trap.Owner, trap.Spell).StartSpell(target);
            trap.Die(trap);
        }
        public override void KillFSM() { }
    }

    public class ModernAdrenalineRush : ModernGroupRealmAbility
    {
        public ModernAdrenalineRush(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 1200, 900, 600, 450, 300);
        public static double DamageFactor(GameLiving living) => living?.effectListComponent != null &&
            living.effectListComponent.ContainsEffectForEffectType(eEffect.ModernAdrenalineRush) ? 2 : 1;
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.effectListComponent.ContainsEffectForEffectType(eEffect.ModernAdrenalineRush)) return;
            ECSGameEffectFactory.Create(new(living, 20000, 1), static (in i) => new ModernAdrenalineRushEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Doubles your base melee damage for 20 seconds. Reuse: 20/15/10/7.5/5 minutes.");
    }

    public class ModernAdrenalineRushEffect : ECSGameAbilityEffect
    {
        public ModernAdrenalineRushEffect(in ECSGameEffectInitParams init) : base(init) { EffectType = eEffect.ModernAdrenalineRush; }
        public override string Name => "Adrenaline Rush";
        public override ushort Icon => 3001;
        public override bool HasPositiveEffect => true;
    }

    public class ModernDashingDefense : ModernGroupRealmAbility
    {
        public const int Range = 1000;
        public ModernDashingDefense(DbAbility ability, int level) : base(ability, level) { }
        public static int Duration(int rank) => ModernRealmRanks.Value(rank, 10, 20, 30, 45, 60);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            bool used = false;
            foreach (GameLiving target in Members(living, Range, false))
            {
                if (target.effectListComponent.ContainsEffectForEffectType(eEffect.ModernDashingDefense)) continue;
                var source = living;
                ECSGameEffectFactory.Create(new(target, Duration(Level) * 1000, 1),
                    (in ECSGameEffectInitParams init) => new ModernDashingDefenseEffect(init, source));
                used = true;
            }
            if (used) DisableSkill(living);
        }
        public static bool CanDefend(GameLiving source, GameLiving target) => source != null && source != target && source.IsAlive &&
            source.ObjectState == GameObject.eObjectState.Active && !source.IsIncapacitated && !source.IsSitting && !source.IsCasting &&
            source.CurrentRegion == target.CurrentRegion && source.IsWithinRadius(target, Range);
        // Live: each attack gets one Dashing Defense attempt, however many tanks have it running.
        public static bool TryDefend(AttackData ad, bool stealthStyle, out eAttackResult result)
        {
            result = eAttackResult.Any;
            GameLiving target = ad.Target;
            if (stealthStyle || ad.AttackType is AttackData.eAttackType.Spell || target?.effectListComponent == null) return false;
            GameLiving source = target.effectListComponent.GetAbilityEffects(eEffect.ModernDashingDefense).OfType<ModernDashingDefenseEffect>()
                .Where(e => e.IsActive && !e.IsEnding).Select(e => e.Source).FirstOrDefault(s => CanDefend(s, target));
            if (source == null) return false;
            if (ad.IsMeleeAttack)
            {
                double parry = source.TryParry(ad, null, source.attackComponent.AttackerTracker.MeleeCount);
                if (parry > 0 && parry > source.GetPseudoDouble(RandomDeckEvent.Parry))
                {
                    ad.ParryChance = parry * 100;
                    result = Redirect(ad, source, eAttackResult.Parried);
                    (target as GamePlayer)?.Out.SendMessage($"{source.GetName(0, true)} parries an attack for you!", eChatType.CT_Missed, eChatLoc.CL_SystemWindow);
                    (source as GamePlayer)?.Out.SendMessage($"You parry an attack for {target.GetName(0, false)}!", eChatType.CT_Missed, eChatLoc.CL_SystemWindow);
                    return true;
                }
            }
            double block = source.TryBlock(ad, out _);
            if (block > 0 && block > source.GetPseudoDouble(RandomDeckEvent.Block))
            {
                ad.BlockChance = block * 100;
                result = Redirect(ad, source, eAttackResult.Blocked);
                return true;
            }
            return false;
        }
        private static eAttackResult Redirect(AttackData ad, GameLiving source, eAttackResult result)
        {
            // Same shape as Guard, so the existing "blocks the attack for you" messages apply.
            ad.OriginalTarget = ad.Target;
            ad.Target = source;
            return result;
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("You block and parry for groupmates within 1000 for 10/20/30/45/60 seconds, using your own block and parry chances.");
            list.Add("Each attack can be blocked or parried this way only once, however many group members have Dashing Defense running.");
        }
    }

    public class ModernDashingDefenseEffect : ECSGameAbilityEffect
    {
        public GameLiving Source { get; }
        public ModernDashingDefenseEffect(in ECSGameEffectInitParams init, GameLiving source) : base(init)
        {
            Source = source; EffectType = eEffect.ModernDashingDefense;
        }
        public override string Name => "Dashing Defense";
        public override ushort Icon => 3032;
        public override bool HasPositiveEffect => true;
    }

    public class ModernDivineIntervention : ModernGroupRealmAbility
    {
        public const int Range = 2000, BuffDuration = 1200000;
        public ModernDivineIntervention(DbAbility ability, int level) : base(ability, level) { }
        public static int Pool(int rank) => ModernRealmRanks.Value(rank, 1000, 1500, 2000, 2500, 3000);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            GameLiving[] targets = Members(living, Range, false)
                .Where(t => !t.effectListComponent.ContainsEffectForEffectType(eEffect.ModernDivineIntervention)).ToArray();
            if (targets.Length == 0) return;
            var pool = new ModernHealingPool(living, Pool(Level));
            foreach (GameLiving target in targets)
                ECSGameEffectFactory.Create(new(target, BuffDuration, 1), (in ECSGameEffectInitParams init) => new ModernDivineInterventionEffect(init, pool));
            DisableSkill(living);
        }
        // Called after combat damage lands. Heals from the shared pool, never more than the hit or the missing health.
        public static int Absorb(GameLiving owner, int damage)
        {
            if (damage <= 0 || !owner.IsAlive || owner.effectListComponent == null) return 0;
            var effect = owner.effectListComponent.GetAbilityEffects(eEffect.ModernDivineIntervention).OfType<ModernDivineInterventionEffect>()
                .FirstOrDefault(e => e.IsActive && !e.IsEnding);
            if (effect == null) return 0;
            int wanted = System.Math.Min(damage, owner.MaxHealth - owner.Health);
            int granted = effect.Pool.Take(wanted);
            int healed = granted > 0 ? owner.ChangeHealth(effect.Pool.Caster, eHealthChangeType.Spell, granted) : 0;
            if (healed < granted) effect.Pool.Refund(granted - healed);
            if (healed > 0 && owner is GamePlayer player)
                player.Out.SendMessage($"Divine Intervention heals you for {healed} hit points!", eChatType.CT_Spell, eChatLoc.CL_SystemWindow);
            if (effect.Pool.Remaining <= 0) effect.Pool.EndAll();
            return healed;
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Gives groupmates within 2000 (not you) a 20-minute buff sharing one healing pool of 1000/1500/2000/2500/3000.");
            list.Add("Whenever a protected member takes combat damage, they are healed from the pool at once.");
        }
    }

    public class ModernHealingPool
    {
        private readonly object _lock = new();
        private readonly List<ModernDivineInterventionEffect> _effects = new();
        public GameLiving Caster { get; }
        public int Remaining { get; private set; }
        public ModernHealingPool(GameLiving caster, int amount) { Caster = caster; Remaining = amount; }
        public int Take(int amount)
        {
            lock (_lock) { int taken = System.Math.Clamp(amount, 0, Remaining); Remaining -= taken; return taken; }
        }
        public void Refund(int amount) { lock (_lock) Remaining += amount; }
        public void Register(ModernDivineInterventionEffect effect) { lock (_lock) _effects.Add(effect); }
        public void EndAll()
        {
            ModernDivineInterventionEffect[] effects;
            lock (_lock) effects = _effects.ToArray();
            foreach (var effect in effects) if (!effect.IsEnding) effect.End();
        }
    }

    public class ModernDivineInterventionEffect : ECSGameAbilityEffect
    {
        public ModernHealingPool Pool { get; }
        public ModernDivineInterventionEffect(in ECSGameEffectInitParams init, ModernHealingPool pool) : base(init)
        {
            Pool = pool; EffectType = eEffect.ModernDivineIntervention; pool.Register(this);
        }
        public override string Name => "Divine Intervention";
        public override ushort Icon => 3035;
        public override bool HasPositiveEffect => true;
    }
}
