import type { WhisperAssetUpdateResult } from "../../types";

export function UpdateResultCard({
  result,
}: {
  result: WhisperAssetUpdateResult | null;
}) {
  if (!result) {
    return null;
  }

  return (
    <div className={`update-card ${result.status}`}>
      <strong>{result.message}</strong>
      {result.currentVersion || result.latestVersion ? (
        <p className="microcopy">
          Current: {result.currentVersion ?? "Unknown"}{" "}
          {result.latestVersion ? `| Latest: ${result.latestVersion}` : ""}
        </p>
      ) : null}
    </div>
  );
}
