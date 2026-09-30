// @vitest-environment jsdom
import { afterEach, describe, expect, test, vi } from 'vitest';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { MatchStatusNotice } from '../components/account/ProfileSettings';
import { SelfReviewPanel, groupOwnFindings } from '../components/account/SelfReview';
import { touchesOwnOrganization, useProfile } from '../components/account/useProfile';
import * as ipc from '../ipc';
import type { AffiliationDisclosure, AffiliationMatchList, Finding, ProfileState } from '../types';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const emptyMatches: AffiliationMatchList = {
  status: 'UNSET',
  candidates: [],
  searchedPortalCount: 0,
  searchedAgencyCount: 0,
  searchedEntityCount: 0,
  limitations: [],
};

function stateNamed(name: string): ProfileState {
  return {
    status: 'SET',
    profile: {
      schemaVersion: 1,
      displayName: name,
      affiliations: [],
      matchDecisions: [],
      verifiedDomains: [],
      updatedAt: '2026-09-30T00:00:00Z',
    },
  };
}

function Harness({ expose }: { expose: (refresh: () => void) => void }) {
  const profile = useProfile();
  expose(profile.refresh);
  return <div data-testid="name">{profile.state?.profile?.displayName ?? 'none'}</div>;
}

describe('useProfile', () => {
  test('a slower superseded profile read never overwrites the newer one', async () => {
    const resolvers: Array<(state: ProfileState) => void> = [];
    vi.spyOn(ipc, 'invokeCommand').mockImplementation(((command: string) => {
      if (command === 'get_profile') {
        return new Promise<ProfileState>((resolve) => resolvers.push(resolve));
      }
      return Promise.resolve(emptyMatches);
    }) as unknown as typeof ipc.invokeCommand);
    let refresh: () => void = () => undefined;
    render(<Harness expose={(fn) => { refresh = fn; }} />);
    await waitFor(() => expect(resolvers.length).toBe(1));
    act(() => refresh());
    await waitFor(() => expect(resolvers.length).toBe(2));
    await act(async () => {
      resolvers[1](stateNamed('newer'));
    });
    await waitFor(() => expect(screen.getByTestId('name').textContent).toBe('newer'));
    await act(async () => {
      resolvers[0](stateNamed('older'));
    });
    expect(screen.getByTestId('name').textContent).toBe('newer');
  });
});

describe('match status', () => {
  test('no match yet never says unaffiliated or no records', () => {
    render(
      <MatchStatusNotice
        list={{ ...emptyMatches, status: 'NO_MATCH_YET', searchedPortalCount: 3, searchedEntityCount: 40 }}
      />,
    );
    const text = screen.getByRole('status').textContent ?? '';
    expect(text).toMatch(/searched/i);
    expect(text).toMatch(/nothing resembled/i);
    expect(text).not.toMatch(/unaffiliated/i);
    expect(text).not.toMatch(/no records/i);
  });
});

const finding = (id: string, kind: Finding['kind'], portalKey: string | null): Finding => ({
  id,
  kind,
  severity: 'NOTABLE',
  confidence: { kind: 'UNASSESSED' },
  subject: { portalKey, agencyId: null, requestId: null, documentId: null, entityId: null },
  headline: `headline ${id}`,
  explanation: 'x',
  citations: [],
  discoveredAt: '2026-09-01T00:00:00Z',
  lastSeenAt: '2026-09-01T00:00:00Z',
  dismissedAt: null,
  sourceDetector: 't',
});

function disclosureWith(overrides: Partial<AffiliationDisclosure>): AffiliationDisclosure {
  return { status: 'UNSET', sentence: null, affiliations: [], ownPortalKeys: [], ownAgencies: [], ownEntityIds: [], ...overrides };
}

describe('organization lens', () => {
  test('touchesOwnOrganization matches only confirmed subjects', () => {
    const disclosure = disclosureWith({ status: 'DECLARED', ownPortalKeys: ['mine'] });
    expect(touchesOwnOrganization(finding('a', 'ABSENCE_OF_RECORDS', 'mine'), disclosure)).toBe(true);
    expect(touchesOwnOrganization(finding('b', 'ABSENCE_OF_RECORDS', 'other'), disclosure)).toBe(false);
    expect(touchesOwnOrganization(finding('c', 'ABSENCE_OF_RECORDS', 'mine'), null)).toBe(false);
  });

  test('groups findings into the records-hygiene categories', () => {
    const groups = groupOwnFindings([
      finding('a', 'CONTRADICTORY_STATEMENT', 'mine'),
      finding('b', 'EXTENSION_ABUSE', 'mine'),
    ]);
    expect(groups.find((g) => g.title.startsWith('Contradictions'))?.findings).toHaveLength(1);
    expect(groups.find((g) => g.title.startsWith('Late'))?.findings).toHaveLength(1);
    expect(groups.find((g) => g.title.startsWith('Missing'))?.findings).toHaveLength(0);
  });

  test('self-review with an unset profile says so instead of listing nothing', async () => {
    vi.spyOn(ipc, 'invokeCommand').mockImplementation(((command: string) => {
      if (command === 'get_affiliation_disclosure') return Promise.resolve(disclosureWith({}));
      return Promise.resolve([]);
    }) as unknown as typeof ipc.invokeCommand);
    render(<SelfReviewPanel onClose={() => undefined} />);
    expect(await screen.findByText(/no profile is set/i)).toBeTruthy();
  });
});
