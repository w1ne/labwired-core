"""
LabWired Telemetry Module

This module provides the instrumentation for tracking the 'Complexity as a Moat' metrics.
It specifically tracks 'AI Operations' and calculates the corresponding 'Simulation Minutes' (SIM_MIN)
which form the basis of the LabWired usage-based monetization model for agents.
"""

import logging
import os
import time
from typing import Dict, Any, Optional

logger = logging.getLogger(__name__)

class UsageTracker:
    """
    Main telemetry engine for the LabWired AIPi.

    Tracks real-time execution duration and AI-specific operations to provide
    a bit-accurate usage summary for external agents.
    """

    def __init__(self):
        """Initialize the tracker with a start timestamp and zero counters."""
        self.start_time = time.time()
        self.ai_ops = 0
        self.sim_min = 0.0
        self.op_type = "ai_ingest"

    def record_ai_op(self, count: int = 1):
        """
        Increment the count of AI-driven synthesis operations.

        Args:
            count: Number of discrete LLM/VLM requests executed.
        """
        self.ai_ops += count

    def calculate_usage(self) -> Dict[str, Any]:
        """
        Finalize usage stats and calculate the equivalent Simulation Minutes.

        Returns:
            A dictionary containing ai_operations, simulation_minutes, and a status message.
        """
        duration_sec = time.time() - self.start_time
        # Heuristic: 1 AI op = 0.5 simulation minutes in terms of complexity/moat value
        self.sim_min = (self.ai_ops * 0.5) + (duration_sec / 60.0)

        return {
            "ai_operations": self.ai_ops,
            "simulation_minutes": round(self.sim_min, 2),
            "op_type": self.op_type,
            "status": "LabWired AIPi: Usage Recorded"
        }

    def report(self):
        """
        Log usage to stdout in a format parsable by external agents.
        """
        stats = self.calculate_usage()
        print(f"\n[TELEMETRY] {stats['status']}")
        print(f"  AI Operations: {stats['ai_operations']}")
        print(f"  Simulation Minutes Utilized: {stats['simulation_minutes']}")

        # Export to Foundry if configured
        self.export_to_foundry()

    def export_to_foundry(
        self,
        api_url: Optional[str] = None,
        api_key: Optional[str] = None,
    ):
        """
        Export usage telemetry to the Foundry backend.

        Sends a POST request with operation type and simulation minutes.
        Only active when LABWIRED_FOUNDRY_URL and LABWIRED_API_KEY env vars are set,
        or when explicit parameters are provided.
        """
        url = api_url or os.environ.get("LABWIRED_FOUNDRY_URL")
        key = api_key or os.environ.get("LABWIRED_API_KEY")

        if not url or not key:
            return  # Silent no-op when not configured

        stats = self.calculate_usage()
        payload = {
            "op_type": stats["op_type"],
            "ai_operations": stats["ai_operations"],
            "sim_minutes": stats["simulation_minutes"],
        }

        try:
            import urllib.request
            import json

            req = urllib.request.Request(
                f"{url.rstrip('/')}/v1/telemetry/ingest",
                data=json.dumps(payload).encode("utf-8"),
                headers={
                    "Content-Type": "application/json",
                    "X-API-Key": key,
                },
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=5) as resp:
                if resp.status == 200 or resp.status == 201:
                    logger.debug("Telemetry exported to Foundry")
                else:
                    logger.debug(f"Telemetry export returned status {resp.status}")
        except Exception as e:
            # Never fail the pipeline due to telemetry export issues
            logger.debug(f"Telemetry export failed (non-fatal): {e}")
