/**
 * A department's statistical tier, from its closure-time z-score alone. A statutory count is a
 * different kind of claim and never moves a department between these tiers; charts mark it
 * separately. Every chart, legend and headline that names a tier reads it from here, so a count
 * in a headline always agrees with the marks under it.
 */
export type DeviationTier = 'typical' | 'elevated' | 'outlier';

export function deviationTier(z: number): DeviationTier {
  if (z >= 2) return 'outlier';
  if (z >= 1) return 'elevated';
  return 'typical';
}

export const TIER_LABEL: Record<DeviationTier, string> = { typical: 'Typical', elevated: 'Elevated (1–2σ)', outlier: 'Outlier (≥2σ)' };

export function deviationWords(z: number): string {
  const size = Math.abs(z).toFixed(1);
  return z >= 0 ? `${size}σ slower than peers` : `${size}σ faster than peers`;
}

export function tierColor(tier: DeviationTier): string {
  return tier === 'outlier' ? 'var(--viz-red)' : tier === 'elevated' ? 'var(--viz-amber)' : 'var(--viz-cyan)';
}
