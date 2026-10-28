import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Button, EmptyState } from '@ui';
import { isChunkLoadError } from './staleBuild';

/**
 * Catches a render error in what it wraps (PDT-209): the view shows "Something went wrong" with Reload and Back to
 * Inbox, and the error goes to the console, instead of the whole app unmounting to a blank page. A new `resetKey`
 * (the location) tries the content again.
 */
export class ErrorBoundary extends Component<
  { children: ReactNode; resetKey?: unknown; onBack: () => void; className?: string },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error : new Error(String(error)) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error('A view failed to render:', error, info.componentStack);
  }

  componentDidUpdate(prev: { resetKey?: unknown }) {
    if (this.state.error && prev.resetKey !== this.props.resetKey) this.setState({ error: null });
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    const back = () => {
      this.props.onBack();
      this.setState({ error: null });
    };
    return (
      <EmptyState
        role="alert"
        className={this.props.className}
        action={
          <div className="flex gap-2">
            <Button className="h-12 px-6 pointer-coarse:h-12" onClick={back}>Back to Inbox</Button>
            <Button variant="primary" className="h-12 px-6 pointer-coarse:h-12" onClick={() => window.location.reload()}>Reload</Button>
          </div>
        }
      >
        <span className="text-base font-medium text-foreground">Something went wrong</span>
        <br />
        {isChunkLoadError(error) ? 'A newer version is out since this page loaded: reload to get it.' : error.message || 'This view failed to show.'}
      </EmptyState>
    );
  }
}
