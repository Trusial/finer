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
import finerIcon from "./finer-icon.png";

export function App() {
  const { toasts, notify, dismiss } = useNotifications();
  const [logId, setLogId] = useState<string | null>(null);
  const [detailsId, setDetailsId] = useState<string | null>(null);
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
    return (
      <QueueRow key={item.id} item={item}>
        {item.issue && ["outputDirectory", "permissionDenied", "diskFull"].includes(item.issue.code) && (
          <Hint label="Output settings">
            <Button
              variant="icon"
              aria-label="Output settings"
              disabled={queue.running}
              onClick={() => setSettingsOpen("output")}
            >
              <Settings2 />
            </Button>
          </Hint>
        )}
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
          <>
            <Hint label="Play video">
              <Button
                variant="icon"
                aria-label="Play video"
                onClick={() => windowAction(desktop.openResult(output.path, false))}
              >
                <Play />
              </Button>
            </Hint>
            <Hint label="Show in Explorer">
              <Button
                variant="icon"
                aria-label="Show in Explorer"
                onClick={() => windowAction(desktop.openResult(output.path, true))}
              >
                <FolderOpen />
              </Button>
            </Hint>
          </>
        )}
        {output && (
          <Hint label="Export details">
            <Button
              variant="icon"
              aria-label="Export details"
              onClick={(event) => {
                detailsTrigger.current = event.currentTarget;
                setDetailsId(item.id);
              }}
            >
              <Info />
            </Button>
          </Hint>
        )}
        {hasLogs && (
          <Hint label="Logs">
            <Button
              variant="icon"
              aria-label={`Logs for ${fileLabel(item.path)}`}
              onClick={(event) => {
                logTrigger.current = event.currentTarget;
                setLogId(item.id);
              }}
            >
              <Terminal />
            </Button>
          </Hint>
        )}
        {item.status !== "encoding" && (
          <Hint label={isFinished(item) ? "Remove from finished" : "Remove from queue"}>
            <Button
              variant="icon"
              className="remove-job"
              aria-label={`Remove ${fileLabel(item.path)}`}
              onClick={() => queue.remove(item.id)}
            >
              <X />
            </Button>
          </Hint>
        )}
      </QueueRow>
    );
  }

  return (
    <Tooltip.Provider delayDuration={480} skipDelayDuration={100}>
      <div className="app">
        <header className="titlebar" data-tauri-drag-region>
          <div className="brand" data-tauri-drag-region>
            <img className="brand-mark" src={finerIcon} alt="" />
            <span>FINER STUDIO</span>
            <span className="local-badge">LOCAL</span>
          </div>
          <div className="window-actions">
            <div className="caption-controls">
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
          <aside className="tool-rail">
            <span className="rail-label">TOOLS</span>
            <nav aria-label="Tools">
              <button
                className={!blenderActive ? "rail-tool is-active" : "rail-tool"}
                aria-pressed={!blenderActive}
                disabled={queue.running}
                onClick={() => void updateRenderSettings({ renderMode: "quality" })}
              >
                <Gauge />
                <span><strong>Finer</strong><small>Quality</small></span>
              </button>
              <button
                className={blenderActive ? "rail-tool is-active" : "rail-tool"}
                aria-pressed={blenderActive}
                disabled={queue.running}
                onClick={() => void updateRenderSettings({ renderMode: blenderActive ? settings.renderMode : "frameBlend" })}
              >
                <Layers3 />
                <span><strong>Blender</strong><small>Motion</small></span>
              </button>
            </nav>
            <div className="rail-spacer" />
            <button
              ref={settingsButton}
              className="rail-tool rail-settings"
              disabled={queue.running || picking}
              onClick={() => setSettingsOpen("general")}
            >
              <Settings2 />
              <span><strong>Settings</strong><small>System</small></span>
            </button>
          </aside>

          <main className={`${queue.jobs.length ? "workspace" : "workspace is-empty"} ${blenderActive ? "is-blender" : ""}`}>
          <div className="content">
            {queue.jobs.length === 0 ? (
              <>
                <div className="home-intro">
                  <span className="eyebrow">{blenderActive ? "MOTION WORKSPACE" : "UPLOAD WORKSPACE"}</span>
                  <h1 className="home-headline">
                    {blenderActive ? "Build motion." : "Refine the source."}
                  </h1>
                  <p>
                    {blenderActive
                      ? "Drop high-frame-rate footage into the stage. Your active recipe is shown on the right."
                      : "Create clean H.264 exports with Finer's TikTok compatibility pass."}
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
                  <strong>{dragging ? "Release to add footage" : "Drop footage on the stage"}</strong>
                  <span className="drop-hint">MP4, MOV, MKV or WebM · Ctrl+O to browse</span>
                </button>
              </>
            ) : (
              <>
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
              <span>OUTPUT RECIPE</span>
              <strong>{blenderActive ? "BLENDER" : "FINER"}</strong>
            </div>
            {blenderActive ? (
              <>
                <section className="inspector-section">
                  <span className="inspector-label">METHOD</span>
                  <div className="method-stack">
                    <button
                      className={settings.renderMode === "frameBlend" ? "method-button is-active" : "method-button"}
                      aria-pressed={settings.renderMode === "frameBlend"}
                      onClick={() => void updateRenderSettings({ renderMode: "frameBlend" })}
                    >
                      <Layers3 /><span><strong>Frame Blend</strong><small>Adjacent-frame mix</small></span>
                    </button>
                    <button
                      className={settings.renderMode === "motionBlur" ? "method-button is-active" : "method-button"}
                      aria-pressed={settings.renderMode === "motionBlur"}
                      onClick={() => void updateRenderSettings({ renderMode: "motionBlur" })}
                    >
                      <WandSparkles /><span><strong>Motion Blur</strong><small>Interpolated accumulation</small></span>
                    </button>
                  </div>
                </section>
                <section className="inspector-section inspector-fields">
                  <label>
                    <span>Target frame rate</span>
                    <select value={settings.targetFps} onChange={(event) => void updateRenderSettings({ targetFps: Number(event.target.value) })}>
                      {[24, 30, 50, 60, 90, 120].map((fps) => <option key={fps} value={fps}>{fps} FPS</option>)}
                    </select>
                  </label>
                  {settings.renderMode === "motionBlur" && (
                    <label>
                      <span>Temporal samples</span>
                      <select value={settings.blurSamples} onChange={(event) => void updateRenderSettings({ blurSamples: Number(event.target.value) })}>
                        {[2, 4, 6, 8, 12, 16].map((samples) => <option key={samples} value={samples}>{samples} samples</option>)}
                      </select>
                    </label>
                  )}
                </section>
              </>
            ) : (
              <section className="quality-recipe">
                <div className="recipe-icon"><Gauge /></div>
                <strong>TikTok Quality Pass</strong>
                <p>Two-pass H.264 export with AAC compatibility patch and output validation.</p>
              </section>
            )}
            <section className="inspector-section recipe-stats">
              <div><span>VIDEO</span><strong>H.264 HIGH</strong></div>
              <div><span>BITRATE</span><strong>{(settings.videoBitrateKbps / 1000).toFixed(1)} MBPS</strong></div>
              <div><span>ACCELERATION</span><strong>{blenderActive ? "CPU HQ" : settings.gpuEncoding ? "GPU" : "CPU"}</strong></div>
              <div><span>COLOR</span><strong>{settings.colorManagement ? "AUTO" : "BYPASS"}</strong></div>
            </section>
            <section className="inspector-section color-control">
              <div>
                <strong>Color management</strong>
                <small>sRGB → Rec.709 when detected</small>
              </div>
              <Switch.Root
                className="switch"
                checked={settings.colorManagement}
                onCheckedChange={(checked) => void updateRenderSettings({ colorManagement: checked })}
                aria-label="Automatic color management"
              >
                <Switch.Thumb />
              </Switch.Root>
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
