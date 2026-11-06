using System.Collections.Generic;
using System.Linq;
using DOL.AI.Brain;
using DOL.Database;
using DOL.GS.PacketHandler;
using DOL.GS.PropertyCalc;

namespace DOL.GS.RealmAbilities
{
    // Live 1.126: Mastery of Stealth is trainable again with the pre-1.121 speed and detection values.
    public class ModernMasteryOfStealth : AtlasOF_MasteryOfStealth, IModernRealmAbility
    {
        public ModernMasteryOfStealth(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int current) => ModernRealmRanks.PassiveCost(current);
        public override bool CheckRequirement(GamePlayer player) => true;
        // Stealth movement speed bonus, read by the max-speed calculator.
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Value(rank, 10, 15, 20, 25, 30, 35, 40, 45, 50);
        public static int Detection(int rank) => ModernRealmRanks.Value(rank, 75, 125, 175, 235, 300, 375, 450, 535, 625);
        // Camouflage counters it, and it does nothing when both sides have Detect Hidden (assassin against assassin).
        public static int DetectionBonus(GameLiving viewer, GameLiving enemy)
        {
            if (viewer?.GetAbility<ModernMasteryOfStealth>() is not { } mos || enemy?.effectListComponent == null ||
                enemy.effectListComponent.ContainsEffectForEffectType(eEffect.Camouflage) ||
                viewer.HasAbility(Abilities.DetectHidden) && enemy.HasAbility(Abilities.DetectHidden))
                return 0;
            return Detection(mos.Level);
        }
        public override IList<string> DelveInfo
        {
            get
            {
                var list = new List<string> { m_description, "" };
                for (int rank = 1; rank <= MaxLevel; rank++)
                    list.Add($"Rank {rank}: +{GetAmountForLevel(rank)}% stealth speed, +{Detection(rank)} detection range");
                return list;
            }
        }
    }

    // Live 1.123: Vanish left the Stealth specialization and is a trainable five-rank realm ability again,
    // with the disarm and silence raised to 30 seconds. 1.121 added the poison cure when it ends.
    public class ModernVanish : TimedRealmAbility, IModernRealmAbility
    {
        public const int LockoutMs = 30000;
        public ModernVanish(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int current) => ModernRealmRanks.ActiveCost(current);
        public override int GetReUseDelay(int rank) => 900;
        public static int Duration(int rank) => ModernRealmRanks.Value(rank, 3, 3, 4, 5, 6);
        public static double Speed(int rank) => rank switch
        {
            2 => MaxSpeedCalculator.SPEED1, 3 => MaxSpeedCalculator.SPEED3, 4 => MaxSpeedCalculator.SPEED4,
            >= 5 => MaxSpeedCalculator.SPEED5, _ => 1.0
        };
        public override void Execute(GameLiving living)
        {
            if (Level < 1 || Level > MaxLevel || CheckPreconditions(living, DEAD | SITTING | STEALTHED)) return;
            SendCasterSpellEffectAndCastMessage(living, 7020, true);
            int rank = Level;
            ECSGameEffectFactory.Create(new(living, Duration(rank) * 1000, Speed(rank)), static (in i) => new ModernVanishEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Instantly hides you in unbreakable stealth, purging damage over time, bleeds and crowd control, with crowd-control immunity while it lasts.");
            list.Add("Rank 1-5: 3/3/4/5/6 seconds at normal speed, speed 1, speed 3, speed 4 and speed 5.");
            list.Add("Cures poisons when it ends. You cannot attack or cast for 30 seconds.");
        }
    }

    public class ModernVanishEffect : ECSGameAbilityEffect
    {
        private bool _speedApplied;
        public ModernVanishEffect(in ECSGameEffectInitParams init) : base(init) { EffectType = eEffect.Vanish; }
        public override string Name => "Vanish";
        public override ushort Icon => 3019;
        public override bool HasPositiveEffect => true;
        public static readonly eEffect[] Purged = { eEffect.DamageOverTime, eEffect.Bleed, eEffect.Mez, eEffect.Stun, eEffect.MovementSpeedDebuff };
        public override void OnStartEffect()
        {
            Owner.attackComponent.StopAttack();
            Owner.StopCurrentSpellcast();
            foreach (ECSGameEffect effect in Purged.SelectMany(type => Owner.effectListComponent.GetEffects(type)).ToArray())
                if (!effect.IsEnding) effect.End();
            Owner.Stealth(true);
            long now = Owner.CurrentRegion?.Time ?? GameLoop.GameLoopTime;
            Owner.DisarmedTime = now + ModernVanish.LockoutMs;
            Owner.SilencedTime = now + ModernVanish.LockoutMs;
            if (Effectiveness > 1)
            {
                Owner.BuffBonusMultCategory1.Set((int)eProperty.MaxSpeed, this, Effectiveness);
                _speedApplied = true;
                OwnerPlayer?.Out.SendUpdateMaxSpeed();
            }
            DropAttackers(Owner);
        }
        public override void OnStopEffect()
        {
            if (_speedApplied)
            {
                Owner.BuffBonusMultCategory1.Remove((int)eProperty.MaxSpeed, this);
                _speedApplied = false;
                OwnerPlayer?.Out.SendUpdateMaxSpeed();
            }
            CurePoisons(Owner);
        }
        public static void CurePoisons(GameLiving owner)
        {
            foreach (ECSGameSpellEffect effect in owner.effectListComponent.GetEffects().OfType<ECSGameSpellEffect>()
                .Where(e => !e.IsEnding && e.SpellHandler?.SpellLine?.KeyName == GlobalSpellsLines.Mundane_Poisons).ToArray())
                effect.End();
        }
        public static void DropAttackers(GameLiving living)
        {
            foreach (GameObject attacker in living.attackComponent.AttackerTracker.Attackers)
            {
                if (attacker is not GameLiving enemy || enemy.TargetObject != living) continue;
                enemy.TargetObject = null;
                if (enemy is GamePlayer player)
                    player.Out.SendChangeTarget(null);
                else if (enemy is GameNPC npc)
                {
                    if (npc.Brain is IOldAggressiveBrain brain) brain.RemoveFromAggroList(living);
                    npc.attackComponent.StopAttack();
                }
            }
        }
    }
}
