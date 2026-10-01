import { useMemo } from 'react';
import { useAppStore } from '../store/useAppStore';

export function useMetrics() {
  const metrics = useAppStore((state) => state.metrics);
  const activeSearchMatchesCount = useAppStore((state) => state.filteredResultsTotal);
  const runtimes = useAppStore((state) => state.portalRuntimes);

  return useMemo(() => {
    const runtime = runtimes.reduce((summary, current) => ({
      requestsProcessed: summary.requestsProcessed + current.requestsProcessed,
      documentsProcessed: summary.documentsProcessed + current.documentsProcessed,
      completedTasks: summary.completedTasks + current.completedTasks,
      failedTasks: summary.failedTasks + current.failedTasks,
      activeTasks: summary.activeTasks + current.activeTasks,
      queuedTasks: summary.queuedTasks + current.queuedTasks,
    }), { requestsProcessed: 0, documentsProcessed: 0, completedTasks: 0, failedTasks: 0, activeTasks: 0, queuedTasks: 0 });
    const finishedTasks = runtime.completedTasks + runtime.failedTasks;

    return {
      ...metrics,
      ...runtime,
      activeSearchMatchesCount,
      taskFailureRatePercentage: finishedTasks > 0
        ? Number(((runtime.failedTasks / finishedTasks) * 100).toFixed(1))
        : 0,
    };
  }, [activeSearchMatchesCount, metrics, runtimes]);
}
