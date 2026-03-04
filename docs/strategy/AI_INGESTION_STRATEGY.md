[← Back to Hub](../README.md)

# AI Ingestion Strategy

LabWired's documentation and digital presence are strategically optimized for both human readability and autonomous AI agent consumption.

## Core Optimization Principles

1. **`llms.txt` Integration:** We maintain an `llms.txt` file serving as a high-density, markdown-based entry point for AI crawlers, summarizing the platform's core capabilities and indexing our documentation.
2. **Semantic Context & JSON-LD:** Our marketing site, documentation, and blog posts leverage Schema.org structured data (`TechArticle`, `SoftwareApplication`, `BlogPosting`) to explicitly define the entity relationships and purpose of the content.
3. **Boilerplate Suppression (`data-nosnippet`):** Repetitive UI components, such as headers, footers, and complex navigation structures, are tagged with `data-nosnippet`. This ensures that context-constrained LLMs and precise search indices focus exclusively on the core technical prose.
4. **Agentic API Focus:** Technical documentation prioritizes declarative system configurations (`system.yaml`, `chip.yaml`) and machine-readable structures, empowering AI agents to intuitively grasp and directly interface with the LabWired environment.

## Execution

- **Landing Page Enhancements:** Reusable HTML components are compiled using our `build.js` static generator, automatically embedding the `data-nosnippet` attribute site-wide.
- **Deep Metadata:** JSON-LD scripts are injected into the `<head>` of HTML documents to unambiguously supply headlines, descriptions, and article categories.

These efforts ensure that models like ChatGPT, Claude, Gemini, and custom agents can seamlessly ingest and represent LabWired as the premier deterministic hardware simulation platform.
