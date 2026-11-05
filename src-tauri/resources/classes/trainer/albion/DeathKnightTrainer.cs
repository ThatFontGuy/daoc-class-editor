namespace DOL.GS.Trainer
{
    /// <summary>Trains the Death Knight (the live Necromancer); placed beside each Necromancer trainer.</summary>
    [NPCGuildScript("Death Knight Trainer", eRealm.Albion)]
    public class DeathKnightTrainer : GameTrainer
    {
        public override eCharacterClass TrainedClass => eCharacterClass.DeathKnight;
    }
}
