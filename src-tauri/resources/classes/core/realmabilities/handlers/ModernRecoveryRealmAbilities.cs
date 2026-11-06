using System.Collections.Generic;
using DOL.Database;

namespace DOL.GS.RealmAbilities
{
    public abstract class ModernRecoveryAbility : TimedRealmAbility, IModernRealmAbility
    {
        protected ModernRecoveryAbility(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.ActiveCost(currentLevel);
        public abstract int RestorePercent(int rank);
        protected abstract int Restore(GameLiving living, int percent);
        protected virtual bool OutOfCombatOnly => false;
        protected virtual ushort Effect => 7009;
        public override void Execute(GameLiving living)
        {
            if (Level < 1 || Level > MaxLevel || CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED | (OutOfCombatOnly ? INCOMBAT : 0)))
                return;
            int restored = Restore(living, RestorePercent(Level));
            SendCasterSpellEffectAndCastMessage(living, Effect, restored > 0);
            if (restored > 0)
                DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add(OutOfCombatOnly ? "Instant self recovery. Cannot be used in combat." : "Instant self recovery. Usable in combat.");
            for (int rank = 1; rank <= MaxLevel; rank++)
                list.Add($"Rank {rank}: {RestorePercent(rank)}%");
        }
        protected static int Portion(int maximum, int percent) => (int)((long)maximum * percent / 100);
    }

    public class ModernIgnorePain : ModernRecoveryAbility
    {
        public ModernIgnorePain(DbAbility ability, int level) : base(ability, level) { }
        public override int RestorePercent(int rank) => ModernRealmRanks.Value(rank, 20, 35, 50, 65, 80);
        protected override int Restore(GameLiving living, int percent) => living.ChangeHealth(living, eHealthChangeType.Spell, Portion(living.MaxHealth, percent));
        protected override ushort Effect => 7004;
        public override int GetReUseDelay(int rank) => 900;
    }
    public class ModernIgnorePainTank : ModernIgnorePain
    {
        public ModernIgnorePainTank(DbAbility ability, int level) : base(ability, level) { }
    }
    public class ModernMCL : ModernRecoveryAbility
    {
        public ModernMCL(DbAbility ability, int level) : base(ability, level) { }
        public override int RestorePercent(int rank) => ModernRealmRanks.Value(rank, 25, 40, 60, 80, 100);
        protected override bool OutOfCombatOnly => true;
        protected override int Restore(GameLiving living, int percent) => living.ChangeMana(living, eManaChangeType.Spell, Portion(living.MaxMana, percent));
        public override int GetReUseDelay(int rank) => 180;
    }
    public class ModernRagingPower : ModernRecoveryAbility
    {
        public ModernRagingPower(DbAbility ability, int level) : base(ability, level) { }
        public override int RestorePercent(int rank) => ModernRealmRanks.Value(rank, 25, 35, 50, 65, 80);
        protected override int Restore(GameLiving living, int percent) => living.ChangeMana(living, eManaChangeType.Spell, Portion(living.MaxMana, percent));
        public override int GetReUseDelay(int rank) => 600;
    }
    public class ModernSecondWind : ModernRecoveryAbility
    {
        public ModernSecondWind(DbAbility ability, int level) : base(ability, level) { }
        public override int RestorePercent(int rank) => 100;
        protected override ushort Effect => 7003;
        protected override int Restore(GameLiving living, int percent) => living.ChangeEndurance(living, eEnduranceChangeType.Spell, living.MaxEndurance);
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 900, 600, 300, 210, 120);
    }

    public class ModernPurge : PurgeAbility, IModernRealmAbility
    {
        public ModernPurge(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.ActiveCost(currentLevel);
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 900, 900, 900, 600, 300);
        public static int DelaySeconds(int rank) => rank == 1 ? 5 : rank == 2 ? 2 : 0;
        public override void Execute(GameLiving living)
        {
            if (Level < 1 || Level > MaxLevel || CheckPreconditions(living, DEAD | SITTING))
                return;
            int delay = DelaySeconds(Level);
            if (delay > 0)
            {
                DisableSkill(living);
                new ECSGameTimer(living, _ =>
                {
                    if (living.IsAlive)
                    {
                        SendCastMessage(living);
                        // The legacy helper shortens cooldowns when there is nothing to remove.
                        RemoveNegativeEffects(living, this, isFromGroupPurge: true);
                    }
                    return 0;
                }, delay * 1000);
            }
            else
            {
                SendCastMessage(living);
                if (RemoveNegativeEffects(living, this, isFromGroupPurge: true))
                    DisableSkill(living);
            }
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Removes negative effects while preserving immunity timers.");
            list.Add("Ranks 1/2 activate after 5/2 seconds; ranks 3–5 activate immediately.");
        }
    }
    public class ModernPurgeReduced : ModernPurge
    {
        public ModernPurgeReduced(DbAbility ability, int level) : base(ability, level) { }
    }

    public class ModernConcentration : ConcentrationAbility, IModernRealmAbility
    {
        public ModernConcentration(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.ActiveCost(currentLevel);
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 900, 540, 180, 90, 30);
    }

    public class ModernCharge : TimedRealmAbility, IModernRealmAbility
    {
        public ModernCharge(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        // Live 1.128c: 3/3/3/5/6 instead of the standard 5/5/5/7/8 active costs.
        public override int CostForUpgrade(int currentLevel) => currentLevel < 0 || currentLevel >= 5 ? 1000 : new[] { 3, 3, 3, 5, 6 }[currentLevel];
        public override int GetReUseDelay(int rank) => ModernRealmRanks.Value(rank, 900, 600, 300, 180, 90);
        public override bool CheckRequirement(GamePlayer player) => player.Level >= 40;
        public override void Execute(GameLiving living)
        {
            if (Level < 1 || Level > MaxLevel || CheckPreconditions(living, DEAD | SITTING | MEZZED | STUNNED) ||
                living.effectListComponent.ContainsEffectForEffectType(eEffect.MovementSpeedDebuff) ||
                living.effectListComponent.ContainsEffectForEffectType(eEffect.Charge))
                return;
            ECSGameEffectFactory.Create(new(living, 15000, 1, null), static (in i) => new ModernChargeEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("15 seconds of movement speed and crowd-control immunity. Target: Self.");
            list.Add("Does not remove existing roots or snares.");
        }
    }
}
