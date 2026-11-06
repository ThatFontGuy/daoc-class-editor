using System.Collections.Generic;
using DOL.Database;

namespace DOL.GS.RealmAbilities
{
    /// <summary>Target-free Charge for the Valkyrie; legacy Charge is enemy-player-only.</summary>
    public sealed class ValkyrieChargeAbility : TimedRealmAbility
    {
        public ValkyrieChargeAbility(DbAbility ability,int level) : base(ability,level) { }
        public override int GetReUseDelay(int level) => ServerProperties.Properties.USE_NEW_ACTIVES_RAS_SCALING
            ? level switch { 1=>900,2=>600,3=>300,4=>180,_=>90 }
            : level switch { 1=>900,2=>300,_=>90 };
        public override bool CheckRequirement(GamePlayer player) => player.Level >= 40;
        public override void Execute(GameLiving living)
        {
            if (CheckPreconditions(living,DEAD|SITTING|MEZZED|STUNNED) ||
                living.effectListComponent.ContainsEffectForEffectType(eEffect.MovementSpeedDebuff) ||
                living.effectListComponent.ContainsEffectForEffectType(eEffect.Charge)) return;
            ECSGameEffectFactory.Create(new(living,15000,1,null),static (in i)=>new ChargeECSGameEffect(i));
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list)
        {
            list.Add("Charge for 15 seconds with movement speed and crowd-control immunity. Does not remove existing snares or roots.");
            list.Add("Target: Self");
        }
    }
}
