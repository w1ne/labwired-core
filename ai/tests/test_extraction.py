import unittest
import os
from pypdf import PdfWriter
from labwired_ai.extract import extract_text_from_pdf
from labwired_ai.main import main # Just to test import, though easier to test logic directly

class TestExtraction(unittest.TestCase):
    def setUp(self):
        self.pdf_path = "test_doc.pdf"
        writer = PdfWriter()
        writer.add_blank_page(width=200, height=200)
        # Note: pypdf can't easily add text to blank page without other libs,
        # so we might rely on the fact that extract_text returns *something* or handle empty.
        # Ideally we'd use reportlab, but it's not in requirements.
        # Let's try to extract from a blank page - should be empty string or newlines.

        # Actually, let's just write the file to valid PDF structure
        with open(self.pdf_path, "wb") as f:
            writer.write(f)

    def tearDown(self):
        if os.path.exists(self.pdf_path):
            os.remove(self.pdf_path)

    def test_extract_blank(self):
        # Empty page might return empty string or \n
        text = extract_text_from_pdf(self.pdf_path)
        print(f"Extracted: {text!r}")
        self.assertIsNotNone(text)

if __name__ == "__main__":
    unittest.main()
