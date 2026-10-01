import { describe, expect, it } from 'vitest';
import fixture from '../utils/nameKeyFixture.json';
import { nameKey } from '../utils/entityEdges';

// The same fixture is read by the Rust test `name_key_matches_the_typescript_fixture`, so the
// candidate blocking in resolution/mod.rs cannot drift from the graph's lookalike view.
describe('nameKey parity fixture', () => {
  it('produces the recorded key for every input', () => {
    expect(fixture.length).toBeGreaterThanOrEqual(12);
    for (const { input, key } of fixture) expect(nameKey(input), JSON.stringify(input)).toBe(key);
  });
});
