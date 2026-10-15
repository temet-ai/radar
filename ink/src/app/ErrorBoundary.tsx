import React from "react";

/**
 * Any render error ends the renderer non-zero after the terminal is restored, so the host's
 * supervisor restarts it (up to three times a minute).
 */
export class ErrorBoundary extends React.Component<
  { onError: (error: Error) => void; children?: React.ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error) {
    this.props.onError(error);
  }

  render() {
    return this.state.failed ? null : this.props.children;
  }
}
