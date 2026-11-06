using System;
using System.Collections.Generic;
using System.Linq;
using DOL.Database;
using DOL.GS.Spells;

namespace DOL.GS.RealmAbilities
{
    // NF-era class RR5s rebuilt to current live values; the legacy handlers they replace reference missing
    // spells or depend on hooks that no longer exist. Sources in docs/LIVE-RA-MODERNIZATION.md.
    public static class ModernRr5Classic
    {
        public static Spell DamageSpell(string name, double damage, eDamageType type, int radius, ushort effect) =>
            new(new DbSpell { Name = name, Type = eSpellType.DirectDamage.ToString(), Target = eSpellTarget.ENEMY.ToString(),
                Damage = damage, DamageType = (int)type, Radius = radius, ClientEffect = effect, Icon = effect }, 50);
        public static IEnumerable<GameLiving> Enemies(GameLiving caster, GameObject center, int radius) =>
            center.GetPlayersInRadius((ushort)radius).Cast<GameLiving>().Concat(center.GetNPCsInRadius((ushort)radius))
                .Where(t => t != caster && t.IsAlive && GameServer.ServerRules.IsAllowedToAttack(caster, t, true)).ToArray();
        public static ModernRr5Effect Buff(GameLiving target, int ms, eEffect type, string name, ushort icon,
            Action<ModernRr5Effect> start = null, Action<ModernRr5Effect> stop = null) =>
            (ModernRr5Effect)ECSGameEffectFactory.Create(new(target, ms, 1),
                (in ECSGameEffectInitParams i) => new ModernRr5Effect(i, type, name, icon, start, stop));
        public static int ArmorAbsorption(GameLiving living)
        {
            DbInventoryItem chest = living.Inventory?.GetItem(eInventorySlot.TorsoArmor);
            return (eObjectType?)chest?.Object_Type switch
            {
                eObjectType.Plate => 34, eObjectType.Chain or eObjectType.Scale => 24,
                eObjectType.Studded or eObjectType.Reinforced => 19, eObjectType.Leather => 10, _ => 0,
            };
        }
    }

    // Valewalker. Group 1000-point 50% ablative for 10 minutes (1.108); the Valewalker gets 2000 points (1.125).
    public class ModernValeDefense : ModernRr5Ability
    {
        public ModernValeDefense(DbAbility ability, int level) : base(ability, level) { }
        public static Spell Ablative(int points) => new(new DbSpell { Name = "Vale Defense", Type = eSpellType.AblativeArmor.ToString(),
            Target = eSpellTarget.SELF.ToString(), Value = points, Damage = 50, Duration = 600, ClientEffect = 7063, Icon = 7063 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7063, true);
            foreach (GameLiving member in ModernGroupRealmAbility.Members(living, 2000))
                new AblativeArmorSpellHandler(living, Ablative(member == living ? 2000 : 1000), GlobalSpellsLines.RealmSpellsSpellLine).ApplyEffectOnTarget(member);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Group within 2000: a 1000-point, 50% ablative for 10 minutes. The Valewalker's own ablative holds 2000 points.");
    }

    // Heretic. Groupmates take 25% less spell damage for 15 seconds (1.128 duration).
    public class ModernFanaticism : ModernRr5Ability
    {
        public const int Value = 25;
        public ModernFanaticism(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7088, true);
            foreach (GameLiving member in ModernGroupRealmAbility.Members(living, 2000))
            {
                ModernRr5.Get(member, eEffect.ModernFanaticism)?.End();
                ModernRr5Classic.Buff(member, 15000, eEffect.ModernFanaticism, "Fanaticism", 3087,
                    e => e.Owner.BaseBuffBonusCategory[eProperty.MagicAbsorption] += Value,
                    e => e.Owner.BaseBuffBonusCategory[eProperty.MagicAbsorption] -= Value);
            }
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Group within 2000: 25% less spell damage taken for 15 seconds.");
    }

