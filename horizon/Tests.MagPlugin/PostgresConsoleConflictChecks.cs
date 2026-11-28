using Npgsql;
using RT.Models.MAG;
using Server.Medius;

internal static partial class LeadershipProgressionChecks
{
    public static void PostgresConsoleConflicts() => WithPostgres("postgres console character conflicts", (database, connection) =>
    {
        LocalCharacterStore Store() => LocalCharacterStore.ForPostgres(database);
        MagCreateCharacterRequest Request(string name = "ConsoleUser!0") => new MagCreateCharacterRequest { AccountName = name, Faction = 1, UniverseId = 26, NpId = ConsoleNp() };
        byte[] ConsoleNp() { var np = new byte[36]; System.Text.Encoding.ASCII.GetBytes("ConsoleUser").CopyTo(np, 0); return np; }
        var store = Store();
        Check(store.CreateForAccount(Request(), "foreignowner", "ConsoleUser").Result == MagNetResult.Success, "PG conflict fixture");
        Check(store.CreateForAccount(Request(), "portalowner", "ConsoleUser").Result == MagNetResult.NameTaken, "PG explicit opt-in required");
        var outcomes = new CharacterCreationOutcome[12];
        Parallel.For(0, outcomes.Length, i => outcomes[i] = Store().CreateForAccount(Request(), "portalowner", "ConsoleUser", true));
        Check(outcomes.All(o => o.Result == MagNetResult.Success), "PG concurrent exact retries converge");
        Check(Scalar(connection, "SELECT revision FROM mag.characters WHERE account='foreignowner'") == 1, "Foreign holder must not be rewritten by fallback/retries");
        var assigned = Store().ReadForAccount("portalowner").Single();
        Check(assigned.ConsoleRequestedName == "ConsoleUser!0" && assigned.Name != "ConsoleUser!0", "PG alias persisted");
        Check(store.CreateForAccount(Request(), "otherowner", "ConsoleUser", true).Result == MagNetResult.Success, "PG aliases across owners");
        Check(Scalar(connection, "SELECT count(*) FROM mag.characters WHERE account='portalowner'") == 1, "PG retry did not duplicate");
        DualAuthenticationChecks.ConflictWire(Store(), fresh: true);
        using (var reopened = new PostgresCharacterDatabase(connection))
        {
            reopened.RequireCurrentSchema();
            DualAuthenticationChecks.ConflictWire(LocalCharacterStore.ForPostgres(reopened));
        }
        Check(Scalar(connection, "SELECT count(*) FROM pg_indexes WHERE schemaname='mag' AND indexname='characters_live_console_alias_key' AND indexdef LIKE '%WHERE%deleted_at IS NULL%'") == 1, "Live partial alias index");
        try
        {
            Sql(connection, "UPDATE mag.characters SET console_requested_name='ConsoleUser!0' WHERE account='portalowner'");
            // A second live character of this owner cannot acquire the same alias, even outside the store API.
            Check(store.CreateForAccount(Request("Distinct!1"), "portalowner", "ConsoleUser").Result == MagNetResult.Success, "PG second slot fixture");
            Sql(connection, "UPDATE mag.characters SET console_requested_name='consoleuser!0' WHERE account='portalowner' AND name='Distinct!1'");
            throw new Exception("Database accepted duplicate live owner alias");
        }
        catch (PostgresException e) when (e.SqlState == PostgresErrorCodes.UniqueViolation) { }
        var extra = store.ReadForAccount("portalowner").Single(c => c.Name == "Distinct!1");
        Check(store.DeleteCharacterForAccount("portalowner", extra.Id, extra.Name).Result == MagNetResult.Success, "PG cleanup second fixture");
        DualAuthenticationChecks.ConflictDeletion(Store());
        DualAuthenticationChecks.ConflictBounds(Store());
        DualAuthenticationChecks.RestoreChosenNameChecks(Store());
        // Force genuine insert uniqueness races; sequence increments survive transaction rollback.
        // Repeated collisions must stop after the existing three transaction attempts, with no saved alias.
        Sql(connection, "CREATE SEQUENCE mag.console_collision_attempts; " +
            "CREATE FUNCTION mag.console_force_collision() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN " +
            "IF NEW.account = 'boundedrace' THEN PERFORM nextval('mag.console_collision_attempts'); " +
            "RAISE EXCEPTION 'synthetic name race' USING ERRCODE = '23505'; END IF; RETURN NEW; END $$; " +
            "CREATE TRIGGER console_force_collision BEFORE INSERT ON mag.characters FOR EACH ROW EXECUTE FUNCTION mag.console_force_collision()");
        try
        {
            store.CreateForAccount(Request(), "boundedrace", "ConsoleUser", true);
            throw new Exception("Unending unique name races must not report success");
        }
        catch (IOException) { }
        Check(Scalar(connection, "SELECT last_value FROM mag.console_collision_attempts") == 3 && store.ReadForAccount("boundedrace").Count == 0,
            "Fallback insert races have a hard retry bound and leave no partial alias");
        Sql(connection, "DROP TRIGGER console_force_collision ON mag.characters; DROP FUNCTION mag.console_force_collision(); DROP SEQUENCE mag.console_collision_attempts");
        var concurrent = new CharacterCreationOutcome[8];
        Parallel.For(0, concurrent.Length, i => concurrent[i] = Store().CreateForAccount(Request("Racing!1"), "racer" + i, "ConsoleUser", true));
        Check(concurrent.All(o => o.Result == MagNetResult.Success), "Cross-owner concurrent names converge");
        var names = Enumerable.Range(0, concurrent.Length).Select(i => Store().ReadForAccount("racer" + i).Single().Name).ToArray();
        Check(names.Distinct(StringComparer.OrdinalIgnoreCase).Count() == names.Length, "PG assigned names remain unique");
        string longRequested = new string('N', 30) + "!0";
        DualAuthenticationChecks.ConflictWire(Store(), fresh: true, requestedName: longRequested);
        using (var reopened = new PostgresCharacterDatabase(connection))
        {
            reopened.RequireCurrentSchema();
            DualAuthenticationChecks.ConflictWire(LocalCharacterStore.ForPostgres(reopened), requestedName: longRequested);
        }
    });
}
