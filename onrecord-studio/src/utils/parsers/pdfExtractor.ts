interface CleanTextOptions {
  stripBOM?: boolean;
  normalizeWhitespace?: boolean;
  removePageHeaders?: boolean;
}

export function sanitizePdfText(rawText: string, options: CleanTextOptions = {}): string {
  if (rawText.length === 0) return '';
  const withoutBom = options.stripBOM === false || rawText.charCodeAt(0) !== 0xfeff
    ? rawText
    : rawText.slice(1);
  const normalizedLines = normalizeLineEndings(withoutBom).split('\n');
  const output: string[] = [];
  let precedingLineWasBlank = false;
  for (const sourceLine of normalizedLines) {
    const line = options.normalizeWhitespace === false
      ? sourceLine.trimEnd()
      : collapseHorizontalWhitespace(sourceLine).trim();
    if (options.removePageHeaders && isPageCounter(line)) continue;
    if (line.length === 0) {
      if (!precedingLineWasBlank && output.length > 0) output.push('');
      precedingLineWasBlank = true;
    } else {
      output.push(line);
      precedingLineWasBlank = false;
    }
  }
  while (output.at(-1) === '') output.pop();
  return output.join('\n').trim();
}

export function requiresOcr(text: string, minimumCharacters = 10): boolean {
  if (!Number.isSafeInteger(minimumCharacters) || minimumCharacters < 1) {
    throw new RangeError('minimumCharacters must be a positive safe integer');
  }
  // This is a cheap text-layer signal, not a legibility assessment. Keep the native density and
  // image checks authoritative when this helper is used outside frontend previews/tests.
  let alphanumericCount = 0;
  for (const character of text) {
    if (isUnicodeLetterOrNumber(character)) {
      alphanumericCount += 1;
      if (alphanumericCount >= minimumCharacters) return false;
    }
  }
  return true;
}

function normalizeLineEndings(value: string) {
  let output = '';
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (character !== '\r') {
      output += character;
      continue;
    }
    if (value[index + 1] === '\n') index += 1;
    output += '\n';
  }
  return output;
}

function collapseHorizontalWhitespace(value: string) {
  let output = '';
  let pendingSpace = false;
  for (const character of value) {
    if (character === ' ' || character === '\t' || character === '\u00a0') {
      pendingSpace = output.length > 0;
    } else {
      if (pendingSpace) output += ' ';
      output += character;
      pendingSpace = false;
    }
  }
  return output;
}

function isPageCounter(value: string) {
  const tokens = value.toLocaleLowerCase().split(' ').filter(Boolean);
  return tokens.length === 4
    && tokens[0] === 'page'
    && isAsciiDigits(tokens[1])
    && tokens[2] === 'of'
    && isAsciiDigits(tokens[3]);
}

function isAsciiDigits(value: string) {
  return value.length > 0 && Array.from(value).every((character) => {
    const code = character.charCodeAt(0);
    return code >= 48 && code <= 57;
  });
}

function isUnicodeLetterOrNumber(character: string) {
  const folded = character.toLocaleLowerCase();
  return folded !== folded.toLocaleUpperCase() || isAsciiDigits(character);
}
