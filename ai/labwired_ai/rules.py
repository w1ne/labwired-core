import yaml
import json
import argparse
from pathlib import Path
from typing import List, Dict, Any

class HardwareRules:
    """
    Formalizes the 'Rules of the Sandbox' that peripheral models must obey.
    This is what agents use to 'verify' their work.
    """

    def __init__(self, device_name: str):
        self.device_name = device_name
        # Ground truth should ideally be derived from verified sources
        # but here we provide a mechanism for agents to compare vs a "Golden Model"
        # or common architectural constraints (e.g., standard UART register layout).
        self.rules = self._load_rules(device_name)

    def _load_rules(self, device_name: str) -> Dict:
        # Placeholder for loading formal specifications
        # or ground truth JSONs from the ai/tests/ directory
        truth_path = Path(__file__).parent.parent / "tests" / f"{device_name.lower()}_ground_truth.json"
        if truth_path.exists():
            with open(truth_path) as f:
                return json.load(f)
        return {}

    def validate_model(self, model_yaml_path: str) -> List[Dict[str, Any]]:
        """
        Compares the generated YAML model against the Hardware Rules.
        """
        with open(model_yaml_path) as f:
            model = yaml.safe_load(f)

        issues = []

        # 1. Structural Checks
        if not model.get("registers"):
            issues.append({"severity": "error", "message": "Model has no registers"})

        # 2. Compliance Checks (if rules exist)
        if self.rules:
            truth_regs = {r['name'].upper(): r for r in self.rules.get('registers', [])}
            model_regs = {r['name'].upper(): r for r in model.get('registers', [])}

            for name, t_reg in truth_regs.items():
                if name not in model_regs:
                    issues.append({
                        "severity": "error",
                        "reg": name,
                        "message": f"Missing required register: {name}",
                        "evidence": "Datasheet Table 5.2"
                    })
                else:
                    m_reg = model_regs[name]
                    # Verify offset
                    if str(m_reg.get('offset')).upper() != str(t_reg.get('offset')).upper():
                        issues.append({
                            "severity": "error",
                            "reg": name,
                            "message": f"Offset mismatch. Expected {t_reg['offset']}, got {m_reg['offset']}"
                        })

        return issues

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, help="Path to generated YAML model")
    parser.add_argument("--device", required=True, help="Device name to compare against")
    args = parser.parse_args()

    validator = HardwareRules(args.device)
    results = validator.validate_model(args.model)

    if not results:
        print(json.dumps({"valid": True, "message": "Sandbox rules obeyed."}, indent=2))
    else:
        print(json.dumps({"valid": False, "issues": results}, indent=2))
