import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/discovery.json';
import type { AnalyticalSchema } from '../types';
import { buildDiscoveryPlan, discoveryDefaults, discoveryDrilldown, type DiscoverySelection } from '../utils/dataDiscovery';

const schema = fixture.schema as AnalyticalSchema;
const selection = fixture.selection as DiscoverySelection;
describe('guided discovery query plans', () => {
  it('keeps the queries executed in native integration tests identical to the UI builder', () => {
    expect(buildDiscoveryPlan(schema, selection).queries).toEqual(fixture.queries);
    expect(buildDiscoveryPlan(schema, fixture.calendar.selection as DiscoverySelection).queries).toEqual(fixture.calendar.queries);
  });
  it('defaults to records, preserves scope in drilldown, and never sums a numeric field implicitly', () => {
    expect(discoveryDefaults(schema, 'fixture').aggregation).toBe('count');
    const plan = buildDiscoveryPlan(schema, selection);
    expect(discoveryDrilldown(plan, "O'Brien")).toContain("'O''Brien'");
    expect(discoveryDrilldown(plan, null)).toContain('"group" IS NULL');
    expect(discoveryDrilldown(plan, null)).toContain("'adopted'");
    expect(plan.warnings.join(' ')).toContain('partial');
  });
  it('rejects stale fields and text measures without coercing suppression markers', () => {
    expect(() => buildDiscoveryPlan(schema, { ...selection, measure: 'phase' })).toThrow('numeric');
    expect(() => buildDiscoveryPlan(schema, { ...selection, dimension: 'nonexistent' })).toThrow('not in this snapshot');
    expect(() => buildDiscoveryPlan(schema, { ...selection, snapshotId: 'gone' })).toThrow('snapshot');
  });
  it('copies report scope and treats wildcard characters as literal text', () => {
    const input = { ...selection, filters: [{ column: 'phase', operator: 'contains' as const, value: "50%_O'Brien" }] };
    const plan = buildDiscoveryPlan(schema, input);
    expect(plan.recordsSql).toContain("'50%_O''Brien'");
    expect(plan.recordsSql).toContain('CONTAINS');
    input.filters[0].value = 'different';
    expect(plan.selection.filters[0].value).toBe("50%_O'Brien");
  });
});
