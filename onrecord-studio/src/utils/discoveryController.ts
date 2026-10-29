import type { AnalyticalImportCandidate, AnalyticalSchema, CorpusPattern, DataMaintenanceStatus, ServiceLeadInput, SqlQueryResult } from '../types';
import { automaticFollowups, automaticProfileQuery, snapshotFingerprint, storedLeads, type AutomaticProfile } from './automaticDiscovery';
import { dataGaps, type DataGap } from './dataGaps';

type FetchRequest = NonNullable<DataGap['fetch']>;
/** Same format as `firstRunTableKey`. */
const fetchKey = (request: FetchRequest) => `${request.dataset}:${request.year ?? 'all'}`;

export interface DiscoveryTransport {
  schema(): Promise<AnalyticalSchema>;
  candidates(): Promise<AnalyticalImportCandidate[]>;
  patterns(): Promise<CorpusPattern[]>;
  query(sql: string): Promise<SqlQueryResult>;
  maintain?(): Promise<DataMaintenanceStatus>;
  fetch?(request: NonNullable<DataGap['fetch']>): Promise<unknown>;
  /** Starts collecting from a records portal for a subject; resolves once the collection has started. */
  collect?(request: NonNullable<DataGap['collect']>): Promise<unknown>;
  /** Stores the feed's leads as findings; `complete` permits dropping earlier leads not sent. */
  record?(leads: ServiceLeadInput[], complete: boolean): Promise<unknown>;
}
export interface AutomaticDiscoveryState {
  schema: AnalyticalSchema | null;
  candidates: AnalyticalImportCandidate[];
  patterns: CorpusPattern[];
  profiles: Record<string, AutomaticProfile>;
  errors: Record<string, string>;
  activeSnapshot: string | null;
  checkedAt: string | null;
  maintenance: DataMaintenanceStatus | null;
  /** Per gap key: `fetching`, `fetched`, or the error of the last attempt. */
  gapStatus: Record<string, string>;
  paused: boolean;
  inspection: { sql: string; sequence: number } | null;
}

/** One bounded query at a time, immutable-snapshot caching, and independent failure domains.
 * Lifecycle tokens prevent StrictMode/unmount races from publishing stale results.
 * A failed check backs off for five minutes; it cannot starve other snapshots.
 */
export class DiscoveryController {
  state: AutomaticDiscoveryState = { schema: null, candidates: [], patterns: [], profiles: {}, errors: {}, activeSnapshot: null, checkedAt: null, maintenance: null, gapStatus: {}, paused: false, inspection: null };
  private listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private generation = 0;
  private active = false;
  private busy = false;
  private forceRefresh = true;
  private refreshedAt = 0;
  private maintainedAt = 0;
  private holds = 0;
  private reserved = new Map<string, number>();
  private failures = new Map<string, number>();
  private secondaryInFlight = new Set<'candidates' | 'patterns'>();
  private gapRetryAt = new Map<string, number>();
  private requestedGaps = new Set<string>();
  private recordedLeads = '';
  private recordRetryAt = 0;
  private pending: { snapshotId: string; fingerprint: string; queries: ReturnType<typeof automaticFollowups> } | null = null;

