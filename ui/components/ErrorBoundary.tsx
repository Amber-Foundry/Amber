import { Component, type ErrorInfo, type ReactNode } from "react";

type ErrorBoundaryProps = {
  children: ReactNode;
};

type ErrorBoundaryState = {
  error: Error | null;
};

class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("Amber crashed:", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="error-boundary">
          <pre className="error-boundary-message">{this.state.error.message}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}

export default ErrorBoundary;
