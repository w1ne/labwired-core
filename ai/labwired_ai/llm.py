import os
import logging
from typing import Optional, Dict, Any, List
from openai import OpenAI
from dotenv import load_dotenv

# Load environment variables from .env file
load_dotenv(os.path.join(os.path.dirname(__file__), "..", ".env"))

logger = logging.getLogger(__name__)

class LLMClient:
    """Client for LLM interaction via xAI (OpenAI-compatible)."""

    def __init__(self, provider: str = "xai", model: str = "grok-4-1-fast-reasoning"):
        self.provider = provider
        self.model = model

        api_key = os.getenv("XAI_API_KEY")
        base_url = os.getenv("XAI_BASE_URL", "https://api.x.ai/v1")

        if not api_key:
            logger.warning("XAI_API_KEY not found in environment. LLM calls will fail.")
            self.client = None
        else:
            self.client = OpenAI(
                api_key=api_key,
                base_url=base_url
            )

    def complete(self, prompt: str, system_prompt: str = "You are a helpful assistant.") -> str:
        """Complete the prompt using the configured model."""
        if not self.client:
            raise ValueError("LLMClient not initialized: Missing API Key.")

        logger.info(f"Sending prompt to {self.provider}/{self.model} (Length: {len(prompt)})")

        try:
            response = self.client.chat.completions.create(
                model=self.model,
                messages=[
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": prompt},
                ]
            )
            return response.choices[0].message.content
        except Exception as e:
            logger.error(f"LLM call failed: {e}")
            raise

def discover_registers(text: str) -> List[Dict[str, Any]]:
    """Stage 1: Identify all registers and their offsets from text with reasoning."""
    client = LLMClient()

    system_prompt = """
    You are an expert hardware engineer. Your primary goal is accuracy and fidelity.
    When analyzing datasheet text, be extremely pedantic about register names and memory offsets.
    """

    prompt = f"""
    Step 1: Analyze the provided datasheet text carefully.
    Step 2: List every memory-mapped register found.
    Step 3: For each register, capture its exact Name and Hex Offset.
    Step 4: Cite the specific snippet or context where this was found to ensure grounding.

    Text:
    {text}

    Respond ONLY with a JSON list of objects:
    [
        {{
            "name": "REG_NAME",
            "offset": "0x00",
            "evidence": "Snippet from text showing this register",
            "reasoning": "Brief explanation of why this offset was chosen"
        }},
        ...
    ]
    """

    response = client.complete(prompt, system_prompt=system_prompt)
    return _parse_json(response, default=[])

def extract_register_fields(text: str, register_name: str) -> Dict[str, Any]:
    """Stage 2: Extract detailed bitfield definitions for a specific register with grounding."""
    client = LLMClient()

    system_prompt = """
    You are an expert embedded systems engineer.
    You excel at mapping bits to functions. Accuracy is paramount for simulation correctness.
    """

    prompt = f"""
    Task: Extract the bitfield mapping for the register: {register_name}.

    Step 1: Find the bit definition table or description for {register_name}.
    Step 2: Identify each bit/field.
    Step 3: Determine bit_range [start, end], access (ReadWrite, ReadOnly, WriteOnly), and reset_value.
    Step 4: Provide reasoning for ambiguous bits (e.g., 'reserved', 'must be 1').

    Text:
    {text}

    Respond ONLY with a JSON object:
    {{
        "name": "{register_name}",
        "offset": "0x??",
        "reset_value": "0x??",
        "access": "ReadWrite",
        "fields": [
            {{
                "name": "FIELD",
                "bit_range": [0, 0],
                "description": "Functional description",
                "evidence": "Source text for this field",
                "reasoning": "Logic for bit range or access type"
            }},
            ...
        ]
    }}
    """

    response = client.complete(prompt, system_prompt=system_prompt)
    return _parse_json(response, default={"name": register_name, "fields": []})

def extract_behavior(text: str, context: Optional[Dict] = None) -> List[Dict[str, Any]]:
    """Stage 3: Identify side effects and causal relationships using discovered register/field context."""
    client = LLMClient()

    system_prompt = """
    You are an expert hardware simulation engineer.
    Your goal is to detect side effects and causal logic that standard SVD files miss.
    This is our PLATFORM MOAT: we find the behavior that others ignore.
    """

    # Format the context to be more readable for the LLM
    registers_detail = context.get('registers', []) if context else []
    context_blob = json.dumps(registers_detail, indent=2) if registers_detail else "None"

    prompt = f"""
    Context (Known Registers and Fields):
    {context_blob}

    Task: Deeply analyze the datasheet text to synthesize simulation behaviors.

    Step 1: Look for phrases like 'setting X start Y', 'clearing A stops B', 'interrupt is triggered when', 'read action resets'.
    Step 2: Map these to concrete Triggers (Read/Write to specific fields) and Actions (State changes, Interrupts, Delays).
    Step 3: Extract timing information (in cycles or microseconds) if mentioned.
    Step 4: Provide reasoning for each behavior to justify its inclusion in the simulation.

    Text:
    {text}

    Respond ONLY with a JSON list of objects:
    [
        {{
            "trigger": {{
                "event": "write",
                "register": "NAME",
                "field": "NAME",
                "value": "1",
                "description": "Setting shutdown bit"
            }},
            "action": {{
                "type": "state_change",
                "target": "Global",
                "delay_cycles": 0,
                "description": "Functional action description"
            }},
            "reasoning": "Causal logic found in text",
            "evidence": "Sentence from datasheet"
        }}
    ]
    """

    response = client.complete(prompt, system_prompt=system_prompt)
    return _parse_json(response, default=[])

import json

def _parse_json(response: str, default: Any) -> Any:
    """Helper to parse JSON from LLM response strings."""
    try:
        json_str = response.strip()
        if json_str.startswith("```json"):
            json_str = json_str[7:-3].strip()
        elif json_str.startswith("```"):
            json_str = json_str[3:-3].strip()

        return json.loads(json_str)
    except Exception as e:
        logger.error(f"Failed to parse LLM response: {e}\nResponse: {response}")
        return default

def generate_peripheral_yaml(name: str, registers: List[Dict], behaviors: List[Dict]) -> str:
    """Generate LabWired peripheral YAML."""
    data = {
        "name": name,
        "registers": registers,
        "side_effects": behaviors
    }
    import yaml
    return yaml.dump(data, sort_keys=False)
