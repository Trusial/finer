import { useRef, useState } from "react";
import {
  desktop,
  failure,
  type Failure,
  type FrameSize,
  type Media,
  type Output,
  type Progress,
  type Settings,
  type Update,
} from "./native";
import { QueuePace } from "./queue-progress";
import { readQueue, writeQueue } from "./saved-queue";

type JobStatus =
  | "pending" | "checking" | "needsResolution" | "ready" | "encoding"
  | "complete" | "failed" | "cancelled" | "alreadyPatched";

export type Job = {
  id: string;
  path: string;
  status: JobStatus;
  media?: Media;
  frameSize?: FrameSize | null;
  output?: Output;
  issue?: Failure;
  progress?: Progress;
  finalizing?: boolean;
};

export function isFinished({ status }: Pick<Job, "status">) {
  return status === "complete" || status === "failed" || status === "cancelled";
}

function canConvert({ status }: Job) {
  return status === "pending" || status === "checking" || status === "ready";
}

export function needsFrameSize(job: Job) {
  return job.status === "needsResolution" && !!job.media?.suggestedFrameSize;
}

export function matchesFrameSize(first: Job, second: Job) {
  const a = first.media;
  const b = second.media;
  return a && b && needsFrameSize(second) && a.width === b.width && a.height === b.height
    && a.suggestedFrameSize?.width === b.suggestedFrameSize?.width
    && a.suggestedFrameSize?.height === b.suggestedFrameSize?.height;
}

export type Notify = (
  message: string,
  kind?: "info" | "success" | "error",
  action?: { label: string; run: () => void },
) => void;

export type JobLog = { lines: string[]; firstLine: number };

const maxLogLines = 500;

