import * as Dialog from "@radix-ui/react-dialog";
import * as Menu from "@radix-ui/react-dropdown-menu";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Check, Columns2, FileVideo, FolderOpen, MoreHorizontal, Play, X } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { fileLabel, remainingLabel } from "./format";
import type { Job } from "./queue";
import { Button } from "./ui";

/** First frame of a video through the asset protocol; falls back to an icon. */
export function Thumb({ path, className = "thumb" }: { path: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [path]);
  if (failed) {
    return (
      <span className={`${className} is-fallback`}>
        <FileVideo />
      </span>
    );
  }
  return (
    <video
      className={className}
      src={`${convertFileSrc(path)}#t=0.5`}
      preload="metadata"
      muted
      playsInline
      aria-hidden="true"
      onError={() => setFailed(true)}
    />
  );
}

type Step = { label: string; state: "done" | "active" | "todo" };

/** Large ring for the file currently encoding, with its pipeline steps. */
export function ExportProgress({ job, timeLeft }: { job: Job; timeLeft: number | null }) {
  const progress = job.progress;
  const percent = job.finalizing ? 100 : Math.floor(progress?.percent ?? 0);
  const passes = progress?.passes ?? 2;
  const pass = progress?.pass ?? 1;
  const steps: Step[] = [{ label: "Analysis", state: "done" }];
  if (passes === 2) {
    steps.push({ label: "Pass 1 · Encoding", state: job.finalizing || pass > 1 ? "done" : "active" });
    steps.push({ label: "Pass 2 · Encoding", state: job.finalizing ? "done" : pass === 2 ? "active" : "todo" });
  } else {
    steps.push({ label: "Encoding", state: job.finalizing ? "done" : "active" });
  }
  steps.push({ label: "Audio patch & check", state: job.finalizing ? "active" : "todo" });
  const radius = 52;
  const circumference = 2 * Math.PI * radius;

  return (
    <section className="export-panel" aria-label="Export progress">
      <div className="ring" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
        <svg viewBox="0 0 120 120" aria-hidden="true">
          <circle className="ring-track" cx="60" cy="60" r={radius} />
          <circle
            className="ring-value"
            cx="60"
            cy="60"
            r={radius}
            strokeDasharray={circumference}
            strokeDashoffset={circumference * (1 - percent / 100)}
          />
        </svg>
        <div className="ring-label">
          <strong>{percent}%</strong>
          <small>{timeLeft !== null ? `${remainingLabel(timeLeft)} left` : job.finalizing ? "Finishing" : "Estimating"}</small>
        </div>
      </div>
      <div className="export-copy">
        <span className="export-kicker">Refining</span>
        <h2 className="truncate" title={job.path}>{job.media?.name ?? fileLabel(job.path)}</h2>
        <ol className="step-list">
          {steps.map((step) => (
            <li key={step.label} className={`step is-${step.state}`}>
              <span className="step-dot">{step.state === "done" && <Check />}</span>
              {step.label}
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

/** Celebratory card after the latest export, with the main follow-up actions. */
export function DoneCard({
  job,
  onPlay,
  onReveal,
  onCompare,
  onDismiss,
}: {
  job: Job;
  onPlay: () => void;
  onReveal: () => void;
  onCompare: () => void;
  onDismiss: () => void;
}) {
  const output = job.output!;
  return (
    <section className="done-card" aria-live="polite">
      <Thumb path={output.path} className="done-thumb" />
      <div className="done-copy">
        <span className="done-badge">
          <Check /> Export finished
        </span>
        <strong className="truncate" title={output.path}>{output.name}</strong>
        <div className="done-actions">
          <Button variant="primary" onClick={onPlay}><Play />Play</Button>
          <Button onClick={onReveal}><FolderOpen />Show in folder</Button>
          <Button variant="ghost" onClick={onCompare}><Columns2 />Compare</Button>
        </div>
      </div>
      <Button variant="icon" className="done-close" aria-label="Dismiss" onClick={onDismiss}><X /></Button>
    </section>
  );
}

/** Before/after split view of source and export, kept in sync. */
export function CompareDialog({ job, onClose }: { job: Job; onClose: () => void }) {
  const [split, setSplit] = useState(50);
  const before = useRef<HTMLVideoElement>(null);
  const after = useRef<HTMLVideoElement>(null);
  const output = job.output!;

  function sync() {
    if (before.current && after.current && Math.abs(before.current.currentTime - after.current.currentTime) > 0.05) {
      before.current.currentTime = after.current.currentTime;
    }
  }
  function toggle() {
    const playing = after.current && !after.current.paused;
    for (const video of [before.current, after.current]) {
      if (!video) continue;
      if (playing) video.pause();
      else void video.play().catch(() => {});
    }
  }

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content className="modal compare-modal">
          <div className="modal-header">
            <Dialog.Title>Before / after</Dialog.Title>
            <Dialog.Close asChild>
              <Button variant="icon" aria-label="Close"><X /></Button>
            </Dialog.Close>
          </div>
          <Dialog.Description className="sr-only">Drag the slider to compare the source with the export.</Dialog.Description>
          <div className="compare-stage" style={{ "--split": `${split}%` } as React.CSSProperties}>
            <video ref={after} src={convertFileSrc(output.path)} muted loop playsInline onTimeUpdate={sync} onClick={toggle} />
            <video ref={before} className="compare-before" src={convertFileSrc(job.path)} muted loop playsInline onClick={toggle} />
            <span className="compare-tag is-before">Before</span>
            <span className="compare-tag is-after">Finer</span>
            <span className="compare-handle" aria-hidden="true" />
            <input
              className="compare-range"
              type="range"
              min={0}
              max={100}
              value={split}
              aria-label="Comparison position"
              onChange={(event) => setSplit(Number(event.target.value))}
            />
          </div>
          <div className="modal-footer">
            <span className="compare-hint">Click the video to play or pause</span>
            <Button variant="primary" onClick={toggle}><Play />Play / pause</Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export type RowAction = { label: string; icon: ReactNode; onSelect: () => void; danger?: boolean; disabled?: boolean };

/** Secondary row actions behind a single "…" button. */
export function RowMenu({ label, actions }: { label: string; actions: RowAction[] }) {
  if (!actions.length) return null;
  return (
    <Menu.Root>
      <Menu.Trigger asChild>
        <Button variant="icon" aria-label={label}><MoreHorizontal /></Button>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content className="row-menu" align="end" sideOffset={6}>
          {actions.map((action) => (
            <Menu.Item
              key={action.label}
              className={action.danger ? "row-menu-item is-danger" : "row-menu-item"}
              disabled={action.disabled}
              onSelect={action.onSelect}
            >
              {action.icon}
              {action.label}
            </Menu.Item>
          ))}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  );
}
