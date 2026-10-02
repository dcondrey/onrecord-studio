import { Component, type ErrorInfo, type ReactNode } from 'react';

interface ErrorBoundaryProps {
  children: ReactNode;
  /** Names the failed surface in the fallback, e.g. "Findings". */
  label: string;
  /** Changing this value clears a prior failure, so navigating away recovers the shell. */
  resetKey?: string;
  /** Full-window fallback for the root boundary; otherwise fills the content area. */
  fullScreen?: boolean;
}

interface ErrorBoundaryState {
  error: Error | null;
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[${this.props.label}] render failure`, error, info.componentStack);
  }

  componentDidUpdate(previous: ErrorBoundaryProps) {
    if (this.state.error && previous.resetKey !== this.props.resetKey) {
      this.setState({ error: null });
    }
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div
        role="alert"
        className={`flex flex-1 items-center justify-center p-8 ${this.props.fullScreen ? 'h-screen w-screen app-theme' : 'min-h-0'}`}
      >
        <div className="app-panel max-w-lg p-6">
          <h2 className="text-sm font-semibold text-fg">{this.props.label} hit an unexpected error</h2>
          <p className="mt-2 text-label leading-relaxed text-muted">
            Your stored records and evidence are unaffected. The rest of the app keeps working;
            this view failed to draw.
          </p>
          <p className="mt-3 break-words rounded border border-line bg-canvas p-2 font-mono text-label text-muted">
            {error.message || String(error)}
          </p>
          <div className="mt-4 flex gap-2">
            <button
              type="button"
              onClick={() => this.setState({ error: null })}
              className="rounded border border-highlight/30 bg-highlight/10 px-3 py-1.5 text-label leading-4 font-semibold text-highlight hover:bg-highlight/20"
            >
              Try again
            </button>
            {this.props.fullScreen && (
              <button
                type="button"
                onClick={() => window.location.reload()}
                className="rounded border border-control px-3 py-1.5 text-label leading-4 font-semibold text-fg-secondary hover:bg-raised/70"
              >
                Reload app
              </button>
            )}
          </div>
        </div>
      </div>
    );
  }
}
