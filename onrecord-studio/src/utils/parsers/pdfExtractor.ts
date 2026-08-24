/**
 * Utility for sanitizing and structural formatting of raw text layer extractions from PDFs.
 */

interface CleanTextOptions {
  stripBOM?: boolean;
  normalizeWhitespace?: boolean;
  removePageHeaders?: boolean;
}

/**
 * Sanitizes raw text extracted from PDFs into a clean, searchable format.
 */
export function sanitizePdfText(rawText: string, options: CleanTextOptions = {}): string {
  if (!rawText) return "";

  let clean = rawText;

  // 1. Strip Byte Order Mark (BOM) if present
  if (options.stripBOM !== false) {
    clean = clean.replace(/^\uFEFF/, "");
  }

  // 2. Normalize line breaks and carriage returns
  clean = clean.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

  // 3. Remove typical PDF header/footer artifacts (e.g., "Page 1 of 10", "NextRequest Agency")
  if (options.removePageHeaders) {
    clean = clean.replace(/Page\s+\d+\s+of\s+\d+/gi, "");
    clean = clean.replace(/NextRequest\s+-[^\n]*/gi, "");
  }

  // 4. Collapse consecutive spaces and tabs while preserving intentional paragraphs
  if (options.normalizeWhitespace !== false) {
    clean = clean.replace(/[ \t]+/g, " ");
    clean = clean.replace(/\n{3,}/g, "\n\n"); // Cap consecutive breaks at a double paragraph
  }

  return clean.trim();
}

/**
 * Checks if a block of text extracted via PDF text-layer appears to be empty or un-extractable image data.
 * Useful for flagging documents that require downstream Local OCR parsing.
 */
export function requiresOcr(text: string, minimumCharacters: number = 10): boolean {
  if (!text) return true;
  const alphanumericCount = text.replace(/[^a-zA-Z0-9]/g, "").length;
  return alphanumericCount < minimumCharacters;
}