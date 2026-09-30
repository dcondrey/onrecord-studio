// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import {
  ConfidenceMeter,
  EdgeBasisChip,
  EvidenceTrace,
  FindingBasisChip,
  edgeVisual,
} from '../components/ui/EvidenceBasis';
import { LEGAL_CLAIM_KINDS, type FindingKind } from '../types';

afterEach(cleanup);

describe('confidence meter', () => {
  it('renders Unassessed differently from a measured zero', () => {
    const { container: unassessed } = render(<ConfidenceMeter confidence={{ kind: 'UNASSESSED' }} />);
    const unassessedHtml = unassessed.innerHTML;
    expect(unassessed.textContent).toContain('Unassessed');
    expect(unassessed.textContent).not.toMatch(/0%/);
    expect(unassessed.querySelector('[role="meter"]')).toBeNull();
    expect(unassessed.querySelector('[data-confidence="unassessed"]')).not.toBeNull();
    cleanup();

    const { container: zero } = render(<ConfidenceMeter confidence={{ kind: 'MEASURED', basisPoints: 0 }} />);
    expect(zero.textContent).toContain('0% measured');
    expect(zero.textContent).not.toContain('Unassessed');
    expect(screen.getByRole('meter').getAttribute('aria-valuenow')).toBe('0');
    expect(zero.innerHTML).not.toBe(unassessedHtml);
  });
});

describe('finding basis chip', () => {
  it('never renders a legal-claim kind as a pattern signal or the reverse', () => {
    const kinds: FindingKind[] = ['POLICY_VIOLATION_DETECTED', 'HELD_BACK_RESPONSE', 'CROSS_CORPUS_TEMPORAL_SHIFT'];
    for (const kind of kinds) {
      const { container } = render(<FindingBasisChip kind={kind} />);
      const legal = LEGAL_CLAIM_KINDS.includes(kind);
      expect(container.querySelector('[data-basis="legal-claim"]') !== null).toBe(legal);
      expect(container.querySelector('[data-basis="pattern-signal"]') !== null).toBe(!legal);
      expect(/counted violation/i.test(container.textContent ?? '')).toBe(legal);
      cleanup();
    }
  });
});

describe('edge basis', () => {
  it('never draws co-occurrence like a typed edge, whatever its basis', () => {
    for (const basis of ['OBSERVED', 'COMPUTED', 'ASSERTED'] as const) {
      const proximity = edgeVisual('CO_OCCURRENCE', basis);
      const typed = edgeVisual('SIGNED_BY', basis);
      expect(proximity.family).toBe('proximity');
      expect(proximity.dash).toBeDefined();
      expect(proximity.markerShape).not.toBe(typed.markerShape);
      expect(proximity.opacity).toBeLessThan(typed.opacity);
    }
    const { container } = render(
      <>
        <EdgeBasisChip kind="CO_OCCURRENCE" basis="OBSERVED" />
        <EdgeBasisChip kind="SIGNED_BY" basis="OBSERVED" />
      </>,
    );
    expect(container.querySelectorAll('[data-edge-family="proximity"]')).toHaveLength(1);
    expect(container.querySelectorAll('[data-edge-family="typed"]')).toHaveLength(1);
    expect(container.querySelector('[data-edge-family="proximity"]')?.textContent).toMatch(/proximity only/);
  });
});

describe('evidence trace', () => {
  it('opens by keyboard without hover, lists the quote, and closes on Escape', () => {
    const onOpenSource = vi.fn();
    const citation = { portalKey: 'p', documentId: 7, characterStart: 3, characterEnd: 9, quote: 'the exact words', label: null };
    render(<EvidenceTrace label="x" explanation="because" citations={[citation]} onOpenSource={onOpenSource} />);
    const trigger = screen.getByRole('button', { name: /why we believe this/i });
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(trigger);
    expect(screen.getByText('the exact words')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /open source document 7/i }));
    expect(onOpenSource).toHaveBeenCalledWith(citation);
    fireEvent.keyDown(trigger, { key: 'Escape' });
    expect(screen.queryByText('the exact words')).toBeNull();
  });
});
