import * as Dialog from "@radix-ui/react-dialog";
import * as Tooltip from "@radix-ui/react-tooltip";
import * as Switch from "@radix-ui/react-switch";
import { ProgressBarStatus } from "@tauri-apps/api/window";
import {
  ArrowRight,
  Gauge,
  FolderOpen,
  Info,
  Layers3,
  Minus,
  Play,
  Plus,
  RotateCcw,
  Settings2,
  Columns2,
  Square,
  SquaresSubtract,
  Terminal,
  Upload,
  WandSparkles,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { fileLabel, remainingLabel, sizeLabel } from "./format";
import { desktop, defaults, failure, windowApi, type Settings } from "./native";
import { ExportDetails } from "./export-details";
import { LogDialog, Notifications, useNotifications } from "./feedback";
import { isFinished, matchesFrameSize, needsFrameSize, useQueue, type Job } from "./queue";
import { QueueRow } from "./queue-row";
import { ResolutionDialog } from "./resolution-dialog";
import { SettingsDialog } from "./settings-dialog";
import { Button, Hint } from "./ui";
import { CompareDialog, DoneCard, ExportProgress, IosSelect, PillGroup, RowMenu, type RowAction } from "./studio-panels";
import finerIcon from "./finer-icon.png";

export function App() {
  const { toasts, notify, dismiss } = useNotifications();
  const [logId, setLogId] = useState<string | null>(null);
  const [detailsId, setDetailsId] = useState<string | null>(null);
  const [compareId, setCompareId] = useState<string | null>(null);
  const [dismissedDoneId, setDismissedDoneId] = useState<string | null>(null);
  const queue = useQueue(notify, setLogId);
  const [settings, setSettings] = useState(defaults);
  const [settingsOpen, setSettingsOpen] = useState<"general" | "output" | null>(null);
  const [closePrompt, setClosePrompt] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const [picking, setPicking] = useState(false);
  const [connected, setConnected] = useState(false);
  const pickerOpen = useRef(false);
  const modalOpenRef = useRef(false);
  const settingsButton = useRef<HTMLButtonElement>(null);
  const convertButton = useRef<HTMLButtonElement>(null);

  const activeJob = queue.jobs.find(({ status }) => status === "encoding");
  const pendingCount = queue.jobs.filter(
    ({ status }) => status === "pending" || status === "checking" || status === "ready",
  ).length;
  const inspecting = queue.jobs.some(({ status }) => status === "pending" || status === "checking");
  const resolutionJob = !queue.running && !inspecting && !picking && !settingsOpen && !logId && !detailsId && !closePrompt
    ? queue.jobs.find(needsFrameSize)
    : undefined;
  const resolution = resolutionJob?.media?.suggestedFrameSize;
  const modalOpen = !!(settingsOpen || logId || detailsId || closePrompt || resolution);
  modalOpenRef.current = modalOpen;

  function windowAction(action: Promise<unknown>) {
    void action.catch((error: unknown) => notify(failure(error).message, "error"));
  }

  function closeRequested() {
    if (queue.runningRef.current) {
      setLogId(null);
      setDetailsId(null);
      setClosePrompt(true);
    } else {
      windowAction(desktop.confirmClose());
    }
  }

  async function pickVideos() {
    if (pickerOpen.current) return;
    pickerOpen.current = true;
    setPicking(true);
    setDragging(false);
    try {
      const result = await desktop.chooseVideo();
      if (typeof result === "string") queue.add([result]);
      else if (Array.isArray(result)) queue.add(result);
    } catch (error) {
      notify(failure(error).message, "error");
    } finally {
      setPicking(false);
      pickerOpen.current = false;
    }
  }

  // Keep event listeners registered once while giving them the latest callbacks.
  const handlers = useRef({
    update: queue.onUpdate,
    inspect: queue.inspectPending,
    add: queue.add,
    closeRequested,
    pick: pickVideos,
    notify,
  });
  handlers.current = { update: queue.onUpdate, inspect: queue.inspectPending, add: queue.add, closeRequested, pick: pickVideos, notify };

  useEffect(() => {
    let mounted = true;
    const update = desktop.onUpdate((value) => handlers.current.update(value));
    const close = desktop.onCloseRequest(() => handlers.current.closeRequested());
    const drag = windowApi.onDragDropEvent(({ payload }) => {
      const allowed = !modalOpenRef.current && !pickerOpen.current;
      setDragging(allowed && (payload.type === "enter" || payload.type === "over"));
      if (allowed && payload.type === "drop") handlers.current.add(payload.paths);
    });
    const resize = windowApi.onResized(() => {
      void windowApi
        .isMaximized()
        .then(setMaximized)
        .catch(() => {});
    });
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "o" && !modalOpenRef.current) {
        event.preventDefault();
        void handlers.current.pick();
      }
    };
    window.addEventListener("keydown", handleShortcut);
    void Promise.all([
      desktop.settings().then((value) => {
        if (mounted) setSettings({ ...defaults, ...value });
      }),
      Promise.all([update, close]).then(() => {
        if (mounted) {
          setConnected(true);
          handlers.current.inspect();
        }
      }),
      drag,
      resize,
    ])
      .catch((error) => {
        if (mounted) handlers.current.notify(failure(error).message, "error");
      })
      .finally(() => {
        if (mounted) {
          void windowApi.show().catch((error) => handlers.current.notify(failure(error).message, "error"));
        }
      });
    return () => {
      mounted = false;
      window.removeEventListener("keydown", handleShortcut);
      for (const listener of [update, close, drag, resize]) {
        void listener.then((off) => off()).catch(() => {});
      }
    };
  }, []);

  useEffect(() => {
    const color = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        settings.theme === "system" ? (color.matches ? "dark" : "light") : settings.theme;
    };
    apply();
    color.addEventListener("change", apply);
    return () => color.removeEventListener("change", apply);
  }, [settings.theme]);

  const { running, batchProgress } = queue;
  useEffect(() => {
    if (!connected) return;
    const status = !running ? ProgressBarStatus.None
      : batchProgress === null ? ProgressBarStatus.Indeterminate
      : ProgressBarStatus.Normal;
    void windowApi.setProgressBar({ status, progress: batchProgress ?? 0 }).catch(() => {});
  }, [connected, running, batchProgress]);

  function saveSettings(value: Settings) {
    setSettings(value);
    if (value.ffmpegPath !== settings.ffmpegPath) queue.recheck();
  }

  async function updateRenderSettings(changes: Partial<Settings>) {
    if (queue.running) return;
    const previous = settings;
    const next = { ...settings, ...changes };
    setSettings(next);
    try {
      setSettings(await desktop.saveSettings(next));
    } catch (error) {
      setSettings(previous);
      notify(failure(error).message, "error");
    }
  }

  const blenderActive = settings.renderMode !== "quality";

  async function copy(value: string, message: string) {
    try {
      await navigator.clipboard.writeText(value);
      notify(message, "success");
    } catch {
      notify("Could not copy to the clipboard.", "error");
    }
  }

  const queued = queue.jobs.filter((item) => !isFinished(item));
  const finished = queue.jobs.filter(isFinished);
  const estimate = queue.running || inspecting ? 0 : queued.reduce((bytes, { media, status }) => {
    if (!media || status === "alreadyPatched") return bytes;
    const audioKbps = media.audioCodec ? 160 : 0;
    return bytes + (settings.videoBitrateKbps + audioKbps) * 125 * media.duration;
  }, 0);
  const logTrigger = useRef<HTMLElement | null>(null);
  const logJob = queue.jobs.find(({ id }) => id === logId);
  const detailsJob = queue.jobs.find(({ id }) => id === detailsId);
  const detailsTrigger = useRef<HTMLButtonElement | null>(null);

  function renderQueueRow(item: Job) {
    const hasLogs = item.status === "encoding" || queue.logs.current.has(item.id);
    const output = item.output;
    const actions: RowAction[] = [];
    if (output) {
      actions.push({ label: "Compare before / after", icon: <Columns2 />, onSelect: () => setCompareId(item.id) });
      actions.push({ label: "Show in Explorer", icon: <FolderOpen />, onSelect: () => windowAction(desktop.openResult(output.path, true)) });
      actions.push({ label: "Export details", icon: <Info />, onSelect: () => setDetailsId(item.id) });
    }
    if (item.issue && ["outputDirectory", "permissionDenied", "diskFull"].includes(item.issue.code)) {
      actions.push({ label: "Output settings", icon: <Settings2 />, disabled: queue.running, onSelect: () => setSettingsOpen("output") });
    }
    if (hasLogs) actions.push({ label: "Logs", icon: <Terminal />, onSelect: () => setLogId(item.id) });
    if (item.status !== "encoding") {
      actions.push({ label: isFinished(item) ? "Remove from finished" : "Remove from queue", icon: <X />, danger: true, onSelect: () => queue.remove(item.id) });
    }
    return (
      <QueueRow key={item.id} item={item}>
        {(item.status === "failed" || item.status === "cancelled") && (
          <Hint label={item.status === "failed" ? "Retry" : "Queue again"}>
            <Button
              variant="icon"
              aria-label={item.status === "failed" ? "Retry" : "Queue again"}
              disabled={queue.running}
              onClick={() => {
                queue.retry(item.id);
                if (item.status === "failed") void queue.start(settings, item.id);
              }}
            >
              <RotateCcw />
            </Button>
          </Hint>
        )}
        {output && (
          <Hint label="Play video">
            <Button variant="icon" aria-label="Play video" onClick={() => windowAction(desktop.openResult(output.path, false))}>
              <Play />
            </Button>
          </Hint>
        )}
        <RowMenu label={`More actions for ${fileLabel(item.path)}`} actions={actions} />
      </QueueRow>
    );
  }

  const latestDone = !queue.running
    ? [...queue.jobs].reverse().find((job) => job.status === "complete" && job.output)
    : undefined;
  const doneJob = latestDone && latestDone.id !== dismissedDoneId ? latestDone : undefined;
  const compareJob = queue.jobs.find((job) => job.id === compareId && !!job.output);

  return (
    <Tooltip.Provider delayDuration={480} skipDelayDuration={100}>
      <div className="app">
        <header className="titlebar" data-tauri-drag-region>
          <div className="brand" data-tauri-drag-region>
            <img className="brand-mark" src={finerIcon} alt="" />
            <span>Finer</span>
          </div>
          <PillGroup
            className="mode-switch"
            axis="x"
            label="Tool"
            activeKey={blenderActive ? "blender" : "finer"}
            onPick={(key) => void updateRenderSettings({ renderMode: key === "blender" ? (blenderActive ? settings.renderMode : "frameBlend") : "quality" })}
          >
            <button
              data-key="finer"
              className={!blenderActive ? "segment is-active" : "segment"}
              aria-pressed={!blenderActive}
              disabled={queue.running}
              onClick={() => void updateRenderSettings({ renderMode: "quality" })}
            >
              Finer
            </button>
            <button
              data-key="blender"
              className={blenderActive ? "segment is-active" : "segment"}
              aria-pressed={blenderActive}
              disabled={queue.running}
              onClick={() => void updateRenderSettings({ renderMode: blenderActive ? settings.renderMode : "frameBlend" })}
            >
              Blender
            </button>
          </PillGroup>
          <div className="window-actions">
            <button
              ref={settingsButton}
              className="caption-button settings-gear"
              aria-label="Settings"
              disabled={queue.running || picking}
              onClick={() => setSettingsOpen("general")}
            >
              <Settings2 />
            </button>            <div className="caption-controls">
              <button
                className="caption-button"
                aria-label="Minimize"
                onClick={() => windowAction(windowApi.minimize())}
              >
                <Minus />
              </button>
              <button
                className="caption-button"
                aria-label={maximized ? "Restore window" : "Maximize"}
                onClick={() => windowAction(windowApi.toggleMaximize())}
              >
                {maximized ? <SquaresSubtract /> : <Square />}
              </button>
              <button
                className="caption-button"
                aria-label="Close window"
                onClick={() => windowAction(windowApi.close())}
              >
                <X />
              </button>
            </div>
          </div>
        </header>

        <div className="studio-layout">

          <main className={`${queue.jobs.length ? "workspace" : "workspace is-empty"} ${blenderActive ? "is-blender" : ""}`}>
          <div className="content">
            {queue.jobs.length === 0 ? (
              <>
                <div className="home-intro" key={blenderActive ? "blender" : "finer"}>
                  <span className="eyebrow">{blenderActive ? "Blender · Motion" : "Finer · Quality"}</span>
                  <h1 className="home-headline">
                    {blenderActive ? "Build motion." : "Make your video sharper."}
                  </h1>
                  <p>
                    {blenderActive
                      ? "Drop high-frame-rate footage into the stage. Your active recipe is shown on the right."
                      : "Clean H.264 exports with a TikTok quality pass."}
                  </p>
                </div>
                <button
                  className={`drop-zone ${dragging ? "is-dragging" : ""}`}
                  disabled={picking}
                  onClick={() => void pickVideos()}
                  aria-label="Browse for videos"
                >
                  <span className="upload-icon">
                    <Upload />
                  </span>
                  <strong>{dragging ? "Release to add your video" : "Drop your video here"}</strong>
                  <span className="drop-hint">MP4, MOV, MKV or WebM · Ctrl+O to browse</span>
                </button>
              </>
            ) : (
              <>
                {activeJob && <ExportProgress job={activeJob} timeLeft={queue.queueTimeLeft} />}
                {doneJob && (
                  <DoneCard
                    key={doneJob.id}
                    job={doneJob}
                    onPlay={() => windowAction(desktop.openResult(doneJob.output!.path, false))}
                    onReveal={() => windowAction(desktop.openResult(doneJob.output!.path, true))}
                    onCompare={() => setCompareId(doneJob.id)}
                    onDismiss={() => setDismissedDoneId(doneJob.id)}
                  />
                )}
                <section aria-labelledby="queued-title" className="queue-section">
                  <div className="queue-heading">
                    <h1 id="queued-title">
                      Queued <span className="queue-count">{queued.length}</span>
                    </h1>
                    <Button className="compact" onClick={() => void pickVideos()} disabled={picking}>
                      <Plus />
                      Add files
                    </Button>
                  </div>
                  {dragging && (
                    <div className="drop-target">
                      <Upload />
                      Release to add to queue
                    </div>
                  )}
                  {queued.length ? (
                    <ul className="queue-list">{queued.map(renderQueueRow)}</ul>
                  ) : (
                    !dragging && <p className="queue-empty">No files queued</p>
                  )}
                </section>
                {finished.length > 0 && (
                  <section aria-labelledby="finished-title" className="queue-section">
                    <div className="queue-heading">
                      <h2 id="finished-title">
                        Finished <span className="queue-count">{finished.length}</span>
                      </h2>
                      <Button variant="ghost" className="compact" onClick={queue.clearFinished}>
                        Clear finished
                      </Button>
                    </div>
                    <ul className="queue-list">{finished.map(renderQueueRow)}</ul>
                  </section>
                )}
              </>
            )}
          </div>
          </main>

          <aside className="inspector" aria-label="Output recipe">
            <div className="inspector-heading">
              <span>Output</span>
              <strong className="mode-chip" key={blenderActive ? "b" : "f"}>{blenderActive ? "Blender" : "Finer"}</strong>
            </div>
            {blenderActive ? (
              <section className="inspector-section mode-card swap-in">
                <PillGroup className="segmented" axis="x" label="Method" activeKey={settings.renderMode} onPick={(key) => void updateRenderSettings({ renderMode: key as Settings["renderMode"] })}>
                  <button
                    data-key="frameBlend"
                    className={settings.renderMode === "frameBlend" ? "segment is-active" : "segment"}
                    aria-pressed={settings.renderMode === "frameBlend"}
                    onClick={() => void updateRenderSettings({ renderMode: "frameBlend" })}
                  >
                    <Layers3 />Frame Blend
                  </button>
                  <button
                    data-key="motionBlur"
                    className={settings.renderMode === "motionBlur" ? "segment is-active" : "segment"}
                    aria-pressed={settings.renderMode === "motionBlur"}
                    onClick={() => void updateRenderSettings({ renderMode: "motionBlur" })}
                  >
                    <WandSparkles />Motion Blur
                  </button>
                </PillGroup>
                <p className="mode-hint" key={settings.renderMode}>
                  {settings.renderMode === "motionBlur" ? "Interpolated shutter blur" : "Adjacent-frame mix"}
                </p>
                <div className="field-row">
                  <IosSelect
                    label="Frame rate"
                    value={settings.targetFps}
                    options={[24, 30, 50, 60, 90, 120].map((fps) => ({ value: fps, label: `${fps} FPS` }))}
                    onChange={(targetFps) => void updateRenderSettings({ targetFps })}
                  />
                  <IosSelect
                    label="Samples"
                    value={settings.blurSamples}
                    disabled={settings.renderMode !== "motionBlur"}
                    options={[3, 5, 7, 9, 11, 13, 15].map((samples) => ({ value: samples, label: String(samples) }))}
                    onChange={(blurSamples) => void updateRenderSettings({ blurSamples })}
                  />
                </div>
              </section>
            ) : (
              <section className="inspector-section mode-card quality-recipe swap-in">
                <div className="recipe-icon"><Gauge /></div>
                <strong>TikTok Quality Pass</strong>
                <p>Two-pass H.264 export with AAC compatibility patch and output validation.</p>
              </section>
            )}
            <section className="inspector-section recipe-stats">
              <div><span>Video</span><strong>H.264 High</strong></div>
              <div>
                <span>{blenderActive ? "Quality" : "Bitrate"}</span>
                <strong>{blenderActive ? "CRF 8 master" : `${(settings.videoBitrateKbps / 1000).toFixed(1)} Mbps`}</strong>
              </div>
              <div><span>Acceleration</span><strong>{blenderActive ? "CPU HQ" : settings.gpuEncoding ? "GPU" : "CPU"}</strong></div>
              {blenderActive && (
                <div className="stat-toggle">
                  <span>Bright Mode<small>Brighter obsidian and dark blocks</small></span>
                  <Switch.Root
                    className="switch"
                    checked={settings.shadowLift}
                    onCheckedChange={(checked) => void updateRenderSettings({ shadowLift: checked })}
                    aria-label="Bright Mode"
                  >
                    <Switch.Thumb />
                  </Switch.Root>
                </div>
              )}
            </section>
          </aside>
        </div>

        <footer className="footer">
          <div className="footer-content">
            {queue.queueTimeLeft !== null && (
              <span className="queue-eta">{remainingLabel(queue.queueTimeLeft)} left</span>
            )}
            {estimate > 0 && <span className="queue-eta" title="Estimated total export size">Est. {sizeLabel(estimate)}</span>}
            <div className="inline-actions">
              {queue.running ? (
                <>
                  {activeJob && (
                    <Button
                      variant="ghost"
                      disabled={queue.stopping || queue.cancelling || activeJob.finalizing}
                      onClick={() => void queue.cancel(false)}
                    >
                      {queue.cancelling ? "Cancelling..." : "Cancel file"}
                    </Button>
                  )}
                  <Button disabled={queue.stopping} onClick={() => void queue.cancel(true)}>
                    <Square />
                    {queue.stopping ? "Stopping..." : "Stop queue"}
                  </Button>
                </>
              ) : (
                <Button
                  ref={convertButton}
                  variant="primary"
                  disabled={!pendingCount || !connected || inspecting || Boolean(resolutionJob)}
                  onClick={() => void queue.start(settings)}
                >
                  {blenderActive
                    ? (pendingCount > 1 ? `Render ${pendingCount} files` : "Render")
                    : (pendingCount > 1 ? `Refine ${pendingCount} files` : "Refine")}
                  <ArrowRight />
                </Button>
              )}
            </div>
          </div>
        </footer>

        <Notifications toasts={toasts} dismiss={dismiss} />
        {settingsOpen && (
          <SettingsDialog
            value={settings}
            outputFocus={settingsOpen === "output"}
            onClose={() => setSettingsOpen(null)}
            onSave={saveSettings}
            returnFocus={() => settingsButton.current?.focus()}
          />
        )}
        {detailsJob?.output && (
          <ExportDetails
            key={detailsJob.id}
            output={detailsJob.output}
            onClose={() => setDetailsId(null)}
            returnFocus={() => {
              if (detailsTrigger.current?.isConnected) detailsTrigger.current.focus();
              detailsTrigger.current = null;
            }}
          />
        )}
        {compareJob && <CompareDialog key={compareJob.id} job={compareJob} onClose={() => setCompareId(null)} />}
        {logJob && (
          <LogDialog
            key={logJob.id}
            name={fileLabel(logJob.path)}
            read={() => queue.logs.current.get(logJob.id)}
            onClose={() => setLogId(null)}
            returnFocus={() => {
              if (logTrigger.current?.isConnected) logTrigger.current.focus();
              logTrigger.current = null;
            }}
            onCopy={(value) => void copy(value, "Logs copied.")}
          />
        )}
        {resolutionJob?.media && resolution && (
          <ResolutionDialog
            key={resolutionJob.id}
            media={resolutionJob.media}
            suggested={resolution}
            matching={queue.jobs.filter((job) => matchesFrameSize(resolutionJob, job)).length}
            onChoose={(size, all) => {
              queue.chooseFrameSize(resolutionJob.id, size, all);
              const remaining = queue.jobs.some((job) =>
                job.id !== resolutionJob.id && needsFrameSize(job) && (!all || !matchesFrameSize(resolutionJob, job)),
              );
              if (!remaining) requestAnimationFrame(() => convertButton.current?.focus());
            }}
          />
        )}
        <Dialog.Root open={closePrompt} onOpenChange={setClosePrompt}>
          <Dialog.Portal>
            <Dialog.Overlay className="modal-overlay" />
            <Dialog.Content className="modal confirmation">
              <div className="modal-header">
                <Dialog.Title>Stop converting and close?</Dialog.Title>
              </div>
              <Dialog.Description className="confirmation-copy">
                The current conversion will be cancelled. Finished videos are saved.
              </Dialog.Description>
              <div className="modal-footer">
                <Dialog.Close asChild>
                  <Button>Keep converting</Button>
                </Dialog.Close>
                <Button variant="primary" onClick={() => windowAction(desktop.confirmClose())}>
                  Stop and close
                </Button>
              </div>
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      </div>
    </Tooltip.Provider>
  );
}
