"""
LabWired Telemetry Module

This module provides the instrumentation for tracking the 'Complexity as a Moat' metrics.
It specifically tracks 'AI Operations' and calculates the corresponding 'Simulation Minutes' (SIM_MIN)
which form the basis of the LabWired usage-based monetization model for agents.
"""

import logging
import time
from typing import Dict, Any

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
