import type {
  SetupChecklistStep,
  SetupChecklistSummary,
} from "../../lib/navigation";
import type { AppPage, SettingsSection } from "../../types";

type StepMarker = "done" | "todo" | "optional";

function stepMarker(step: SetupChecklistStep): StepMarker {
  if (step.done === true) {
    return "done";
  }
  return step.required ? "todo" : "optional";
}

const STATUS_LABEL: Record<StepMarker, string> = {
  done: "Done",
  todo: "To do",
  optional: "Open",
};

const STATUS_TONE: Record<StepMarker, "success" | "accent" | "neutral"> = {
  done: "success",
  todo: "accent",
  optional: "neutral",
};

export function SetupChecklist({
  steps,
  summary,
  onOpenSection,
  onNavigate,
}: {
  steps: SetupChecklistStep[];
  summary: SetupChecklistSummary;
  onOpenSection: (section: SettingsSection) => void;
  onNavigate: (page: AppPage) => void;
}) {
  const requiredSteps = steps.filter((step) => step.required);
  const optionalSteps = steps.filter((step) => !step.required);
  const progressPercent =
    summary.total > 0 ? Math.round((summary.done / summary.total) * 100) : 0;
  // The next action to draw the eye to: the first required step still to do.
  const nextStepId = requiredSteps.find((step) => step.done !== true)?.id ?? null;

  const renderStep = (step: SetupChecklistStep) => {
    const marker = stepMarker(step);
    const detail = step.value ?? step.description;
    const isNext = step.id === nextStepId;
    return (
      <li key={step.id}>
        <button
          type="button"
          className={`setup-step ${isNext ? "setup-step-next" : ""}`}
          onClick={() => onOpenSection(step.target)}
        >
          <span className={`setup-step-marker ${marker}`} aria-hidden="true">
            {marker === "done" ? "✓" : ""}
          </span>
          <span className="setup-step-body">
            <span className="setup-step-label">{step.label}</span>
            <small className={step.value ? "setup-step-value" : undefined}>
              {detail}
            </small>
          </span>
          <span className={`status-chip status-chip-${STATUS_TONE[marker]}`}>
            {STATUS_LABEL[marker]}
          </span>
        </button>
      </li>
    );
  };

  return (
    <div className="settings-scroll">
      <article className="panel settings-card setup-checklist">
        <header className="panel-header">
          <div>
            <p className="panel-kicker">Setup</p>
            <h2>Guided setup</h2>
          </div>
          <span className={`setup-progress ${summary.allDone ? "complete" : ""}`}>
            {summary.allDone ? "All set" : `${summary.done} of ${summary.total}`}
          </span>
        </header>

        <div
          className="progress-track"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={summary.total}
          aria-valuenow={summary.done}
          aria-label="Required setup progress"
        >
          <div className="progress-fill" style={{ width: `${progressPercent}%` }} />
        </div>

        <p className="microcopy">
          {summary.allDone
            ? "Everything's ready — revisit any step below whenever you need."
            : "Finish these steps to start transcribing and pushing cards to Anki."}
        </p>

        {summary.allDone ? (
          <div className="setup-complete-row">
            <span className="setup-step-marker done" aria-hidden="true">
              ✓
            </span>
            <p className="setup-complete-text">
              You're ready to transcribe and push cards
            </p>
            <button type="button" onClick={() => onNavigate("home")}>
              Go to Home
            </button>
          </div>
        ) : null}

        <ul className="setup-steps">
          {requiredSteps.map(renderStep)}
          {optionalSteps.length > 0 ? (
            <li className="setup-group-label" aria-hidden="true">
              Optional
            </li>
          ) : null}
          {optionalSteps.map(renderStep)}
        </ul>
      </article>
    </div>
  );
}