    // Paladin. 1.122b: no stat penalty; heal pulses every 3 s for 15 s within 750. 1.129: 10-minute reuse.
    public class ModernSelflessDevotion : ModernRr5Ability
    {
        public ModernSelflessDevotion(DbAbility ability, int level) : base(ability, level) { }
        public static int GroupHeal(int groupSize) => groupSize >= 6 ? 500 : groupSize >= 2 ? 300 : 0;
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || ModernRr5.Get(living, eEffect.ModernSelflessDevotion) != null) return;
            SendCasterSpellEffectAndCastMessage(living, 7039, true);
            ECSGameEffectFactory.Create(new(living, 15000, 1), static (in i) => new ModernSelflessDevotionEffect(i));
            DisableSkill(living);
        }
        public static void Pulse(GameLiving paladin)
        {
            double effectiveness = paladin.GetModified(eProperty.HealingEffectiveness) * 0.01;
            int size = paladin.Group?.MemberCount ?? 1;
            paladin.ChangeHealth(paladin, eHealthChangeType.Spell, (int)(100 * (1 + effectiveness)));
            int heal = (int)(GroupHeal(size) * (1 + effectiveness));
            if (heal <= 0) return;
            foreach (GameLiving member in ModernGroupRealmAbility.Members(paladin, 750, false))
                member.ChangeHealth(paladin, eHealthChangeType.Spell, heal);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Every 3 seconds for 15 seconds: heals you for 100, and groupmates within 750 for 300 (groups of 2-5) or 500 (6 or more). Healing bonuses apply.");
    }

    public class ModernSelflessDevotionEffect : ECSGameAbilityEffect
    {
        public ModernSelflessDevotionEffect(in ECSGameEffectInitParams init) : base(init)
        {
            EffectType = eEffect.ModernSelflessDevotion; PulseFreq = 3000; NextTick = StartTick;
        }
        public override string Name => "Selfless Devotion";
        public override ushort Icon => 3038;
        public override bool HasPositiveEffect => true;
        public override void OnEffectPulse() { if (Owner.IsAlive) ModernSelflessDevotion.Pulse(Owner); }
    }

    // Healer. Cheat death for 20 seconds: a killing blow instead heals at least 30% and grants 5 seconds of damage
    // and crowd-control immunity, which is also granted when the buff runs out.
    public class ModernSputinsLegacy : ModernRr5Ability
    {
        public const int ImmunityMs = 5000;
        public ModernSputinsLegacy(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 900;
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || ModernRr5.Get(living, eEffect.ModernSputinsLegacy) != null) return;
            SendCasterSpellEffectAndCastMessage(living, 7070, true);
            ModernRr5Classic.Buff(living, 20000, eEffect.ModernSputinsLegacy, "Sputin's Legacy", 3069, null, e =>
            {
                if (e.Owner.IsAlive && e.Remaining == 0) Immunity(e.Owner);
            });
            DisableSkill(living);
        }
        public static void Immunity(GameLiving owner) =>
            ModernRr5Classic.Buff(owner, ImmunityMs, eEffect.ModernSputinsImmunity, "Sputin's Legacy", 3069);
        // Returns true when the killing blow was cheated.
        public static bool CheatDeath(GameLiving target, AttackData ad)
        {
            if (ModernRr5.Get(target, eEffect.ModernSputinsLegacy) is not { } legacy || ad.Damage + ad.CriticalDamage < target.Health) return false;
            ad.CriticalDamage = 0;
            ad.Damage = Math.Max(0, target.Health - 1);
            legacy.Remaining = 1; // fired by the deathblow, not by expiry
            legacy.End();
            target.ChangeHealth(target, eHealthChangeType.Spell, target.MaxHealth * Util.Random(30, 100) / 100);
            Immunity(target);
            return true;
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("20 seconds: a killing blow heals you for at least 30% instead. When it fires or expires you gain 5 seconds of damage and crowd-control immunity.");
    }

