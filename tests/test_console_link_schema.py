"""Keep the .NET integration fixture identical to the portal-owned link migration."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ConsoleSchemaContract(unittest.TestCase):
    def test_dotnet_fixture_is_portal_migration(self):
        self.assertEqual(
            (ROOT / 'portal/migrations/0009_external_identities.sql').read_bytes(),
            (ROOT / 'horizon/Tests.MagPlugin/console-link-schema.sql').read_bytes(),
        )
