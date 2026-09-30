// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { LEGAL_VERIFICATION_STATES, PLAIN_FINDING_TEXT, PLAIN_VERIFICATION_TEXT, type PlainText } from '../content/plainLanguage';
import { PlainLanguageDisclosure } from '../components/ui/EvidenceBasis';
import { isLegalClaimKind, type FindingKind } from '../types/findings';
import type { VerificationState } from '../types/audit';

afterEach(cleanup);

const words = (entry: PlainText) => `${entry.observed} ${entry.notShown} ${entry.next}`;

// No plain text may accuse, whatever the kind.
const ACCUSATION = /illegal|unlawful|guilty|fraud|accus|misconduct|corrupt|wrongdo|cover-?up|conceal|knowingly|intentionally|deliberately/i;
// A pattern, statistical or procedural label must not borrow the vocabulary of a legal conclusion.
const LEGAL_WORDING = /violat|\bbroke|breach|non-?complian|not in compliance|in contempt|unauthori[sz]ed/i;

const kinds = Object.keys(PLAIN_FINDING_TEXT) as FindingKind[];
const states = Object.keys(PLAIN_VERIFICATION_TEXT) as VerificationState[];

describe('plain-language text', () => {
  it('has all three statements, non-empty, for every kind and state', () => {
    for (const entry of [...Object.values(PLAIN_FINDING_TEXT), ...Object.values(PLAIN_VERIFICATION_TEXT)]) {
      for (const field of ['observed', 'notShown', 'next'] as const) expect(entry[field].trim().length).toBeGreaterThan(10);
    }
  });

  it('never accuses anyone', () => {
    for (const kind of kinds) expect(words(PLAIN_FINDING_TEXT[kind]), kind).not.toMatch(ACCUSATION);
    for (const state of states) expect(words(PLAIN_VERIFICATION_TEXT[state]), state).not.toMatch(ACCUSATION);
  });

  it('uses no legal wording for a kind that is not a legal claim', () => {
    const patternKinds = kinds.filter((kind) => !isLegalClaimKind(kind));
    expect(patternKinds.length).toBeGreaterThan(0);
    for (const kind of patternKinds) expect(words(PLAIN_FINDING_TEXT[kind]), kind).not.toMatch(LEGAL_WORDING);
  });

  it('uses no legal wording for a verification state that is not a counted comparison', () => {
    for (const state of states.filter((value) => !LEGAL_VERIFICATION_STATES.includes(value))) {
      expect(words(PLAIN_VERIFICATION_TEXT[state]), state).not.toMatch(LEGAL_WORDING);
    }
  });

  it('says a legal-claim kind is not a court finding, and a latency outlier is a peer comparison', () => {
    expect(PLAIN_FINDING_TEXT.POLICY_VIOLATION_DETECTED.notShown).toMatch(/not a court finding/);
    expect(PLAIN_VERIFICATION_TEXT.StatutoryDeadlineExceeded.notShown).toMatch(/not a court finding/);
    expect(PLAIN_VERIFICATION_TEXT.LatencyOutlier.notShown).toMatch(/comparison with peers/);
    expect(PLAIN_VERIFICATION_TEXT.LatencyOutlier.notShown).toMatch(/No statute is involved/);
    expect(PLAIN_VERIFICATION_TEXT.Unaudited.notShown).toMatch(/not a result/);
  });
});

describe('PlainLanguageDisclosure', () => {
  it('is collapsed, labelled, and operable', () => {
    render(<PlainLanguageDisclosure kind="BOILERPLATE_DENIAL" label="Shared denial" />);
    const toggle = screen.getByRole('button', { name: 'Plain-language explanation: Shared denial' });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText(PLAIN_FINDING_TEXT.BOILERPLATE_DENIAL.observed)).toBeNull();
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(toggle.getAttribute('aria-controls')).toBe(screen.getByRole('group').id);
    expect(screen.getByText(PLAIN_FINDING_TEXT.BOILERPLATE_DENIAL.notShown)).toBeTruthy();
  });

  it('takes its basis sentence from the kind, never from the text', () => {
    render(<PlainLanguageDisclosure kind="HELD_BACK_RESPONSE" label="Pattern" />);
    fireEvent.click(screen.getByRole('button'));
    expect(screen.getByText(/a pattern signal, not a legal claim/)).toBeTruthy();
    cleanup();
    render(<PlainLanguageDisclosure kind="CONTRACT_CEILING_EXCEEDED" label="Counted" />);
    fireEvent.click(screen.getByRole('button'));
    expect(screen.getByText(/counted comparison against a stated rule/)).toBeTruthy();
    expect(screen.queryByText(/pattern signal, not a legal claim/)).toBeNull();
  });
});
