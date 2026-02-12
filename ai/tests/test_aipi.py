import unittest
import time
from unittest.mock import MagicMock, patch
from labwired_ai.telemetry import UsageTracker
from labwired_ai.schematic import SchematicAnalyzer

class TestAIPi(unittest.TestCase):
    """Unit tests for the Agentic Interface for Peripheral Ingestion (AIPi)."""

    def test_telemetry_usage_calculation(self):
        """Verify that simulation minutes are calculated correctly based on AI ops and duration."""
        tracker = UsageTracker()

        # Record some AI operations
        tracker.record_ai_op(2)

        # Simulate some passage of time
        tracker.start_time = time.time() - 60  # Mock 1 minute elapsed

        stats = tracker.calculate_usage()

        # 2 AI ops * 0.5 = 1.0 SIM_MIN
        # 1 minute duration = 1.0 SIM_MIN
        # Total should be around 2.0
        self.assertEqual(stats['ai_operations'], 2)
        self.assertAlmostEqual(stats['simulation_minutes'], 2.1, delta=0.5)

    @patch('labwired_ai.schematic.LLMClient')
    def test_schematic_analysis_protocol(self, mock_llm_client):
        """Verify that the schematic analyzer follows the VLM protocol and parses JSON correctly."""
        mock_instance = mock_llm_client.return_value
        mock_instance.complete.return_value = '[{"designator": "U5", "part_number": "MCP9808", "bus": "I2C1", "evidence": "Traced SDA to PA10"}]'

        analyzer = SchematicAnalyzer()
        results = analyzer.extract_netlist("mock_schematic.png")

        self.assertEqual(len(results), 1)
        self.assertEqual(results[0]['part_number'], "MCP9808")
        self.assertEqual(results[0]['bus'], "I2C1")

if __name__ == '__main__':
    unittest.main()
