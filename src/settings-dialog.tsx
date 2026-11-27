import * as Dialog from "@radix-ui/react-dialog";
import * as Select from "@radix-ui/react-select";
import * as Switch from "@radix-ui/react-switch";
import { Check, ChevronDown, LoaderCircle, Minus, Monitor, Moon, Plus, Sun, X } from "lucide-react";
import { useRef, useState, type FocusEvent } from "react";
import { bitrateStepKbps, bitrateSteps, desktop, failure, type Settings } from "./native";
import { Button } from "./ui";

const themes = [
  { value: "system", name: "System", icon: Monitor },
  { value: "light", name: "Light", icon: Sun },
  { value: "dark", name: "Dark", icon: Moon },
] as const;

const bitrateFormat = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const bitrateOptions = Array.from({ length: bitrateSteps }, (_, index) => (index + 1) * bitrateStepKbps);

export function SettingsDialog({
  value,
  outputFocus,
  onClose,
  onSave,
  returnFocus,
}: {
  value: Settings;
  outputFocus: boolean;
  onClose: () => void;
  onSave: (settings: Settings) => void;
  returnFocus: () => void;
}) {
  const [open, setOpen] = useState(true);
  const [draft, setDraft] = useState(value);
  const [threads, setThreads] = useState(String(value.threads));
  const [error, setError] = useState("");
  const [browsing, setBrowsing] = useState(false);
  const [writing, setWriting] = useState(false);
  const saved = useRef(value);
  const desired = useRef(value);
  const saving = useRef(false);
  const outputButton = useRef<HTMLButtonElement>(null);
  const busy = browsing || writing;

  async function persist() {
    if (saving.current) return;
    saving.current = true;
    setError("");
    setWriting(true);
    try {
      while (desired.current !== saved.current) {
        const requested = desired.current;
        try {
          const result = await desktop.saveSettings(requested);
          saved.current = requested;
          onSave(result);
        } catch (error) {
          setError(failure(error).message);
          if (desired.current === requested) desired.current = saved.current;
        }
      }
      setDraft(saved.current);
      setThreads(String(saved.current.threads));
    } finally {
      saving.current = false;
      setWriting(false);
    }
  }

  function changeSettings(changes: Partial<Settings>) {
    desired.current = { ...desired.current, ...changes };
    setDraft(desired.current);
    void persist();
  }

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    changeSettings({ [key]: value });
  }

  async function choose(kind: "outputDirectory" | "ffmpegPath") {
    setBrowsing(true);
    setError("");
    try {
      const path = await (kind === "outputDirectory" ? desktop.chooseDirectory() : desktop.chooseFFmpeg());
      if (typeof path === "string") {
        if (kind === "outputDirectory") changeSettings({ outputDirectory: path, useSourceDirectory: false });
        else update("ffmpegPath", path);
      }
    } catch (error) {
      setError(failure(error).message);
    } finally {
      setBrowsing(false);
    }
  }

  function commitThreads(event: FocusEvent<HTMLInputElement>) {
    const count = Number(event.currentTarget.value);
    if (!Number.isInteger(count) || count < 1 || count > 256) {
      setThreads(String(draft.threads));
      setError("Enter a thread count from 1 to 256.");
    } else if (count !== draft.threads) update("threads", count);
  }

  function changeThreads(step: number) {
    update("threads", Math.min(256, Math.max(1, (Number(threads) || 16) + step)));
  }

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => {
        if (!saving.current && !browsing) setOpen(next);
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content
          className="modal settings-modal"
          aria-describedby={undefined}
          onOpenAutoFocus={(event) => {
            if (outputFocus) {
              event.preventDefault();
              outputButton.current?.focus();
            }
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            returnFocus();
            onClose();
          }}
        >
          <div className="modal-header">
            <Dialog.Title>Settings</Dialog.Title>
            {writing && (
              <span className="settings-saving" role="status">
                <LoaderCircle className="spinner" />
                Saving
              </span>
            )}
            <Dialog.Close asChild>
              <Button variant="icon" aria-label="Close settings" disabled={busy}>
                <X />
              </Button>
            </Dialog.Close>
          </div>
          <div className="settings-scroll">
            <fieldset className="theme-field" disabled={busy}>
              <legend>Appearance</legend>
              <div className="theme-options">
                {themes.map(({ value, name, icon: Icon }) => (
                  <label key={value}>
                    <input
                      type="radio"
                      name="theme"
                      value={value}
                      checked={draft.theme === value}
                      onChange={() => update("theme", value)}
                    />
                    <span>
                      <Icon />
                      {name}
                    </span>
                  </label>
                ))}
              </div>
            </fieldset>
            <section className="settings-group" aria-labelledby="export-settings">
              <h3 id="export-settings">Export</h3>
              <div className="setting-row setting-row-with-value">
                <div className="setting-copy">
                  <span className="setting-label">Output folder</span>
                  <span
                    className="setting-value"
                    title={draft.useSourceDirectory ? undefined : (draft.outputDirectory ?? undefined)}
                  >
                    {draft.useSourceDirectory ? "Same as source" : draft.outputDirectory}
                  </span>
                </div>
                <div className="setting-actions">
                  {!draft.useSourceDirectory && (
                    <Button
                      variant="ghost"
                      className="compact"
                      disabled={busy}
                      onClick={() => update("useSourceDirectory", true)}
                    >
                      Use source
                    </Button>
                  )}
                  <Button
                    ref={outputButton}
                    className="compact"
                    disabled={busy}
                    onClick={() => void choose("outputDirectory")}
                  >
                    Browse
                  </Button>
                </div>
              </div>
              <Toggle
                id="auto-open"
                label="Open output folder when finished"
                checked={draft.openFolderAfterConversion}
                disabled={busy}
                onChange={(checked) => update("openFolderAfterConversion", checked)}
              />
            </section>
            <section className="settings-group" aria-labelledby="encoder-settings">
              <h3 id="encoder-settings">Encoder</h3>
              <div className="setting-row">
                <label className="setting-label" htmlFor="video-bitrate">Video bitrate</label>
                <Select.Root
                  value={String(draft.videoBitrateKbps)}
                  disabled={browsing}
                  onValueChange={(kbps) => update("videoBitrateKbps", Number(kbps))}
                >
                  <Select.Trigger id="video-bitrate" className="bitrate-picker">
                    <Select.Value />
                    <Select.Icon asChild><ChevronDown /></Select.Icon>
                  </Select.Trigger>
                  <Select.Portal>
                    <Select.Content
                      className="bitrate-menu"
                      position="popper"
                      align="end"
                      sideOffset={6}
                      collisionPadding={10}
                    >
                      <Select.Viewport className="bitrate-options">
                        {bitrateOptions.map((kbps) => (
                          <Select.Item key={kbps} value={String(kbps)} className="bitrate-option">
                            <Select.ItemText>{bitrateFormat.format(kbps / 1000)} Mbps</Select.ItemText>
                            <Select.ItemIndicator asChild><Check /></Select.ItemIndicator>
                          </Select.Item>
                        ))}
                      </Select.Viewport>
                    </Select.Content>
                  </Select.Portal>
                </Select.Root>
              </div>
              <Toggle
                id="gpu-encoding"
                label="GPU encoding"
                checked={draft.gpuEncoding}
                disabled={busy}
                onChange={(checked) => update("gpuEncoding", checked)}
              />
              <Toggle
                id="gpu-decoding"
                label="GPU decoding"
                checked={draft.gpuDecoding}
                disabled={busy}
                onChange={(checked) => update("gpuDecoding", checked)}
              />
              <div className="setting-row">
                <label className="setting-label" htmlFor="threads">
                  CPU threads
                </label>
                <div className="stepper">
                  <Button
                    variant="icon"
                    aria-label="Use one fewer thread"
                    onClick={() => changeThreads(-1)}
                    disabled={busy || Number(threads) <= 1}
                  >
                    <Minus />
                  </Button>
                  <input
                    id="threads"
                    type="number"
                    min={1}
                    max={256}
                    required
                    value={threads}
                    disabled={busy}
                    onChange={(event) => setThreads(event.target.value)}
                    onBlur={commitThreads}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") event.currentTarget.blur();
                    }}
                  />
                  <Button
                    variant="icon"
                    aria-label="Use one more thread"
                    onClick={() => changeThreads(1)}
                    disabled={busy || Number(threads) >= 256}
                  >
                    <Plus />
                  </Button>
                </div>
              </div>
            </section>
            <section className="settings-group" aria-labelledby="color-settings">
              <h3 id="color-settings">Color</h3>
              <Toggle
                id="shadow-lift"
                label="Shadow Lift"
                checked={draft.shadowLift}
                disabled={busy}
                onChange={(checked) => update("shadowLift", checked)}
              />
              <p className="setting-help">
                Brightens dark Minecraft blocks such as obsidian while protecting bright areas.
              </p>
            </section>
            <section className="settings-group" aria-labelledby="advanced-settings">
              <h3 id="advanced-settings">Advanced</h3>
              <div className="setting-row setting-row-with-value">
                <div className="setting-copy">
                  <span className="setting-label">FFmpeg executable</span>
                  <span className="setting-value" title={draft.ffmpegPath ?? undefined}>
                    {draft.ffmpegPath ?? "Bundled FFmpeg"}
                  </span>
                </div>
                <div className="setting-actions">
                  {draft.ffmpegPath && (
                    <Button
                      variant="ghost"
                      className="compact"
                      disabled={busy}
                      onClick={() => update("ffmpegPath", null)}
                    >
                      Use bundled
                    </Button>
                  )}
                  <Button className="compact" disabled={busy} onClick={() => void choose("ffmpegPath")}>
                    Browse
                  </Button>
                </div>
              </div>
              <Toggle
                id="keep-logs"
                label="Keep encoding logs"
                checked={draft.keepLogs}
                disabled={busy}
                onChange={(checked) => update("keepLogs", checked)}
              />
            </section>
            {error && (
              <p className="settings-error" role="alert">
                {error}
              </p>
            )}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function Toggle({
  id,
  label,
  checked,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="setting-row">
      <div className="setting-copy">
        <label className="setting-label" htmlFor={id}>
          {label}
        </label>
      </div>
      <Switch.Root
        id={id}
        className="switch"
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
      >
        <Switch.Thumb />
      </Switch.Root>
    </div>
  );
}
