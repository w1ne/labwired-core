"""Package and instruction regression checks; no network or live MCP requests."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'plugins' / 'labwired'


class PluginContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads((PLUGIN / '.claude-plugin' / 'plugin.json').read_text())
        cls.marketplace = json.loads((ROOT / '.claude-plugin' / 'marketplace.json').read_text())
        cls.mcp = json.loads((PLUGIN / '.mcp.json').read_text())
        cls.skill = ' '.join((PLUGIN / 'skills' / 'electronics-lab' / 'SKILL.md').read_text().split())
        cls.readme = (PLUGIN / 'README.md').read_text()

    def test_marketplace_resolves_one_matching_plugin(self):
        entries = [p for p in self.marketplace['plugins'] if p['name'] == 'labwired']
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]['source'], './plugins/labwired')
        self.assertEqual(self.marketplace['name'], 'labwired')
        self.assertEqual(self.manifest['name'], entries[0]['name'])

    def test_release_is_versioned(self):
        self.assertEqual(self.manifest['version'], '0.1.1')
        self.assertEqual(self.manifest['license'], 'MIT')

    def test_directory_urls_are_first_party(self):
        expected = {
            'documentationUrl': 'https://labwired.com/mcp.html',
            'supportUrl': 'https://labwired.com/faq.html',
            'privacyPolicyUrl': 'https://labwired.com/privacy',
            'termsOfServiceUrl': 'https://labwired.com/terms',
        }
        for key, value in expected.items():
            self.assertEqual(self.manifest[key], value)

    def test_remote_only_configuration_has_no_bundled_credentials_or_launchers(self):
        self.assertEqual(self.mcp, {'mcpServers': {'labwired': {
            'type': 'http', 'url': 'https://api.labwired.com/mcp'}}})
        self.assertNotIn('hooks', self.manifest)
        self.assertFalse((PLUGIN / 'hooks').exists())

    def test_default_surface_does_not_require_expert_tools(self):
        self.assertIn('labwired_hardware({ request })', self.skill)
        self.assertIn('do not assume that the default endpoint exposes the expert catalog tools', self.skill)
        self.assertIn('Do not stop at catalog discovery', self.skill)

    def test_revisions_preserve_returned_project_and_session(self):
        self.assertIn('labwired_hardware({ request, project_id })', self.skill)
        self.assertIn('exact returned `project_id`', self.skill)
        self.assertIn('returned `session_id`', self.skill)
        self.assertIn('Never invent identifiers', self.skill)

    def test_deliverables_require_evidence(self):
        self.assertIn('Compilation success is not verification', self.skill)
        self.assertIn('only when the relevant simulation assertion passes', self.skill)
        self.assertIn('Deliver manufacturing files only when actually returned', self.skill)
        self.assertIn('does not prove that an embedded viewer rendered', self.skill)

    def test_negative_controls_and_payment_boundary(self):
        self.assertIn('A theory question that needs only an explanation does not require a project or tool call', self.skill)
        self.assertIn('Do not place orders, initiate checkout or charge anything', self.skill)
        self.assertIn('never request credentials in chat', self.skill)

    def test_install_documents_supported_fallback_and_publication_boundary(self):
        self.assertIn('Claude Code 2.1.292 or later', self.readme)
        self.assertIn('claude plugin marketplace add w1ne/labwired-core', self.readme)
        self.assertIn('claude plugin install labwired@labwired', self.readme)
        self.assertIn("Repository installation is separate from approval or publication in Anthropic's directory", self.readme)
        self.assertIn('it does not install the bundled skill', self.readme)


if __name__ == '__main__':
    unittest.main()
