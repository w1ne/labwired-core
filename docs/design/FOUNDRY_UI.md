# LabWired Foundry: Design System

The LabWired Foundry UI follows a **high-contrast, bento-box inspired aesthetic** designed to align perfectly with the core LabWired brand while prioritizing clarity and speed for AI-agent interactions.

## Core Philosophy: "Cyber-Physical Utility"
The UI intentionally avoids excessive decoration in favor of bold, functional elements that mimic high-end engineering documentation and laboratory equipment interfaces.

## 🎨 Color Palette

| Token | Hex | Role |
| :--- | :--- | :--- |
| **White** | `#FFFFFF` | Primary background, card foundation |
| **Off-White** | `#F8F9FA` | Page background, input fields |
| **Black** | `#000000` | Headings, borders, primary buttons |
| **Pink** | `#E83E8C` | Brand accent, highlight, "active" states |
| **Gray** | `#444444` | Secondary text, inactive states |
| **Green** | `#27C93F` | Success states, "Solid Proven" badge |

## Typography
- **Headings**: `Outfit` (Bold/900). A geometric sans-serif that feels modern and precise.
- **Body**: `Inter`. Optimized for readability in high-density data views.
- **Code/Data**: `JetBrains Mono`. Used for prompts, register values, and status logs.

## 🧱 UI Components

### 1. Bento Cards
The fundamental building block. Every section is encapsulated in a card with:
- `2px solid #000` border.
- `4px 4px 0px #000` solid shadow (simulating physical depth).
- `12px` corner radius.

### 2. Status Badges
High-contrast pills with bold typography and specific semantic coloring (e.g., Pink for "Running", Green for "Proven").

### 3. Action Buttons
Solid black backgrounds with white text and a translation effect (`-2px -2px`) on hover to simulate physical tactile feedback.

---
> **Guideline**: When in doubt, make it bolder. Use negative space and borders rather than gradients or soft shadows.
