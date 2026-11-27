import type React from "react";
import * as Dialog from "@radix-ui/react-dialog";
import * as Menu from "@radix-ui/react-dropdown-menu";
import * as Select from "@radix-ui/react-select";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Check, ChevronsUpDown, Columns2, FileVideo, FolderOpen, MoreHorizontal, Play, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
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

/**
 * iOS-style segmented selection. The thumb is driven by a real damped spring
 * (UIKit-like response/damping), stretches slightly with its velocity, shrinks
 * while pressed and can be dragged across segments before it snaps.
 */
const SPRING = { response: 0.26, damping: 1 };

export function PillGroup({
  className,
  axis = "y",
  activeKey,
  label,
  onPick,
  children,
}: {
  className: string;
  axis?: "x" | "y";
  activeKey: string;
  label?: string;
  onPick?: (key: string) => void;
  children: ReactNode;
}) {
  const root = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLSpanElement>(null);
  const state = useRef({ pos: 0, size: 0, vel: 0, target: 0, targetSize: 0, sizeVel: 0, ready: false, frame: 0, last: 0, pressed: false, dragging: false, drag: 0 });

  const paint = () => {
    const s = state.current;
    const el = thumb.current;
    if (!el) return;
    const stretch = 0;
    const press = s.pressed ? 0.97 : 1;
    const main = (1 + stretch) * press;
    const cross = (1 - stretch * 0.35) * press;
    const translate = axis === "x" ? `translate3d(${s.pos}px,0,0)` : `translate3d(0,${s.pos}px,0)`;
    el.style.transform = `${translate} scale(${axis === "x" ? main : cross}, ${axis === "x" ? cross : main})`;
    el.style[axis === "x" ? "width" : "height"] = `${s.size}px`;
    el.style.opacity = s.ready ? "1" : "0";
  };

  const tick = (now: number) => {
    const s = state.current;
    const dt = Math.min((now - (s.last || now)) / 1000, 1 / 30);
    s.last = now;
    const k = (2 * Math.PI / SPRING.response) ** 2;
    const c = (4 * Math.PI * SPRING.damping) / SPRING.response;
    const steps = 4;
    for (let i = 0; i < steps; i++) {
      const h = dt / steps;
      const goal = s.dragging ? s.drag : s.target;
      s.vel += (-k * (s.pos - goal) - c * s.vel) * h;
      s.pos += s.vel * h;
      s.sizeVel += (-k * (s.size - s.targetSize) - c * s.sizeVel) * h;
      s.size += s.sizeVel * h;
    }
    paint();
    const settled = !s.dragging && Math.abs(s.pos - s.target) < 0.2 && Math.abs(s.vel) < 2
      && Math.abs(s.size - s.targetSize) < 0.2 && Math.abs(s.sizeVel) < 2;
    if (settled) {
      s.pos = s.target; s.size = s.targetSize; s.vel = 0; s.sizeVel = 0; s.frame = 0; s.last = 0;
      paint();
    } else {
      s.frame = requestAnimationFrame(tick);
    }
  };
  const run = () => {
    const s = state.current;
    if (!s.frame) { s.last = 0; s.frame = requestAnimationFrame(tick); }
  };

  const segments = () => [...(root.current?.querySelectorAll<HTMLElement>(":scope > button") ?? [])];
  const boxOf = (el: HTMLElement) => axis === "x"
    ? { start: el.offsetLeft, size: el.offsetWidth }
    : { start: el.offsetTop, size: el.offsetHeight };

  useLayoutEffect(() => {
    const measure = (animate: boolean) => {
      const active = root.current?.querySelector<HTMLElement>(':scope > button[aria-pressed="true"]');
      if (!active) return;
      const { start, size } = boxOf(active);
      const s = state.current;
      s.target = start; s.targetSize = size;
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      if (!s.ready || !animate || reduce) {
        s.pos = start; s.size = size; s.vel = 0; s.sizeVel = 0; s.ready = true; paint();
      } else run();
    };
    measure(true);
    const observer = new ResizeObserver(() => measure(false));
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, [activeKey, axis]);

  useEffect(() => () => cancelAnimationFrame(state.current.frame), []);

  // Press, drag and release like UISegmentedControl.
  const onPointerDown = (event: React.PointerEvent) => {
    const button = (event.target as HTMLElement).closest("button");
    if (!button || button.disabled || event.button !== 0) return;
    const s = state.current;
    s.pressed = true; paint(); run();
    if (onPick && button.getAttribute("aria-pressed") === "true") {
      s.dragging = true; s.drag = s.pos;
      root.current?.setPointerCapture(event.pointerId);
    }
  };
  const onPointerMove = (event: React.PointerEvent) => {
    const s = state.current;
    if (!s.dragging || !root.current) return;
    const rect = root.current.getBoundingClientRect();
    const local = axis === "x" ? event.clientX - rect.left : event.clientY - rect.top;
    const limit = (axis === "x" ? root.current.clientWidth : root.current.clientHeight) - s.targetSize;
    s.drag = Math.max(0, Math.min(limit, local - s.targetSize / 2));
    run();
  };
  const release = () => {
    const s = state.current;
    const wasDragging = s.dragging;
    s.pressed = false; s.dragging = false;
    if (wasDragging && onPick) {
      const center = s.drag + s.targetSize / 2;
      let best: HTMLElement | undefined;
      let distance = Infinity;
      for (const el of segments()) {
        const { start, size } = boxOf(el);
        const d = Math.abs(start + size / 2 - center);
        if (d < distance && !(el as HTMLButtonElement).disabled) { distance = d; best = el; }
      }
      const key = best?.dataset.key;
      if (key && best?.getAttribute("aria-pressed") !== "true") onPick(key);
    }
    run();
  };

  return (
    <div
      ref={root}
      role="group"
      aria-label={label}
      className={`pill-group axis-${axis} ${className}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={release}
      onPointerCancel={release}
      onLostPointerCapture={release}
    >
      <span ref={thumb} className="pill" aria-hidden="true" />
      {children}
    </div>
  );
}
/** Apple-style pop-up button: compact trigger, floating menu with a checkmark. */
export function IosSelect({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: number;
  options: { value: number; label: string }[];
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className={disabled ? "ios-field is-disabled" : "ios-field"}>
      <span className="ios-field-label">{label}</span>
      <Select.Root value={String(value)} disabled={disabled} onValueChange={(next) => onChange(Number(next))}>
        <Select.Trigger className="ios-select" aria-label={label}>
          <Select.Value />
          <Select.Icon className="ios-select-icon"><ChevronsUpDown /></Select.Icon>
        </Select.Trigger>
        <Select.Portal>
          <Select.Content className="ios-menu" position="item-aligned">
            <Select.Viewport className="ios-menu-list">
              {options.map((option) => (
                <Select.Item key={option.value} value={String(option.value)} className="ios-menu-item">
                  <Select.ItemIndicator className="ios-menu-check"><Check /></Select.ItemIndicator>
                  <Select.ItemText>{option.label}</Select.ItemText>
                </Select.Item>
              ))}
            </Select.Viewport>
          </Select.Content>
        </Select.Portal>
      </Select.Root>
    </div>
  );
}