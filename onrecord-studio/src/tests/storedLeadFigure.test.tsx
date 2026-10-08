// @vitest-environment jsdom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { invokeCommand } from '../ipc';
import StoredLeadFigure from '../components/dashboard/StoredLeadFigure';
import type { Finding } from '../types';

vi.mock('../ipc', () => ({ invokeCommand: vi.fn(), formatNativeError: String }));
vi.mock('../components/charts/figures/LeadFigure', () => ({
  default: ({ figure }: { figure: { title: string } }) => <div data-testid="lead-figure">{figure.title}</div>,
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const finding = (id: string, figure: unknown): Finding => ({ id, explanation: `${id} prose`, figure } as unknown as Finding);

describe('StoredLeadFigure', () => {
  it('never draws a figure read for a lead the row no longer shows', async () => {
    const pending = new Map<string, (rows: Finding[]) => void>();
    vi.mocked(invokeCommand).mockImplementation(((_command: string, args: { filter: { id: string } }) =>
      new Promise((resolve) => pending.set(args.filter.id, resolve))) as unknown as typeof invokeCommand);
    const { rerender } = render(<StoredLeadFigure findingId="lead-a" fallback={<p>fallback</p>} />);
    rerender(<StoredLeadFigure findingId="lead-b" fallback={<p>fallback</p>} />);

    await act(async () => { pending.get('lead-b')?.([finding('lead-b', null)]); });
    await act(async () => { pending.get('lead-a')?.([finding('lead-a', { title: 'Figure of lead A' })]); });

    expect(screen.queryByTestId('lead-figure')).toBeNull();
    expect(screen.getByText('fallback')).toBeTruthy();
  });

  it('leads with a stored figure and keeps the prose as text', async () => {
    vi.mocked(invokeCommand).mockResolvedValue([finding('lead-a', { title: 'Figure of lead A' })] as never);
    render(<StoredLeadFigure findingId="lead-a" fallback={<p>fallback</p>} reserve />);
    await act(async () => {});

    expect(screen.getByTestId('lead-figure').textContent).toBe('Figure of lead A');
    expect(screen.getByText('Figures as text')).toBeTruthy();
    expect(screen.queryByText('fallback')).toBeNull();
  });
});
