import argparse
import sys
import logging
import json
import os
from .extract import extract_text_from_pdf
from .schematic import analyze_schematic
from .telemetry import UsageTracker
from .llm import (
    discover_registers,
    extract_register_fields,
    extract_behavior,
    generate_peripheral_yaml
)
from .convert_to_ir import convert as convert_to_ir
from .orchestrator import PipelineOrchestrator

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    handlers=[logging.StreamHandler(sys.stdout)]
)
# Force flush on every log
for handler in logging.root.handlers:
    handler.flush = sys.stdout.flush
logger = logging.getLogger(__name__)

def main(args_list=None):
    tracker = UsageTracker()
    parser = argparse.ArgumentParser(description="LabWired AI: Asset Foundry Tools")
    subparsers = parser.add_subparsers(dest="command", help="Available commands")

    # Command: extract-text
    extract_parser = subparsers.add_parser("extract-text", help="Extract text from PDF datasheet")
    extract_parser.add_argument("--pdf", required=True, help="Path to PDF file")
    extract_parser.add_argument("--pages", help="Page range (e.g., '10-15', '5,8,10')")
    extract_parser.add_argument("--output", help="Output text file (default: stdout)")

    # Command: analyze-schematic
    schematic_parser = subparsers.add_parser("analyze-schematic", help="Analyze schematic for components")
    schematic_parser.add_argument("--image", required=True, help="Path to schematic image/PDF")

    # Command: ingest-datasheet (The Multi-Stage Pipeline)
    ingest_parser = subparsers.add_parser("ingest-datasheet", help="Run full multi-stage ingestion on a PDF")
    ingest_parser.add_argument("--pdf", required=True, help="Path to PDF file")
    ingest_parser.add_argument("--pages", required=True, help="Page ranges for registers & behavior (e.g., '6-12')")
    ingest_parser.add_argument("--name", required=True, help="Peripheral name")
    ingest_parser.add_argument("--output", help="Output YAML file")
    ingest_parser.add_argument("--strict-ir", help="Output Strict IR JSON file")

    # Command: auto-ingest (Zero-Touch Pipeline)
    auto_parser = subparsers.add_parser("auto-ingest", help="Zero-touch pipeline: PDF → verified model with retry loop")
    auto_parser.add_argument("--pdf", required=True, help="Path to PDF file")
    auto_parser.add_argument("--pages", required=True, help="Page ranges for registers & behavior (e.g., '6-12')")
    auto_parser.add_argument("--name", required=True, help="Peripheral name")
    auto_parser.add_argument("--output-dir", required=True, help="Output directory for all artifacts")
    auto_parser.add_argument("--max-retries", type=int, default=3, help="Max retry attempts on verification failure")
    auto_parser.add_argument("--auto-approve-threshold", type=float, default=0.9, help="Confidence threshold for auto-approval (0.0-1.0)")

    args = parser.parse_args(args_list)

    try:
        if args.command == "extract-text":
            logger.info(f"Extracting text from {args.pdf}")
            tracker.record_ai_op(1)
            text = extract_text_from_pdf(args.pdf, args.pages)
            if args.output:
                with open(args.output, "w") as f:
                    f.write(text)
                logger.info(f"Wrote output to {args.output}")
            else:
                print(text)

        elif args.command == "analyze-schematic":
            logger.info(f"Analyzing schematic: {args.image}")
            tracker.record_ai_op(2) # Vision is expensive
            results = analyze_schematic(args.image)
            print(json.dumps(results, indent=2))

        elif args.command == "ingest-datasheet":
            logger.info(f"Starting multi-stage ingestion for {args.name}...")
            # Step 1: Extract Text
            tracker.record_ai_op(1)
            full_text = extract_text_from_pdf(args.pdf, args.pages)

            # Stage 1: Register Discovery
            logger.info("Stage 1: Register Discovery...")
            tracker.record_ai_op(1)
            discovered = discover_registers(full_text)
            logger.info(f"Found {len(discovered)} potential registers: {[r['name'] for r in discovered]}")

            # Stage 2: Bit-Mapping
            registers_detail = []
            for reg in discovered:
                logger.info(f"Stage 2: Extracting fields for {reg['name']}...")
                tracker.record_ai_op(1)
                detail = extract_register_fields(full_text, reg['name'])
                if 'offset' not in detail or detail['offset'] == '0x??':
                    detail['offset'] = reg.get('offset', '0x00')
                registers_detail.append(detail)

            # Stage 3: Behavioral Synthesis
            logger.info("Stage 3: Behavioral Synthesis...")
            tracker.record_ai_op(2)
            context = {"registers": registers_detail}
            behaviors = extract_behavior(full_text, context=context)

            # Final Generation
            logger.info("Step 4: Generating YAML...")
            yaml_content = generate_peripheral_yaml(args.name, registers_detail, behaviors)

            if args.output:
                with open(args.output, "w") as f:
                    f.write(yaml_content)
                logger.info(f"Ingestion complete! Wrote to {args.output}")
            else:
                print(yaml_content)

            if args.strict_ir:
                logger.info(f"Converting to Strict IR: {args.strict_ir}...")
                # We need a temporary file if args.output wasn't provided,
                # but for simplicity, we'll assume the user provides a YAML path
                # or we use a default.
                yaml_path = args.output if args.output else f"{args.name}.yaml"
                if not args.output:
                    with open(yaml_path, "w") as f:
                        f.write(yaml_content)

                convert_to_ir(yaml_path, args.strict_ir)
                logger.info(f"Strict IR generated at {args.strict_ir}")

        elif args.command == "auto-ingest":
            logger.info(f"Starting zero-touch pipeline for {args.name}...")
            orchestrator = PipelineOrchestrator(
                pdf_path=args.pdf,
                pages=args.pages,
                name=args.name,
                output_dir=args.output_dir,
                max_retries=args.max_retries,
                auto_approve_threshold=args.auto_approve_threshold,
                tracker=tracker,
            )
            result = orchestrator.run()

            if result.success:
                logger.info(f"Pipeline succeeded: {result.confidence_label} "
                           f"({result.passed}/{result.total}, {result.confidence:.1%})")
                logger.info(f"  YAML: {result.yaml_path}")
                logger.info(f"  IR:   {result.ir_path}")
            else:
                logger.error(f"Pipeline failed after {result.attempts} attempts")
                for err in result.errors:
                    logger.error(f"  - {err}")
                sys.exit(1)

        else:
            parser.print_help()

    except Exception as e:
        logger.error(f"Operation failed: {e}")
        sys.exit(1)
    finally:
        tracker.report()

if __name__ == "__main__":
    main()
