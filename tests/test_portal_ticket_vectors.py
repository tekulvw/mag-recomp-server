"""The Medius verifier tests read a copy of the portal's ticket vectors; keep the two identical."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


class PortalTicketVectorCopyTests(unittest.TestCase):
    def test_medius_copy_matches_portal_vectors(self):
        portal = (ROOT / 'portal' / 'tests' / 'ticket-vectors.json').read_bytes()
        medius = (ROOT / 'horizon' / 'Tests.MagPlugin' / 'portal-ticket-vectors.json').read_bytes()
        self.assertEqual(portal, medius, 'copy portal/tests/ticket-vectors.json to horizon/Tests.MagPlugin/portal-ticket-vectors.json')


if __name__ == '__main__':
    unittest.main()
