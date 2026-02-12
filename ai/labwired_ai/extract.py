import pypdf
from typing import Optional, List

def parse_page_range(pages_str: Optional[str]) -> Optional[List[int]]:
    """Parse page range string (e.g., '1-3', '5,7') into 0-indexed page numbers."""
    if not pages_str:
        return None

    pages = []
    parts = pages_str.split(',')
    for part in parts:
        if '-' in part:
            start, end = map(int, part.split('-'))
            # Convert to 0-indexed, inclusive end
            pages.extend(range(start - 1, end))
        else:
            pages.append(int(part) - 1)
    return sorted(list(set(pages)))

def extract_text_from_pdf(pdf_path: str, pages_str: Optional[str] = None) -> str:
    """Read a PDF and return extracted text from specified pages."""
    reader = pypdf.PdfReader(pdf_path)
    page_indices = parse_page_range(pages_str)

    extracted_text = []

    # If no pages specified, extract all
    if page_indices is None:
        page_indices = range(len(reader.pages))

    for i in page_indices:
        if i < 0 or i >= len(reader.pages):
            continue

        page = reader.pages[i]
        text = page.extract_text()
        if text:
            extracted_text.append(f"--- Page {i+1} ---\n{text}\n")

    return "\n".join(extracted_text)
