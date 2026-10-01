// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ErrorBoundary } from '../components/ui/ErrorBoundary';

function Boom({ fail }: { fail: boolean }) {
  if (fail) throw new Error('render exploded');
  return <p>healthy view</p>;
}

describe('ErrorBoundary', () => {
  it('contains a render failure and recovers when the reset key changes', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const { rerender } = render(
      <ErrorBoundary label="Findings" resetKey="a"><Boom fail /></ErrorBoundary>,
    );
    expect(screen.getByRole('alert').textContent).toContain('render exploded');
    rerender(<ErrorBoundary label="Findings" resetKey="b"><Boom fail={false} /></ErrorBoundary>);
    expect(screen.getByText('healthy view')).toBeTruthy();
  });
});
