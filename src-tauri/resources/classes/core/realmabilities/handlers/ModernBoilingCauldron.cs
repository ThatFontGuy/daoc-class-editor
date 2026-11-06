using System.Collections.Generic;
using System.Linq;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    // Warlock RR5. A cauldron at the target boils for 3.5 seconds, then spills 650 heat damage on up to 8 enemies
    // within 350 (1.108, 1.128) and bedazzles them: 30% chance to fumble melee or miscast spells for 20 seconds,
    // unresistable (1.128, 1.129b). 10-minute reuse (1.128).
    public class ModernBoilingCauldron : ModernRr5Ability
    {
        public const int Range = 1500, Radius = 350, Damage = 650, MaxTargets = 8, BoilMs = 3500, BedazzleMs = 20000, BedazzlePercent = 30;
        public ModernBoilingCauldron(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.CurrentRegion == null) return;
            if (living.TargetObject is not GameLiving target || !target.IsAlive || !living.IsWithinRadius(target, Range) ||
                !GameServer.ServerRules.IsAllowedToAttack(living, target, true))
            {
                (living as GamePlayer)?.Out.SendMessage("You need an enemy target within range.", PacketHandler.eChatType.CT_System, PacketHandler.eChatLoc.CL_SystemWindow);
                return;
            }
            living.Stealth(false);
            SendCasterSpellEffectAndCastMessage(living, 7086, true);
            var cauldron = new ModernBoilingCauldronObject(living, target);
            if (cauldron.AddToWorld()) cauldron.Begin();
            DisableSkill(living);
        }
        public static bool IsBedazzled(GameLiving living) => ModernRr5.Get(living, eEffect.ModernBedazzle) != null;
        // Spell half of the bedazzle: a spell that finishes casting can fail.
        public static bool Miscasts(GameLiving caster) => IsBedazzled(caster) && Util.Chance(BedazzlePercent);
        public static void Bedazzle(GameLiving target)
        {
            ModernRr5.Get(target, eEffect.ModernBedazzle)?.End();
            ModernRr5Classic.Buff(target, BedazzleMs, eEffect.ModernBedazzle, "Bedazzled", 3085,
                e => e.Owner.DebuffCategory[eProperty.FumbleChance] += BedazzlePercent,
                e => e.Owner.DebuffCategory[eProperty.FumbleChance] -= BedazzlePercent);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("A cauldron at your target spills after 3.5 seconds: 650 heat damage to up to 8 enemies within 350, who are bedazzled for 20 seconds (30% chance to fumble melee attacks or miscast spells).");
    }

    public class ModernBoilingCauldronObject : GameStaticItem
    {
        private readonly GameLiving _caster;
        private ECSGameTimer _timer;
        public ModernBoilingCauldronObject(GameLiving caster, GameLiving target)
        {
            _caster = caster;
            Name = "Boiling Cauldron"; Model = 2947; Realm = caster.Realm; CurrentRegion = target.CurrentRegion;
            X = target.X; Y = target.Y; Z = target.Z; Heading = caster.Heading;
        }
        public void Begin() => _timer = new ECSGameTimer(this, _ => Spill(), ModernBoilingCauldron.BoilMs);
        private int Spill()
        {
            if (_caster.ObjectState == eObjectState.Active && _caster.CurrentRegion == CurrentRegion)
            {
                Spell spell = ModernRr5Classic.DamageSpell("Boiling Cauldron", ModernBoilingCauldron.Damage, eDamageType.Heat, 0, 7086);
                foreach (GameLiving target in ModernRr5Classic.Enemies(_caster, this, ModernBoilingCauldron.Radius)
                    .OrderBy(t => t.GetDistanceTo(this)).Take(ModernBoilingCauldron.MaxTargets))
                {
                    new ModernRealmDamage(_caster, spell).StartSpell(target);
                    if (target.IsAlive) ModernBoilingCauldron.Bedazzle(target);
                }
            }
            Delete();
            return 0;
        }
        public override void Delete() { _timer?.Stop(); _timer = null; base.Delete(); }
    }
}
