"""
LabWired Pipeline Orchestrator

Chains the full datasheet-to-verified-model pipeline:
  PDF → text extraction → register discovery → bitfield extraction →
  behavioral synthesis → YAML generation → IR conversion → verification

Supports automatic retry with LLM feedback on verification failures,
and confidence scoring for auto-approve vs human review.
"""

import json
import logging
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

from .extract import extract_text_from_pdf
from .llm import (
    discover_registers,
    extract_register_fields,
    extract_behavior,
    generate_peripheral_yaml,
)
from .convert_to_ir import convert as convert_to_ir
from .telemetry import UsageTracker

logger = logging.getLogger(__name__)


@dataclass
class PipelineResult:
    """Result of a full pipeline run."""

    success: bool
    yaml_path: Optional[str] = None
    ir_path: Optional[str] = None
    passed: int = 0
    total: int = 0
    confidence: float = 0.0
    confidence_label: str = "unknown"
    attempts: int = 0
    errors: list = field(default_factory=list)


class PipelineOrchestrator:
    """
    End-to-end orchestrator for zero-touch peripheral model generation.

    Usage:
        orchestrator = PipelineOrchestrator(
            pdf_path="datasheet.pdf",
            pages="6-12",
            name="USART",
            output_dir="out/usart",
        )
        result = orchestrator.run()
    """

    def __init__(
        self,
        pdf_path: str,
        pages: str,
        name: str,
        output_dir: str,
        max_retries: int = 3,
        auto_approve_threshold: float = 0.9,
        tracker: Optional[UsageTracker] = None,
    ):
        self.pdf_path = pdf_path
        self.pages = pages
        self.name = name
        self.output_dir = Path(output_dir)
        self.max_retries = max_retries
        self.auto_approve_threshold = auto_approve_threshold
        self.tracker = tracker or UsageTracker()

        self.output_dir.mkdir(parents=True, exist_ok=True)

    def run(self) -> PipelineResult:
        """Execute the full pipeline with retry loop."""
        result = PipelineResult(success=False)

        # Step 1: Extract text from PDF
        logger.info(f"Extracting text from {self.pdf_path} (pages: {self.pages})")
        self.tracker.record_ai_op(1)
        try:
            full_text = extract_text_from_pdf(self.pdf_path, self.pages)
        except Exception as e:
            result.errors.append(f"PDF extraction failed: {e}")
            return result

        # Step 2: Register discovery
        logger.info("Stage 1: Register Discovery...")
        self.tracker.record_ai_op(1)
        try:
            discovered = discover_registers(full_text)
        except Exception as e:
            result.errors.append(f"Register discovery failed: {e}")
            return result
        logger.info(
            f"Found {len(discovered)} registers: {[r['name'] for r in discovered]}"
        )

        # Retry loop: stages 2-4 + verification
        feedback_context = ""
        for attempt in range(1, self.max_retries + 1):
            result.attempts = attempt
            logger.info(f"--- Attempt {attempt}/{self.max_retries} ---")

            # Stage 2: Bit-mapping
            registers_detail = []
            for reg in discovered:
                logger.info(f"Stage 2: Extracting fields for {reg['name']}...")
                self.tracker.record_ai_op(1)
                try:
                    detail = extract_register_fields(full_text, reg["name"])
                    if "offset" not in detail or detail["offset"] == "0x??":
                        detail["offset"] = reg.get("offset", "0x00")
                    registers_detail.append(detail)
                except Exception as e:
                    logger.warning(f"Field extraction failed for {reg['name']}: {e}")
                    registers_detail.append(
                        {"name": reg["name"], "offset": reg.get("offset", "0x00"), "fields": []}
                    )

            # Stage 3: Behavioral synthesis (include feedback from previous attempt)
            logger.info("Stage 3: Behavioral Synthesis...")
            self.tracker.record_ai_op(2)
            context = {"registers": registers_detail}
            if feedback_context:
                context["verification_feedback"] = feedback_context
            try:
                behaviors = extract_behavior(full_text, context=context)
            except Exception as e:
                result.errors.append(f"Behavioral synthesis failed (attempt {attempt}): {e}")
                continue

            # Stage 4: YAML generation
            logger.info("Stage 4: Generating YAML...")
            try:
                yaml_content = generate_peripheral_yaml(
                    self.name, registers_detail, behaviors
                )
            except Exception as e:
                result.errors.append(f"YAML generation failed (attempt {attempt}): {e}")
                continue

            yaml_path = self.output_dir / f"{self.name}.yaml"
            yaml_path.write_text(yaml_content)
            result.yaml_path = str(yaml_path)

            # Stage 5: IR conversion
            logger.info("Stage 5: Converting to Strict IR...")
            ir_path = self.output_dir / f"{self.name}.ir.json"
            try:
                convert_to_ir(str(yaml_path), str(ir_path))
            except Exception as e:
                result.errors.append(f"IR conversion failed (attempt {attempt}): {e}")
                continue
            result.ir_path = str(ir_path)

            # Stage 6: Verification
            logger.info("Stage 6: Verification...")
            self.tracker.record_ai_op(1)
            verification = self._run_verification(ir_path)

            result.passed = verification["passed"]
            result.total = verification["total"]

            if verification["total"] > 0:
                result.confidence = verification["passed"] / verification["total"]
            else:
                result.confidence = 0.0

            result.confidence_label = self._classify_confidence(result.confidence)

            logger.info(
                f"Verification: {result.passed}/{result.total} "
                f"(confidence: {result.confidence:.1%}, label: {result.confidence_label})"
            )

            if result.confidence >= self.auto_approve_threshold:
                logger.info("Auto-approved: confidence meets threshold.")
                result.success = True
                break

            if attempt < self.max_retries:
                # Build feedback for next LLM attempt
                feedback_context = self._build_feedback(verification)
                logger.info(f"Retrying with verification feedback...")
            else:
                logger.warning(
                    f"Max retries reached. Final confidence: {result.confidence:.1%}"
                )
                # Still mark as success if confidence is reasonable
                if result.confidence >= 0.5:
                    result.success = True

        # Write summary
        summary_path = self.output_dir / "pipeline_summary.json"
        summary_path.write_text(
            json.dumps(
                {
                    "name": self.name,
                    "success": result.success,
                    "confidence": round(result.confidence, 4),
                    "confidence_label": result.confidence_label,
                    "passed": result.passed,
                    "total": result.total,
                    "attempts": result.attempts,
                    "yaml_path": result.yaml_path,
                    "ir_path": result.ir_path,
                    "errors": result.errors,
                },
                indent=2,
            )
        )
        logger.info(f"Pipeline summary written to {summary_path}")

        return result

    def _run_verification(self, ir_path: Path) -> dict:
        """Run the verify harness and collect structured results."""
        try:
            from .verify_harness import verify_structured

            result = verify_structured(str(ir_path))
            return {
                "exit_code": result.get("exit_code", 1),
                "passed": result.get("passed", 0),
                "failed": result.get("failed", 0),
                "total": result.get("total", 0),
            }

        except ImportError:
            logger.warning(
                "labwired Python module not available; skipping live verification"
            )
            # Fall back to structural validation only
            return self._structural_validation(ir_path)

    def _structural_validation(self, ir_path: Path) -> dict:
        """Validate IR structure without running simulation."""
        try:
            with open(ir_path) as f:
                ir_data = json.load(f)

            peripherals = ir_data.get("peripherals", {})
            if not peripherals:
                return {"exit_code": 1, "passed": 0, "failed": 1, "total": 1, "log": "No peripherals in IR"}

            passed = 0
            failed = 0

            for pname, pdata in peripherals.items():
                registers = pdata.get("registers", [])
                for reg in registers:
                    # Check required fields
                    if reg.get("name") and isinstance(reg.get("offset"), int):
                        passed += 1
                    else:
                        failed += 1

            return {
                "exit_code": 0 if failed == 0 else 1,
                "passed": passed,
                "failed": failed,
                "total": passed + failed,
                "log": f"Structural validation: {passed} passed, {failed} failed",
            }

        except Exception as e:
            return {"exit_code": 1, "passed": 0, "failed": 1, "total": 1, "log": str(e)}

    def _classify_confidence(self, confidence: float) -> str:
        """Classify confidence into human-readable labels."""
        if confidence >= self.auto_approve_threshold:
            return "auto-approved"
        elif confidence >= 0.5:
            return "needs-review"
        else:
            return "needs-rework"

    def _build_feedback(self, verification: dict) -> str:
        """Build feedback context for the LLM retry."""
        lines = [
            f"Previous verification attempt failed ({verification['passed']}/{verification['total']} passed).",
            "Verification log:",
            verification.get("log", "(no log)"),
            "",
            "Please fix the behavioral model to address the failures above.",
        ]
        return "\n".join(lines)
