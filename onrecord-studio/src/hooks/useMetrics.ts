import { useMemo } from 'react';
import { useAppStore } from '../store/useAppStore';

export function useMetrics() {
  // TODO(metrics): Replace the fixed 50-record page estimate with telemetry
  // from the actual page-size and source pagination metadata.
  const { metrics, filteredResults } = useAppStore();

  // PERF(metrics): Prefer narrow Zustand selectors if this hook is mounted in
  // high-frequency event views; the current selector observes the whole store.
  return useMemo(() => {
    const totalProcessedAssets = metrics.pagesProcessed * 50; // Based on standard page size bounds
    const discoveryConversionRatio = totalProcessedAssets > 0 
      ? parseFloat(((metrics.documentsDownloaded / totalProcessedAssets) * 100).toFixed(1))
      : 0;

    const systemicAnomalyRiskScore = metrics.documentsDownloaded > 0
      ? parseFloat(((metrics.anomaliesFound / metrics.documentsDownloaded) * 100).toFixed(1))
      : 0;

    return {
      // Raw state vectors
      ...metrics,
      activeSearchMatchesCount: filteredResults.length,
      
      // Derived statistical trends
      estimatedTotalAssetsEvaluated: totalProcessedAssets,
      attachmentExtractionRatePercentage: discoveryConversionRatio,
      calculatedRiskScorePercentage: systemicAnomalyRiskScore,
      
      // Qualitative triage flags
      requiresPriorityIntervention: systemicAnomalyRiskScore > 15.0 || metrics.anomaliesFound > 20
    };
  }, [metrics, filteredResults]);
}
