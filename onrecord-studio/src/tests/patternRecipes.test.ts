import { describe, expect, it } from 'vitest';
import {
  loadPatternRecipes,
  parsePatternRecipeTerms,
  patternRecipeFingerprint,
  persistPatternRecipes,
  type PatternRecipe,
} from '../utils/patternRecipes';

const recipe: PatternRecipe = {
  id: 'recipe-1',
  name: 'Shared dates',
  kindFilter: 'TEMPORAL_CLUSTER',
  minimumEvidenceCount: 3,
  minimumConfidence: 9_000,
  indexedFrom: '',
  indexedTo: '',
  requiredTerms: 'deadline, response',
  createdAt: '2026-08-28T12:00:00.000Z',
};

describe('pattern recipes', () => {
  it('normalizes bounded required terms into a stable preview fingerprint', () => {
    expect(parsePatternRecipeTerms(' deadline, response, deadline, x ')).toEqual([
      'deadline',
      'response',
    ]);
    expect(patternRecipeFingerprint({
      kindFilter: recipe.kindFilter,
      minimumEvidenceCount: recipe.minimumEvidenceCount,
      minimumConfidence: recipe.minimumConfidence,
      indexedFrom: recipe.indexedFrom,
      indexedTo: recipe.indexedTo,
      requiredTerms: recipe.requiredTerms,
    })).toContain('["deadline","response"]');
  });

  it('round-trips valid recipes and rejects corrupt storage records', () => {
    let serialized = '';
    persistPatternRecipes({ setItem: (_key, value) => { serialized = value; } }, [recipe]);
    expect(loadPatternRecipes({ getItem: () => serialized })).toEqual([recipe]);
    expect(loadPatternRecipes({ getItem: () => '{broken' })).toEqual([]);
    expect(loadPatternRecipes({ getItem: () => '[{"id":"partial"}]' })).toEqual([]);
  });
});