    // Spiritmaster. Releases the pet to heal the group from a 3200-point pool scaled by the pet's health, at most
    // 600 per member. 1.127c: the pet dies.
    public class ModernSpiritMartyr : ModernRr5Ability
    {
        public const int Pool = 3200, PerMember = 600;
        public ModernSpiritMartyr(DbAbility ability, int level) : base(ability, level) { }
        public static int PoolFor(int petHealthPercent) => Pool * Math.Clamp(petHealthPercent, 0, 100) / 100;
        public override void Execute(GameLiving living)
        {
            GameNPC pet = living.ControlledBrain?.Body;
            if (CannotUse(living) || pet == null || !pet.IsAlive) return;
            SendCasterSpellEffectAndCastMessage(living, 7067, true);
            int pool = PoolFor(pet.HealthPercent);
            foreach (GameLiving member in ModernGroupRealmAbility.Members(living, 2000).OrderBy(m => m.HealthPercent))
            {
                if (pool <= 0) break;
                int heal = Math.Min(Math.Min(PerMember, member.MaxHealth - member.Health), pool);
                if (heal <= 0) continue;
                pool -= member.ChangeHealth(living, eHealthChangeType.Spell, heal);
            }
            pet.Die(living);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Sacrifices your pet to heal the group from a pool of up to 3200 (scaled by the pet's health), at most 600 per member.");
    }

    // Warden. 15 seconds (1.121): style damage doubled; all weapon damage dealt heals the rest of the group.
    public class ModernFuryOfNature : ModernRr5Ability
    {
        public ModernFuryOfNature(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || ModernRr5.Get(living, eEffect.ModernFuryOfNature) != null) return;
            SendCasterSpellEffectAndCastMessage(living, 7072, true);
            ModernRr5Classic.Buff(living, 15000, eEffect.ModernFuryOfNature, "Fury of Nature", 3053);
            DisableSkill(living);
        }
        // Spread heal: the most injured groupmates are healed first.
        public static void SpreadHeal(GameLiving warden, int amount)
        {
            foreach (GameLiving member in ModernGroupRealmAbility.Members(warden, 2000, false).OrderBy(m => m.HealthPercent))
            {
                if (amount <= 0) break;
                int heal = Math.Min(amount, member.MaxHealth - member.Health);
                if (heal > 0) amount -= member.ChangeHealth(warden, eHealthChangeType.Spell, heal);
            }
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("15 seconds: style damage is doubled and 100% of your weapon damage heals your group (not you).");
    }

    // Cleric. 30 seconds: each melee attack on the Cleric has a 50% chance to stun the attacker for 3 seconds.
    public class ModernRetributionOfTheFaithful : ModernRr5Ability
    {
        public ModernRetributionOfTheFaithful(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 300;
        public static readonly Spell Stun = new(new DbSpell { Name = "Retribution of the Faithful", Type = "UnresistableStun",
            Target = eSpellTarget.ENEMY.ToString(), Duration = 3, ClientEffect = 7042, Icon = 7042 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || ModernRr5.Get(living, eEffect.ModernRetribution) != null) return;
            SendCasterSpellEffectAndCastMessage(living, 7042, true);
            ModernRr5Classic.Buff(living, 30000, eEffect.ModernRetribution, "Retribution of the Faithful", 3041);
            DisableSkill(living);
        }
        public static void Retaliate(GameLiving cleric, AttackData ad)
        {
            if (!ad.IsMeleeAttack || ad.Attacker is not GameLiving attacker || !attacker.IsAlive || ModernRr5.Get(cleric, eEffect.ModernRetribution) == null ||
                !Util.Chance(50)) return;
            new UnresistableStunSpellHandler(cleric, Stun, GlobalSpellsLines.RealmSpellsSpellLine).ApplyEffectOnTarget(attacker);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("30 seconds: melee attacks on you have a 50% chance to stun the attacker for 3 seconds.");
    }

    // Vampiir. Group 30-second damage add that returns its damage to the Vampiir as power, plus +20% melee and
    // spell crit (1.128).
    public class ModernMarkOfPrey : ModernRr5Ability
    {
        public const double Dps = 5.1;
        public const int Crit = 20;
        public ModernMarkOfPrey(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7090, true);
            var spell = new Spell(new DbSpell { Name = "Mark of Prey", Type = eSpellType.DamageAdd.ToString(), Target = "Realm", Range = 2000,
                Duration = 30, Damage = Dps, DamageType = (int)eDamageType.Heat, EffectGroup = 99998, ClientEffect = 7090, Icon = 7090 }, 0);
            foreach (GameLiving member in ModernGroupRealmAbility.Members(living, 2000))
            {
                new ModernMarkOfPreyDamageAdd(living, spell, GlobalSpellsLines.RealmSpellsSpellLine).ApplyEffectOnTarget(member);
                ModernRr5.Get(member, eEffect.ModernMarkOfPrey)?.End();
                ModernRr5Classic.Buff(member, 30000, eEffect.ModernMarkOfPrey, "Mark of Prey", 3089,
                    e => { e.Owner.AbilityBonus[eProperty.CriticalMeleeHitChance] += Crit; e.Owner.AbilityBonus[eProperty.CriticalSpellHitChance] += Crit; },
                    e => { e.Owner.AbilityBonus[eProperty.CriticalMeleeHitChance] -= Crit; e.Owner.AbilityBonus[eProperty.CriticalSpellHitChance] -= Crit; });
            }
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Group within 2000: a 30-second damage add that stacks with others and returns its damage to you as power, plus +20% melee and spell critical chance.");
    }

    public class ModernMarkOfPreyDamageAdd : ModernAngerDamageAdd
    {
        public ModernMarkOfPreyDamageAdd(GameLiving caster, Spell spell, SpellLine line) : base(caster, spell, line) { }
        public override void Handle(AttackData hit, double effectiveness)
        {
            if (!AreArgumentsValid(hit, out GameLiving attacker, out GameLiving target)) return;
            AttackData extra = CreateAttackData(SwingDamage(Spell.Damage, hit.Interval), attacker, target);
            target.OnAttackedByEnemy(extra);
            attacker.DealDamage(extra);
            if (Caster.IsAlive && extra.Damage > 0) Caster.ChangeMana(Caster, eManaChangeType.Spell, extra.Damage);
        }
    }

    // Minstrel. Instant mez of every enemy within 750 for 20 seconds (1.108).
    public class ModernCalmingNotes : ModernRr5Ability
    {
        public ModernCalmingNotes(DbAbility ability, int level) : base(ability, level) { }
        public override int GetReUseDelay(int level) => 300;
        public static readonly Spell Mez = new(new DbSpell { Name = "Calming Notes", Type = eSpellType.Mesmerize.ToString(),
            Target = eSpellTarget.ENEMY.ToString(), Duration = 20, ClientEffect = 7045, Icon = 7045 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7045, true);
            foreach (GameLiving target in ModernRr5Classic.Enemies(living, living, 750))
                new MesmerizeSpellHandler(living, Mez, GlobalSpellsLines.RealmSpellsSpellLine).StartSpell(target);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Mesmerizes every enemy within 750 for 20 seconds.");
    }

    // Bainshee. Group (including the Bainshee, 1.128): armor absorption raised by the armor's own absorption for 45 seconds.
    public class ModernSonicBarrier : ModernRr5Ability
    {
        public ModernSonicBarrier(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            SendCasterSpellEffectAndCastMessage(living, 7085, true);
            foreach (GameLiving member in ModernGroupRealmAbility.Members(living, 2000))
            {
                ModernRr5.Get(member, eEffect.ModernSonicBarrier)?.End();
                int bonus = ModernRr5Classic.ArmorAbsorption(member);
                if (bonus <= 0) continue;
                ModernRr5Classic.Buff(member, 45000, eEffect.ModernSonicBarrier, "Sonic Barrier", 3088,
                    e => { e.Remaining = bonus; e.Owner.BaseBuffBonusCategory[eProperty.ArmorAbsorption] += bonus; },
                    e => e.Owner.BaseBuffBonusCategory[eProperty.ArmorAbsorption] -= e.Remaining);
            }
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Group within 2000, including you: armor absorption is doubled for 45 seconds.");
    }

    // Reaver. Instant PBAE drain, 200 damage (1.114b) scaled by Soulrending, returning 75% to the Reaver.
    public class ModernSoulQuench : ModernRr5Ability
    {
        public const int Radius = 350;
        public ModernSoulQuench(DbAbility ability, int level) : base(ability, level) { }
        public static double Damage(GameLiving reaver) => 200 * Math.Clamp(reaver.GetModifiedSpecLevel(Specs.Soulrending) / 50.0, 0.5, 1.5);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            living.Stealth(false);
            Spell spell = ModernRr5Classic.DamageSpell("Soul Quench", Damage(living), eDamageType.Body, 0, 7044);
            foreach (GameLiving target in ModernRr5Classic.Enemies(living, living, Radius))
                new ModernSoulQuenchDamage(living, spell).StartSpell(target);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Drains about 200 health (scaled by Soulrending) from every enemy within 350; you gain 75% of it.");
    }

    public class ModernSoulQuenchDamage : ModernRealmDamage
    {
        public ModernSoulQuenchDamage(GameLiving caster, Spell spell) : base(caster, spell) { }
        public override void DamageTarget(AttackData ad, bool showEffectAnimation)
        {
            base.DamageTarget(ad, showEffectAnimation);
            int drained = (ad.Damage + ad.CriticalDamage) * 3 / 4;
            if (drained > 0 && Caster.IsAlive) Caster.ChangeHealth(Caster, eHealthChangeType.Spell, drained);
        }
    }

    // Wizard. A ward pulses 400 heat damage in a 150 radius every 3 seconds for 15 seconds; the Wizard's 700-point
    // 100% ablative refreshes every 2.5 seconds (1.125, 1.127). 1.129: 10-minute reuse.
    public class ModernWallOfFlame : ModernRr5Ability
    {
        public const int Radius = 150, Damage = 400, Lifetime = 15000, AblativePoints = 700;
        public ModernWallOfFlame(DbAbility ability, int level) : base(ability, level) { }
        public static readonly Spell Ablative = new(new DbSpell { Name = "Wall of Flame", Type = eSpellType.AblativeArmor.ToString(),
            Target = eSpellTarget.SELF.ToString(), Value = AblativePoints, Damage = 100, Duration = 10, ClientEffect = 7086, Icon = 7086 }, 50);
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || living.CurrentRegion == null) return;
            SendCasterSpellEffectAndCastMessage(living, 7086, true);
            var ward = new ModernWallOfFlameWard(living);
            if (ward.AddToWorld()) ward.Begin();
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) =>
            list.Add("Drops a ward that deals 400 heat damage to enemies within 150 every 3 seconds for 15 seconds, while you keep a refreshing 700-point ablative.");
    }

    public class ModernWallOfFlameWard : GameStaticItem
    {
        private readonly GameLiving _caster;
        private readonly long _ends;
        private long _nextDamage, _nextAblative;
        private ECSGameTimer _timer;
        public ModernWallOfFlameWard(GameLiving caster)
        {
            _caster = caster; _ends = GameLoop.GameLoopTime + ModernWallOfFlame.Lifetime;
            Name = "Wall of Flame"; Model = 2656; Realm = caster.Realm; CurrentRegion = caster.CurrentRegion;
            X = caster.X; Y = caster.Y; Z = caster.Z; Heading = caster.Heading;
        }
        public void Begin() => _timer = new ECSGameTimer(this, _ => Pulse(), 1);
        private int Pulse()
        {
            long now = GameLoop.GameLoopTime;
            if (now >= _ends || _caster.ObjectState != eObjectState.Active || _caster.CurrentRegion != CurrentRegion) { Delete(); return 0; }
            if (now >= _nextAblative && _caster.IsAlive)
            {
                new AblativeArmorSpellHandler(_caster, ModernWallOfFlame.Ablative, GlobalSpellsLines.RealmSpellsSpellLine).ApplyEffectOnTarget(_caster);
                _nextAblative = now + 2500;
            }
            if (now >= _nextDamage)
            {
                Spell spell = ModernRr5Classic.DamageSpell("Wall of Flame", ModernWallOfFlame.Damage, eDamageType.Heat, 0, 7086);
                foreach (GameLiving target in ModernRr5Classic.Enemies(_caster, this, ModernWallOfFlame.Radius))
                    new ModernRealmDamage(_caster, spell).StartSpell(target);
                _nextDamage = now + 3000;
            }
            return 500;
        }
        public override void Delete() { _timer?.Stop(); _timer = null; base.Delete(); }
    }

    // Friar. PBAE crush damage within 350 that leaves every target unable to attack for 6 seconds.
    public class ModernWhirlingStaff : ModernRr5Ability
    {
        public const int Radius = 350, Damage = 251, DisarmMs = 6000;
        public ModernWhirlingStaff(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living)) return;
            living.Stealth(false);
            SendCasterSpellEffectAndCastMessage(living, 7043, true);
            Spell spell = ModernRr5Classic.DamageSpell("Whirling Staff", Damage, eDamageType.Crush, 0, 7043);
            foreach (GameLiving target in ModernRr5Classic.Enemies(living, living, Radius))
            {
                new ModernRealmDamage(living, spell).StartSpell(target);
                long now = target.CurrentRegion?.Time ?? GameLoop.GameLoopTime;
                target.DisarmedTime = Math.Max(target.DisarmedTime, now + DisarmMs);
                target.attackComponent.StopAttack();
            }
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("251 crush damage to every enemy within 350; they cannot attack for 6 seconds.");
    }

    // Druid. 1.108: silences the Druid (no longer stuns) for 5 seconds and turns all damage taken into healing.
    public class ModernNaturesWomb : ModernRr5Ability
    {
        public const int DurationMs = 5000;
        public ModernNaturesWomb(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CheckPreconditions(living, DEAD | SITTING) || ModernRr5.Get(living, eEffect.ModernNaturesWomb) != null) return;
            SendCasterSpellEffectAndCastMessage(living, 7071, true);
            living.StopCurrentSpellcast();
            long now = living.CurrentRegion?.Time ?? GameLoop.GameLoopTime;
            living.SilencedTime = Math.Max(living.SilencedTime, now + DurationMs);
            ModernRr5Classic.Buff(living, DurationMs, eEffect.ModernNaturesWomb, "Nature's Womb", 3052);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("5 seconds: you are silenced and all damage you take heals you instead.");
    }

    // Cabalist. The pet is immune to crowd control for 60 seconds; existing effects are not removed.
    public class ModernResoluteMinion : ModernRr5Ability
    {
        public ModernResoluteMinion(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            GameNPC pet = living.ControlledBrain?.Body;
            if (CannotUse(living) || pet == null || !pet.IsAlive) return;
            SendCasterSpellEffectAndCastMessage(living, 7050, true);
            ModernRr5.Get(pet, eEffect.ModernResoluteMinion)?.End();
            ModernRr5Classic.Buff(pet, 60000, eEffect.ModernResoluteMinion, "Resolute Minion", 3046);
            DisableSkill(living);
        }
        public override void AddEffectsInfo(IList<string> list) => list.Add("Your pet is immune to crowd control for 60 seconds. Effects already on it remain.");
    }

    // Champion. For 20 seconds the target's armor absorption increases melee damage instead of reducing it.
    // 1.121: 10-minute reuse.
    public class ModernBadgeOfValor : ModernRr5Ability
    {
        public ModernBadgeOfValor(DbAbility ability, int level) : base(ability, level) { }
        public override void Execute(GameLiving living)
        {
            if (CannotUse(living) || ModernRr5.Get(living, eEffect.ModernBadgeOfValor) != null) return;
            SendCasterSpellEffectAndCastMessage(living, 7057, true);
            ModernRr5Classic.Buff(living, 20000, eEffect.ModernBadgeOfValor, "Badge of Valor", 3056);
            DisableSkill(living);
        }
        // damageMod already carries (1 - absorb); turn that into (1 + absorb).
        public static double ArmorFactor(GameLiving attacker, double absorb) =>
            ModernRr5.Get(attacker, eEffect.ModernBadgeOfValor) == null || absorb <= 0 || absorb >= 1 ? 1 : (1 + absorb) / (1 - absorb);
        public override void AddEffectsInfo(IList<string> list) => list.Add("20 seconds: your target's armor absorption increases your melee damage instead of reducing it.");
    }
}
