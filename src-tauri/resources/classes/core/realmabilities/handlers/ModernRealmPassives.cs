using DOL.Database;

namespace DOL.GS.RealmAbilities
{
    public interface IModernRealmAbility { }

    // Explicit live tables: enabling a global NF flag also changes unaudited legacy handlers.
    public static class ModernRealmRanks
    {
        public static int Value(int rank, params int[] amounts) => rank <= 0 ? 0 : amounts[System.Math.Min(rank, amounts.Length) - 1];
        public static int PassiveCost(int current) => current < 0 || current >= 9 ? 1000 : new[] { 1, 1, 2, 3, 3, 5, 5, 7, 7 }[current];
        public static int ActiveCost(int current) => current < 0 || current >= 5 ? 1000 : new[] { 5, 5, 5, 7, 8 }[current];
        public static int Stats(int rank) => Value(rank, 4, 8, 12, 17, 22, 28, 34, 41, 48);
        public static int Crit(int rank) => Value(rank, 3, 6, 9, 13, 17, 22, 27, 33, 39);
        public static int Defense(int rank) => Value(rank, 2, 4, 6, 9, 12, 15, 18, 21, 25);
        public static int Healing(int rank) => Value(rank, 2, 4, 6, 9, 12, 16, 20, 25, 30);
        public static int Magery(int rank) => Value(rank, 2, 3, 4, 6, 8, 10, 12, 14, 16);
        public static int Magic(int rank) => Value(rank, 2, 3, 5, 7, 10, 12, 15, 17, 20);
        public static int Determination(int rank) => Value(rank, 4, 8, 12, 17, 23, 30, 38, 46, 55);
        public static int Veil(int rank) => Value(rank, 10, 15, 20, 30, 40, 50, 60, 70, 80);
        public static int Health(int rank) => Value(rank, 25, 50, 75, 100, 150, 200, 250, 325, 400);
        public static int Power(int rank) => Value(rank, 15, 25, 40, 55, 75, 100, 130, 165, 200);
    }

    public class ModernAugStr : AtlasOF_RAStrengthEnhancer, IModernRealmAbility
    {
        public ModernAugStr(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Stats(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernAugCon : AtlasOF_RAConstitutionEnhancer, IModernRealmAbility
    {
        public ModernAugCon(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Stats(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernAugDex : AtlasOF_RADexterityEnhancer, IModernRealmAbility
    {
        public ModernAugDex(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Stats(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernAugQui : AtlasOF_RAQuicknessEnhancer, IModernRealmAbility
    {
        public ModernAugQui(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Stats(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernAugAcuity : AtlasOF_RAAcuityEnhancer, IModernRealmAbility
    {
        public ModernAugAcuity(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Stats(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernAoM : AtlasOF_AvoidanceOfMagicAbility, IModernRealmAbility
    {
        public ModernAoM(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Magic(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernMasteryOfPain : AtlasOF_MasteryOfPain, IModernRealmAbility
    {
        public ModernMasteryOfPain(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Crit(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernMasteryOfBlocking : AtlasOF_MasteryOfBlocking, IModernRealmAbility
    {
        public ModernMasteryOfBlocking(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Defense(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernMasteryOfParrying : AtlasOF_MasteryOfParrying, IModernRealmAbility
    {
        public ModernMasteryOfParrying(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Defense(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernMasteryOfHealing : AtlasOF_MasteryOfHealing, IModernRealmAbility
    {
        public ModernMasteryOfHealing(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Healing(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernMasteryOfMagery : AtlasOF_MasteryOfMagery, IModernRealmAbility
    {
        public ModernMasteryOfMagery(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Magery(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernWildPower : AtlasOF_WildPowerAbility, IModernRealmAbility
    {
        public ModernWildPower(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Crit(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernWildHealing : AtlasOF_WildHealingAbility, IModernRealmAbility
    {
        public ModernWildHealing(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Crit(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernWildMinion : AtlasOF_WildMinionAbility, IModernRealmAbility
    {
        public ModernWildMinion(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Crit(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernFalconsEye : AtlasOF_FalconsEye, IModernRealmAbility
    {
        public ModernFalconsEye(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Crit(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernVeilRecovery : AtlasOF_VeilRecovery, IModernRealmAbility
    {
        public ModernVeilRecovery(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Veil(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernDetermination : AtlasOF_DeterminationAbility, IModernRealmAbility
    {
        public ModernDetermination(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Determination(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernDeterminationHybrid : AtlasOF_DeterminationHybridAbility, IModernRealmAbility
    {
        public ModernDeterminationHybrid(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Determination(level);
        public override bool CheckRequirement(GamePlayer player) => true;
    }

    public class ModernToughness : RAPropertyEnhancer, IModernRealmAbility
    {
        public ModernToughness(DbAbility ability, int level) : base(ability, level, eProperty.MaxHealth) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Health(level);
        protected override void SendUpdates(GameLiving target) => target.effectListComponent.RequestPlayerUpdate(EffectHelper.PlayerUpdate.Stats | EffectHelper.PlayerUpdate.Status);
    }

    public class ModernEtherealBond : RAPropertyEnhancer, IModernRealmAbility
    {
        public ModernEtherealBond(DbAbility ability, int level) : base(ability, level, eProperty.MaxMana) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int level) => ModernRealmRanks.Power(level);
        protected override void SendUpdates(GameLiving target) => target.effectListComponent.RequestPlayerUpdate(EffectHelper.PlayerUpdate.Stats | EffectHelper.PlayerUpdate.Status);
    }

    public class ModernLightTankDetermination : ModernDetermination, IModernRealmAbility
    {
        public ModernLightTankDetermination(DbAbility ability, int level) : base(ability, level) { }
        public override int CostForUpgrade(int currentLevel) => currentLevel < 0 || currentLevel >= 9 ? 1000 : new[] { 1, 1, 2, 2, 3, 3, 3, 5, 5 }[currentLevel];
    }

    public class ModernSerenity : AtlasOF_SerenityAbility, IModernRealmAbility
    {
        public ModernSerenity(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        public override bool CheckRequirement(GamePlayer player) => true;
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Value(rank, 1, 2, 3, 5, 7);
    }
    public class ModernLifter : AtlasOF_LifterAbility, IModernRealmAbility
    {
        public ModernLifter(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Value(rank, 10, 20, 30, 40, 50, 60, 70, 80, 90);
    }
    public class ModernLongWind : AtlasOF_LongWindAbility, IModernRealmAbility
    {
        public ModernLongWind(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 5;
        // The sprint consumer expects a percentage of its five-point tick cost.
        public override System.Collections.Generic.IList<string> DelveInfo => new System.Collections.Generic.List<string>
        {
            "Reduces sprint endurance consumption by 1/2/3/4/5 points per tick.",
            "Five ranks; incremental costs: 1/3/6/10/14."
        };
    }
    public class ModernMasteryOfFocus : MasteryOfFocusAbility, IModernRealmAbility
    {
        public ModernMasteryOfFocus(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Crit(rank);
    }
    public class ModernPhysicalDefense : PhysicalDefenceAbility, IModernRealmAbility
    {
        public ModernPhysicalDefense(DbAbility ability, int level) : base(ability, level) { }
        public override int MaxLevel => 9;
        public override int CostForUpgrade(int currentLevel) => ModernRealmRanks.PassiveCost(currentLevel);
        public override int GetAmountForLevel(int rank) => ModernRealmRanks.Healing(rank);
    }
}
