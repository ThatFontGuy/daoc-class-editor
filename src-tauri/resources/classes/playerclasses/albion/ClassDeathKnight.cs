using System.Collections.Generic;
using DOL.GS.Realm;

namespace DOL.GS.PlayerClass
{
    /// <summary>
    /// Death Knight: the live 1.122B+ Necromancer (forms, direct casting and melee, death servants), offered beside the
    /// classic shade Necromancer (class 12), which stays unchanged. It deliberately does not derive from ClassDisciple,
    /// whose shade, pet-health and combat-lock behavior belongs to the classic design. It shares the Necromancer's
    /// armor, items, stats and races.
    /// </summary>
    [CharacterClass((int)eCharacterClass.DeathKnight, "Death Knight", "Disciple")]
    public class ClassDeathKnight : CharacterClassBase
    {
        public ClassDeathKnight()
        {
            m_profession = "PlayerClass.Profession.TempleofArawn";
            m_specializationMultiplier = 10;
            m_primaryStat = eStat.INT;
            m_secondaryStat = eStat.DEX;
            m_tertiaryStat = eStat.QUI;
            m_manaStat = eStat.INT;
            m_wsbase = 280;
            m_baseHP = 560;
        }

        public override eClassType ClassType => eClassType.ListCaster;

        public override void Init(GamePlayer player)
        {
            base.Init(player);
            // Nethersbane is removed when its wielder logs out (live 1.122B).
            foreach (DOL.Database.DbInventoryItem item in System.Linq.Enumerable.ToList(System.Linq.Enumerable.Where(player.Inventory.AllItems, i => i.Id_nb == Spells.DeathKnight.Nethersbane)))
                player.Inventory.RemoveItem(item);
        }

        public override GameTrainer.eChampionTrainerType ChampionTrainerType() => GameTrainer.eChampionTrainerType.Disciple;

        public override bool HasAdvancedFromBaseClass() => true;

        public override List<PlayerRace> EligibleRaces => ServerProperties.Properties.ENABLE_DEATH_KNIGHT_TEST
            ? new List<PlayerRace> { PlayerRace.Avalonian, PlayerRace.Briton, PlayerRace.Inconnu, PlayerRace.Saracen }
            : new List<PlayerRace>();
    }
}
