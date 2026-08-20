import { Component, type ErrorInfo, type ReactNode } from "react";
import { logToFile } from "../../lib/log";

type Props = { children: ReactNode };
type State = { message: string | null };

/**
 * Catches a render that throws, so the window shows something and the log says what.
 *
 * A throw during render unmounts the whole tree, which leaves a blank window and no record of
 * why. React only reports these to a class component, which is why this one is not a hook.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { message: null };

  static getDerivedStateFromError(error: unknown): State {
    return {
      message: error instanceof Error ? error.message : String(error),
    };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    logToFile("ERROR", "ui.render_failed", `${error.name}: ${error.message}`, {
      stack: error.stack,
      // Which components were mounting when it threw — usually the fastest way to the cause.
      componentStack: info.componentStack,
    });
  }

  render(): ReactNode {
    if (this.state.message === null) {
      return this.props.children;
    }

    return (
      <section className="banner banner-error">
        Something in the interface stopped working, and the details are in the
        log. Restart Wonder of U to carry on. {this.state.message}
      </section>
    );
  }
}