export function useQueue(notify: Notify, openLogs: (id: string) => void) {
  const [jobs, setJobs] = useState<Job[]>(() =>
    readQueue().map((item, index) => ({ ...item, id: String(index + 1), status: "pending" })),
  );
  const [running, setRunning] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  // Native callbacks and the conversion loop read the latest queue between renders.
  const jobsRef = useRef(jobs);
  const active = useRef<{ id: string; attempt: string } | null>(null);
  const batch = useRef<{ onlyJobId?: string; settled: number; pace: QueuePace } | null>(null);
  const runningRef = useRef(false);
  const stopRequested = useRef(false);
  const inspectionTask = useRef<Promise<void> | null>(null);
  const nextId = useRef(jobs.length);
  const savedSnapshot = useRef("");
  const saveErrorShown = useRef(false);
  const logs = useRef(new Map<string, JobLog>());

  function replaceJobs(update: (previous: Job[]) => Job[]) {
    jobsRef.current = update(jobsRef.current);
    setJobs(jobsRef.current);
    const snapshot = JSON.stringify(jobsRef.current
      .filter((job) => !isFinished(job))
      .map(({ path, frameSize }) => ({ path, frameSize })));
    if (snapshot === savedSnapshot.current) return;
    try {
      writeQueue(snapshot);
      savedSnapshot.current = snapshot;
      saveErrorShown.current = false;
    } catch {
      if (!saveErrorShown.current) notify("Could not save the queue for next time.", "error");
      saveErrorShown.current = true;
    }
  }

  function patch(id: string, changes: Partial<Job>) {
    const currentBatch = batch.current;
    if (
      currentBatch &&
      (!currentBatch.onlyJobId || currentBatch.onlyJobId === id) &&
      changes.status && isFinished({ status: changes.status }) &&
      jobsRef.current.some((job) => job.id === id && !isFinished(job))
    ) {
      currentBatch.settled++;
    }
    replaceJobs((previous) => previous.map((job) => (job.id === id ? { ...job, ...changes } : job)));
  }

  function appendLog(id: string, line: string) {
    const log = logs.current.get(id) ?? { lines: [], firstLine: 1 };
    log.lines.push(...line.split(/\r\n|\r|\n/));
    const overflow = log.lines.length - maxLogLines;
    if (overflow > 0) {
      log.lines.splice(0, overflow);
      log.firstLine += overflow;
    }
    logs.current.set(id, log);
  }

  function markFailed(job: Job, issue: Failure) {
    appendLog(job.id, [issue.message, issue.detail].filter(Boolean).join("\n"));
    patch(job.id, { status: "failed", issue });
    notify(issue.message, "error", {
      label: "Logs",
      run: () => {
        if (jobsRef.current.some(({ id }) => id === job.id)) openLogs(job.id);
      },
    });
  }

  async function inspect(job: Job) {
    patch(job.id, { status: "checking", issue: undefined });
    try {
      const media = await desktop.inspect(job.path);
      patch(job.id, {
        media,
        status: media.alreadyPatched ? "alreadyPatched"
          : media.suggestedFrameSize && job.frameSize === undefined ? "needsResolution"
          : "ready",
      });
    } catch (error) {
      if (jobsRef.current.some((candidate) => candidate.id === job.id)) markFailed(job, failure(error));
    }
  }

  function inspectPending() {
    if (runningRef.current || inspectionTask.current) return;
    inspectionTask.current = (async () => {
      while (!runningRef.current) {
        const next = jobsRef.current.find((job) => job.status === "pending");
        if (!next) break;
        await inspect(next);
      }
    })().finally(() => {
      inspectionTask.current = null;
      if (jobsRef.current.some((job) => job.status === "pending")) inspectPending();
    });
  }

  function add(paths: string[]) {
    const queuedPaths = new Set(
      jobsRef.current.filter((job) => job.status !== "complete").map((job) => job.path.toLowerCase()),
    );
    const newJobs: Job[] = [];
    for (const path of paths) {
      const key = path.toLowerCase();
      if (queuedPaths.has(key)) continue;
      queuedPaths.add(key);
      newJobs.push({ id: String(++nextId.current), path, status: "pending" });
    }
    if (!newJobs.length) {
      if (paths.length) notify("Already in the queue.");
      return;
    }
    replaceJobs((previous) => [...previous, ...newJobs]);
    inspectPending();
  }

  function forgetJobs(ids: string[]) {
    if (!ids.length) return;
    for (const id of ids) logs.current.delete(id);
    void desktop.discardCheckpoints(ids).catch((error: unknown) => notify(failure(error).message, "error"));
  }

  function remove(id: string) {
    if (active.current?.id === id) return;
    replaceJobs((jobs) => jobs.filter((job) => job.id !== id));
    forgetJobs([id]);
  }

  function clearFinished() {
    forgetJobs(jobsRef.current.filter(isFinished).map(({ id }) => id));
    replaceJobs((jobs) => jobs.filter((job) => !isFinished(job)));
  }

  function retry(id: string) {
    const job = jobsRef.current.find((job) => job.id === id);
    if (!job) return;
    logs.current.delete(id);
    patch(id, {
      issue: undefined,
      progress: undefined,
      finalizing: false,
      status: !job.media ? "pending"
        : job.media.suggestedFrameSize && job.frameSize === undefined ? "needsResolution"
        : "ready",
    });
    inspectPending();
  }

  function chooseFrameSize(id: string, frameSize: FrameSize | null, applyToMatching: boolean) {
    const job = jobsRef.current.find((candidate) => candidate.id === id);
    if (!job || !needsFrameSize(job)) return;
    replaceJobs((previous) => previous.map((candidate) => {
      if (candidate.id === id || (applyToMatching && matchesFrameSize(job, candidate))) {
        return { ...candidate, frameSize, status: "ready" };
      }
      return candidate;
    }));
  }

  function recheck() {
    forgetJobs(jobsRef.current.filter((job) => job.status !== "complete").map(({ id }) => id));
    replaceJobs((previous) => previous.map((job) => job.status === "complete" ? job : {
      id: String(++nextId.current),
      path: job.path,
      frameSize: job.frameSize,
      status: "pending",
    }));
    inspectPending();
  }

  async function start(settings: Settings, onlyJobId?: string) {
    const matchesRequestedJob = (job: Job) => !onlyJobId || job.id === onlyJobId;
    if (
      runningRef.current ||
      !jobsRef.current.some(
        (job) => matchesRequestedJob(job) && canConvert(job),
      )
    ) {
      return;
    }
    runningRef.current = true;
    stopRequested.current = false;
    batch.current = { onlyJobId, settled: 0, pace: new QueuePace() };
    setRunning(true);
    let completed = 0;
    let lastOutput: Output | undefined;

    try {
      await inspectionTask.current;
      while (!stopRequested.current) {
        const job = jobsRef.current.find(
          (candidate) =>
            matchesRequestedJob(candidate) &&
            (candidate.status === "ready" || candidate.status === "pending"),
        );
        if (!job) break;
        if (job.status === "pending") {
          await inspect(job);
          continue;
        }

        const attempt = String(++nextId.current);
        active.current = { id: job.id, attempt };
        logs.current.delete(job.id);
        patch(job.id, { status: "encoding", issue: undefined, finalizing: false, progress: undefined });
        try {
          const output = await desktop.convert(attempt, job.id, job.path, job.frameSize ?? null);
          patch(job.id, { output, status: "complete" });
          lastOutput = output;
          completed++;
        } catch (error) {
          const issue = failure(error);
          if (issue.code === "cancelled") {
            patch(job.id, { status: "cancelled" });
            if (!stopRequested.current) notify("Conversion cancelled.");
          } else if (issue.code === "alreadyPatched") {
            if (job.media) {
              patch(job.id, { status: "alreadyPatched", media: { ...job.media, alreadyPatched: true } });
            }
          } else {
            markFailed(job, issue);
          }
        } finally {
          active.current = null;
          setCancelling(false);
        }
      }
    } finally {
      if (stopRequested.current) notify("Queue stopped.");
      runningRef.current = false;
      batch.current = null;
      stopRequested.current = false;
      setRunning(false);
      setStopping(false);
      inspectPending();
    }

    if (completed) notify(completed === 1 ? "Video exported." : `${completed} videos exported.`, "success");
    if (lastOutput && settings.openFolderAfterConversion) {
      try {
        await desktop.openFolder(lastOutput.directory);
      } catch (error) {
        notify(failure(error).message, "error");
      }
    }
  }

  async function cancel(stopAll: boolean) {
    if (stopAll && !stopRequested.current) {
      stopRequested.current = true;
      setStopping(true);
    }
    if (active.current) {
      setCancelling(true);
      try {
        await desktop.cancel(active.current.attempt);
      } catch (error) {
        setCancelling(false);
        notify(failure(error).message, "error");
      }
    }
  }

  function onUpdate(update: Update) {
    const current = active.current;
    if (!current || update.jobId !== current.attempt) return;
    if (update.type === "log") appendLog(current.id, update.line);
    if (update.type === "progress") {
      const job = jobsRef.current.find(({ id }) => id === current.id);
      if (job) batch.current?.pace.record(job, update);
      patch(current.id, { progress: update });
    }
    if (update.type === "finalizing") patch(current.id, { finalizing: true });
  }

  const activeBatch = batch.current;
  let batchProgress: number | null = null;
  let queueTimeLeft: number | null = null;
  if (activeBatch) {
    const remaining = jobs.filter((job) =>
      (!activeBatch.onlyJobId || activeBatch.onlyJobId === job.id) &&
      (canConvert(job) || job.status === "encoding"),
    );
    if (!stopping) queueTimeLeft = activeBatch.pace.estimate(remaining);
    const current = remaining.find((job) => job.status === "encoding");
    const percent = current?.finalizing ? 100 : current?.progress?.percent;
    const total = activeBatch.settled + remaining.length;
    if (total && (activeBatch.settled || percent !== undefined)) {
      batchProgress = Math.floor((activeBatch.settled * 100 + (percent ?? 0)) / total);
    }
  }

  return {
    jobs,
    running,
    runningRef,
    stopping,
    cancelling,
    batchProgress,
    queueTimeLeft,
    logs,
    add,
    remove,
    clearFinished,
    retry,
    chooseFrameSize,
    recheck,
    inspectPending,
    start,
    cancel,
    onUpdate,
  };
}
