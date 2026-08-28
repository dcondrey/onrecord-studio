import type { CorpusPatternKind } from '../types';

export const PATTERN_RECIPE_STORAGE_KEY = 'onrecord:corpus-pattern-recipes:v1';

export type PatternKindFilter = CorpusPatternKind | 'ALL';

export interface PatternRecipe {
  id: string;
  name: string;
  kindFilter: PatternKindFilter;
  minimumEvidenceCount: number;
  minimumConfidence: number;
  indexedFrom: string;
  indexedTo: string;
  requiredTerms: string;
  createdAt: string;
}

const patternKinds = new Set<PatternKindFilter>([
  'ALL',
  'EXACT_DUPLICATE',
  'NEAR_DUPLICATE',
  'SHARED_STATUTE',
  'SHARED_ENTITY',
  'SHARED_TERM',
  'TEMPORAL_CLUSTER',
  'ENTITY_RELATIONSHIP',
]);

export function parsePatternRecipeTerms(value: string): string[] {
  return [...new Set(
    value
      .split(',')
      .map((term) => term.trim())
      .filter((term) => term.length >= 2 && term.length <= 64),
  )].slice(0, 12);
}

export function patternRecipeFingerprint(recipe: Omit<PatternRecipe, 'id' | 'name' | 'createdAt'>): string {
  return JSON.stringify({
    ...recipe,
    requiredTerms: parsePatternRecipeTerms(recipe.requiredTerms),
  });
}

export function loadPatternRecipes(storage: Pick<Storage, 'getItem'>): PatternRecipe[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(PATTERN_RECIPE_STORAGE_KEY) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isPatternRecipe).slice(0, 50);
  } catch {
    return [];
  }
}

export function persistPatternRecipes(
  storage: Pick<Storage, 'setItem'>,
  recipes: PatternRecipe[],
): void {
  storage.setItem(PATTERN_RECIPE_STORAGE_KEY, JSON.stringify(recipes.slice(0, 50)));
}

function isPatternRecipe(value: unknown): value is PatternRecipe {
  if (!value || typeof value !== 'object') return false;
  const recipe = value as Partial<PatternRecipe>;
  return typeof recipe.id === 'string'
    && recipe.id.length > 0
    && typeof recipe.name === 'string'
    && recipe.name.trim().length > 0
    && recipe.name.length <= 80
    && typeof recipe.kindFilter === 'string'
    && patternKinds.has(recipe.kindFilter as PatternKindFilter)
    && [2, 3, 5, 10].includes(recipe.minimumEvidenceCount ?? 0)
    && [0, 7_000, 8_000, 9_000, 9_500].includes(recipe.minimumConfidence ?? -1)
    && typeof recipe.indexedFrom === 'string'
    && typeof recipe.indexedTo === 'string'
    && typeof recipe.requiredTerms === 'string'
    && parsePatternRecipeTerms(recipe.requiredTerms).length <= 12
    && typeof recipe.createdAt === 'string'
    && !Number.isNaN(Date.parse(recipe.createdAt));
}