  constructor(private transport: DiscoveryTransport, private visible: () => boolean = () => true, private now: () => number = Date.now) { this.maintainedAt = this.now() - 55_000; }
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  snapshot = () => this.state;
  private publish(patch: Partial<AutomaticDiscoveryState>) { this.state = { ...this.state, ...patch }; this.listeners.forEach((listener) => listener()); }
  start() { if (this.active) return; this.forceRefresh = true; this.active = true; this.generation += 1; this.schedule(0); }
  stop() {
    this.active = false; this.generation += 1; clearTimeout(this.timer);
    if (this.pending) {
      const id = this.pending.snapshotId;
      this.pending = null;
      this.publish({ profiles: Object.fromEntries(Object.entries(this.state.profiles).filter(([key]) => key !== id)), activeSnapshot: null });
    }
  }
  refresh = () => { this.forceRefresh = true; this.schedule(0); };
  setMaintenance = (maintenance: DataMaintenanceStatus) => { this.publish({ maintenance }); this.maintainedAt = this.now() - 55_000; this.schedule(0); };
  /** Fetch one missing table now, whether or not automatic upkeep is on. */
  fill = (key: string) => { this.requestedGaps.add(key); this.schedule(0); };
  gaps = (): DataGap[] => this.state.schema ? dataGaps(this.state.schema, this.state.profiles, new Date(this.now()).toISOString()) : [];
  /** Missing tables are fetched one at a time, automatically only under the owner's upkeep policy.
   * A collection from a records portal downloads many files, so it starts only when asked for. */
  private nextGap(): DataGap | undefined {
    const policy = this.state.maintenance?.policy;
    const automatic = Boolean(policy?.enabled && policy.autoRefresh);
    return this.gaps().find((gap) => (gap.fetch ? this.transport.fetch : this.transport.collect)
      && !['fetching', 'fetched'].includes(this.state.gapStatus[gap.key])
      && !(gap.fetch && this.reserved.has(fetchKey(gap.fetch)))
      && (this.requestedGaps.has(gap.key) || (gap.fetch && automatic && this.now() >= (this.gapRetryAt.get(gap.key) ?? 0))));
  }
  inspect = (sql: string) => this.publish({ inspection: { sql, sequence: (this.state.inspection?.sequence ?? 0) + 1 } });
  hold = () => {
    this.holds += 1; this.publish({ paused: true });
    let released = false;
    return () => { if (released) return; released = true; this.holds = Math.max(0, this.holds - 1); this.publish({ paused: this.holds > 0 }); this.schedule(0); };
  };
  /** Keeps gap fetches of these tables to the caller, which is fetching them itself; everything else runs. */
  reserve = (fetches: FetchRequest[]) => {
    const keys = fetches.map(fetchKey);
    for (const key of keys) this.reserved.set(key, (this.reserved.get(key) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return; released = true;
      for (const key of keys) {
        const count = (this.reserved.get(key) ?? 0) - 1;
        if (count > 0) this.reserved.set(key, count); else this.reserved.delete(key);
      }
      this.schedule(0);
    };
  };
  private schedule(delay: number) {
    clearTimeout(this.timer);
    if (this.active && !this.busy) this.timer = setTimeout(() => void this.tick(), delay);
  }
  private valid(generation: number) { return this.active && this.generation === generation; }

  private refreshSecondary<K extends 'candidates' | 'patterns'>(key: K, fetch: () => Promise<AutomaticDiscoveryState[K]>) {
    if (this.secondaryInFlight.has(key)) return;
    this.secondaryInFlight.add(key);
    const generation = this.generation;
    void fetch().then((value) => {
      if (!this.valid(generation)) return;
      if (!Array.isArray(value)) throw new Error(`Invalid ${key} response`);
      const errors = { ...this.state.errors }; delete errors[key];
      this.publish({ [key]: value, errors });
    }).catch((reason) => {
      if (this.valid(generation)) this.publish({ errors: { ...this.state.errors, [key]: String(reason) } });
    }).finally(() => {
      this.secondaryInFlight.delete(key);
      if (this.active && !this.valid(generation)) this.refresh();
    });
  }

  async tick() {
    if (!this.active || this.busy) return;
    const maintenanceDue = this.transport.maintain && this.now() - this.maintainedAt >= 60_000;
    if (this.holds || (!this.visible() && !maintenanceDue)) { this.schedule(1000); return; }
    const generation = this.generation;
    this.busy = true;
    let nextDelay = 250;
    try {
      if (this.visible() && (this.forceRefresh || this.now() - this.refreshedAt >= 30_000)) {
        this.forceRefresh = false;
        // Document-pattern indexing can be slow; it must not delay table observations.
        // Each independent stream has one in-flight request, including across teardown/restart.
        this.refreshSecondary('candidates', this.transport.candidates);
        this.refreshSecondary('patterns', this.transport.patterns);
        const outcome = await this.transport.schema().then((value) => ({ value, error: null }), (reason) => ({ value: null, error: String(reason) }));
        if (!this.valid(generation)) return;
        const errors = { ...this.state.errors };
        const patch: Partial<AutomaticDiscoveryState> = {};
        const schema = outcome.value;
        if (!schema || !Array.isArray(schema.datasets) || !Array.isArray(schema.columns)) {
          errors.schema = outcome.error ?? 'The analytical catalog did not return a usable schema.';
        } else {
          delete errors.schema;
          patch.schema = schema;
          patch.profiles = Object.fromEntries(Object.entries(this.state.profiles).filter(([id, profile]) => {
            const dataset = schema.datasets.find((item) => item.snapshotId === id);
            return dataset && profile.fingerprint === snapshotFingerprint(schema, dataset);
          }));
          if (this.pending && !schema.datasets.some((item) => item.snapshotId === this.pending?.snapshotId && snapshotFingerprint(schema, item) === this.pending?.fingerprint)) this.pending = null;
          for (const key of Object.keys(errors)) if (key.startsWith('profile:') && !schema.datasets.some((item) => key === `profile:${item.snapshotId}`)) delete errors[key];
        }
        this.refreshedAt = this.now();
        this.publish({ ...patch, errors, checkedAt: new Date(this.now()).toISOString() });
        return;
      }
      // Each cycle performs at most one import, one compaction, and one source refresh.
      if (this.transport.maintain && this.now() - this.maintainedAt >= 60_000) {
        this.maintainedAt = this.now();
        try {
          const maintenance = await this.transport.maintain();
          if (!maintenance?.policy || !Array.isArray(maintenance.events)) throw new Error('Invalid maintenance status');
          if (this.valid(generation)) {
            const previous = this.state.maintenance;
            const errors = { ...this.state.errors }; delete errors.maintenance;
            this.publish({ maintenance, errors }); this.forceRefresh = true;
            if (previous && (previous.events[0]?.recordedAt !== maintenance.events[0]?.recordedAt || previous.referencedDocuments !== maintenance.referencedDocuments)) this.maintainedAt = this.now() - 55_000;
          }
        } catch (reason) {
          if (this.valid(generation)) this.publish({ errors: { ...this.state.errors, maintenance: String(reason) } });
        }
        return;
      }
      if (!this.visible()) { nextDelay = 1000; return; }
      const schema = this.state.schema;
      if (!schema || this.state.errors.schema) { nextDelay = 30_000; return; }
      if (!this.pending) {
        for (const [key, retryAt] of this.failures) if (this.now() >= retryAt) {
          this.failures.delete(key);
          this.publish({ profiles: Object.fromEntries(Object.entries(this.state.profiles).filter(([, profile]) => profile.fingerprint !== key)) });
        }
        const stored = storedLeads(schema, this.state.profiles);
        const leadsKey = JSON.stringify(stored);
        if (this.transport.record && leadsKey !== this.recordedLeads && (stored.complete || stored.leads.length) && this.now() >= this.recordRetryAt) {
          // Keyed by what was sent, so it is noted even if the controller stopped meanwhile.
          const outcome = await this.transport.record(stored.leads, stored.complete).then(() => null, (reason) => String(reason));
          if (outcome === null) this.recordedLeads = leadsKey;
          else this.recordRetryAt = this.now() + 300_000;
          if (this.valid(generation)) {
            const errors = { ...this.state.errors }; delete errors.leads;
            this.publish({ errors: outcome === null ? errors : { ...errors, leads: outcome } });
          }
          return;
        }
        const dataset = schema.datasets.find((item) => !this.state.profiles[item.snapshotId] && this.now() >= (this.failures.get(snapshotFingerprint(schema, item)) ?? 0));
        if (!dataset) {
          this.publish({ activeSnapshot: null });
          const gap = this.nextGap();
          if (!gap) { nextDelay = 5000; return; }
          this.requestedGaps.delete(gap.key);
          this.publish({ gapStatus: { ...this.state.gapStatus, [gap.key]: 'fetching' } });
          // Recorded even if the controller stopped meanwhile: the outcome is keyed by its own gap,
          // and a restart that forgot it would download the same file twice.
          const outcome = await (gap.fetch ? this.transport.fetch!(gap.fetch) : this.transport.collect!(gap.collect!)).then(() => 'fetched', (reason) => String(reason));
          if (outcome !== 'fetched') this.gapRetryAt.set(gap.key, this.now() + 21_600_000);
          else this.forceRefresh = true;
          this.publish({ gapStatus: { ...this.state.gapStatus, [gap.key]: outcome } });
          return;
        }
        const fingerprint = snapshotFingerprint(schema, dataset);
        let query;
        try {
          query = automaticProfileQuery(schema, dataset);
          if (!query?.sql) throw new Error('No usable columns to profile');
          const errors = { ...this.state.errors }; delete errors[`profile:${dataset.snapshotId}`];
          this.publish({ errors });
        } catch (reason) {
          this.failures.set(fingerprint, this.now() + 300_000);
          this.publish({ errors: { ...this.state.errors, [`profile:${dataset.snapshotId}`]: `${dataset.displayName}: ${String(reason)}` } });
          return;
        }
        this.pending = { snapshotId: dataset.snapshotId, fingerprint, queries: [query] };
        this.publish({ activeSnapshot: dataset.snapshotId, profiles: { ...this.state.profiles, [dataset.snapshotId]: { fingerprint, checkedAt: new Date(this.now()).toISOString(), results: [] } } });
      }
      const job = this.pending;
      const query = job.queries.shift();
      if (!query) { this.pending = null; return; }
      const dataset = schema.datasets.find((item) => item.snapshotId === job.snapshotId)!;
      try {
        const result = await this.transport.query(query.sql);
        if (!this.valid(generation)) return;
        if (!result || !Array.isArray(result.rows) || !Array.isArray(result.columns)) throw new Error('The check returned no usable result.');
        const profile = this.state.profiles[job.snapshotId];
        if (!profile || profile.fingerprint !== job.fingerprint) return;
        this.publish({ profiles: { ...this.state.profiles, [job.snapshotId]: { ...profile, results: [...profile.results, { query, result }] } } });
        if (query.id === 'quality') job.queries.push(...automaticFollowups(schema, dataset, result));
      } catch (reason) {
        if (!this.valid(generation)) return;
        this.failures.set(job.fingerprint, this.now() + 300_000);
        const profile = this.state.profiles[job.snapshotId];
        if (profile) this.publish({ profiles: { ...this.state.profiles, [job.snapshotId]: { ...profile, results: [...profile.results, { query, error: String(reason) }] } } });
      }
      if (!job.queries.length) { this.pending = null; this.publish({ activeSnapshot: null }); }
    } catch (reason) {
      if (this.valid(generation)) this.publish({ errors: { ...this.state.errors, discovery: String(reason) } });
      nextDelay = 5000;
    } finally {
      this.busy = false;
      this.schedule(this.forceRefresh && this.visible() ? 0 : nextDelay);
    }
  }
}
