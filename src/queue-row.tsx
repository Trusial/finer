import { Check, CircleAlert, CircleSlash, FileVideo, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";
import { fileLabel, fpsLabel, frameSizeLabel, sizeLabel, timeLabel } from "./format";
import type { Job } from "./queue";
import { Hint } from "./ui";

export function QueueRow({ item, children }: { item: Job; children: ReactNode }) {
  const encoding = item.status === "encoding";
  const progress = encoding ? item.progress : undefined;
  const name = item.output?.name ?? item.media?.name ?? fileLabel(item.path);
  const metadata = item.media
    ? [frameSizeLabel(item.frameSize ?? item.media), fpsLabel(item.media.fps), timeLabel(item.media.duration)]
    : [];
  if (item.output) metadata.push(sizeLabel(item.output.size));
  const checking = item.status === "pending" || item.status === "checking";
  const status = item.status === "pending" ? "Waiting to check file" : "Checking file";
  const phase = item.finalizing ? "Finishing" : "Converting";
  const percent = item.finalizing ? 100 : Math.floor(progress?.percent ?? 0);

  const audio = item.media;
  let notice: string | undefined;
  if (item.status === "alreadyPatched") notice = "Already patched";
  else if (item.output && item.output.audioPatch !== "applied") notice = "Audio patch skipped";
  else if (!item.output && audio && !item.issue) {
    if (!audio.audioCodec) notice = "No audio to patch";
    else if (audio.sampleRate !== 48000 || audio.channelLayout !== "stereo") notice = "Audio patch needs 48 kHz stereo";
  }

  return (
    <li className="queue-row">
      <div className="queue-item">
        <div className="file-summary">
          <span className="file-icon">
            <FileVideo />
          </span>
          <div className="file-copy">
            <div className="file-title">
              <strong className="truncate" title={item.output?.path ?? item.path}>
                {name}
              </strong>
              {(checking || encoding) && (
                <span className="file-status" role="status" aria-label={checking ? status : phase}>
                  <LoaderCircle className="spinner" aria-hidden="true" />
                </span>
              )}
              {(item.status === "ready" || item.status === "complete") && (
                <Hint label={item.status === "ready" ? "Ready" : "Exported"}>
                  <span
                    className="file-status"
                    tabIndex={0}
                    role="img"
                    aria-label={item.status === "ready" ? "Ready" : "Exported"}
                  >
                    <Check />
                  </span>
                </Hint>
              )}
              {item.status === "cancelled" && (
                <Hint label="Cancelled">
                  <span className="file-status" tabIndex={0} role="img" aria-label="Cancelled">
                    <CircleSlash />
                  </span>
                </Hint>
              )}
              {item.status === "failed" && (
                <Hint label="Failed">
                  <span className="file-status is-error" tabIndex={0} role="img" aria-label="Failed">
                    <CircleAlert />
                  </span>
                </Hint>
              )}
            </div>
            {metadata.length > 0 ? (
              <div className="file-meta">
                {metadata.map((value) => (
                  <span key={value}>{value}</span>
                ))}
              </div>
            ) : (
              checking && (
                <div className="metadata-skeleton" aria-hidden="true">
                  <i />
                  <i />
                  <i />
                </div>
              )
            )}
            {item.status === "needsResolution" && (
              <Hint label="Frame size needs a choice">
                <span className="file-status" tabIndex={0} role="img" aria-label="Frame size needs a choice">
                  <CircleAlert />
                </span>
              </Hint>
            )}
          </div>
        </div>
        <div className="row-actions">{children}</div>
      </div>
      {notice && <p className="row-issue">{notice}</p>}
      {item.issue && <p className="row-issue">{item.issue.message}</p>}
      {encoding && (
        <div className="row-progress">
          <div className="progress-timing">
            <span>{phase}</span>
            {(progress || item.finalizing) && <strong>{percent}%</strong>}
          </div>
          <progress
            max={100}
            value={progress || item.finalizing ? percent : undefined}
            aria-label="File progress"
          />
        </div>
      )}
    </li>
  );
}
