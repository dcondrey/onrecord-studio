import { describe, expect, it } from 'vitest';
import { normalisedName, parseOrganisationList } from '../utils/organisationNames';

describe('organisation lists', () => {
  it('keeps commas inside names, splits on lines, and deduplicates by normalised form', () => {
    const { eins, names } = parseOrganisationList('33-0215585\nAlpha Project for the Homeless, Inc.\nALPHA PROJECT FOR THE HOMELESS INC\n330215585; Father Joe\'s Villages\n\n');
    expect(eins).toEqual(['330215585']);
    expect(names).toEqual(['Alpha Project for the Homeless, Inc.', "Father Joe's Villages"]);
  });

  it('compares names on letters and digits only', () => {
    expect(normalisedName('St. Vincent de Paul Village, Inc.')).toBe(normalisedName('ST VINCENT DE PAUL VILLAGE INC'));
    expect(normalisedName('Alpha Project For The Homeless')).not.toBe(normalisedName('Alpha Project for the Homeless, Inc.'));
  });
});
