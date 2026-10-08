import type { GlyphShape } from '../ui/shapeGlyph';
import type { FindingSeverity } from '../../types';

export function severityGlyph(severity: FindingSeverity): [GlyphShape, string] {
  switch (severity) {
    case 'CRITICAL':
      return ['diamond', 'text-danger'];
    case 'SERIOUS':
      return ['triangle', 'text-warning'];
    case 'NOTABLE':
      return ['circle', 'text-accent'];
    case 'INFO':
      return ['ring', 'text-muted'];
  }
}
