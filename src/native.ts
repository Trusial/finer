import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";

export type Settings = {
  theme: "system" | "light" | "dark";
  outputDirectory: string | null;
  useSourceDirectory: boolean;
  threads: number;
  gpuDecoding: boolean;
  gpuEncoding: boolean;
  videoBitrateKbps: number;
  openFolderAfterConversion: boolean;
  keepLogs: boolean;
  ffmpegPath: string | null;
  renderMode: "quality" | "frameBlend" | "motionBlur";
  targetFps: number;
  blurSamples: number;
  shadowLift: boolean;
};

export const bitrateStepKbps = 16_617;
export const bitrateSteps = 7;

export const defaults: Settings = {
  theme: "system",
  outputDirectory: null,
  useSourceDirectory: true,
  threads: 16,
  gpuDecoding: true,
  gpuEncoding: false,
  videoBitrateKbps: bitrateStepKbps * bitrateSteps,
  openFolderAfterConversion: false,
  keepLogs: false,
  ffmpegPath: null,
  renderMode: "quality",
  targetFps: 60,
  blurSamples: 5,
  shadowLift: false,
};

export type FrameSize = { width: number; height: number };

export type Media = FrameSize & {
  path: string;
  name: string;
  alreadyPatched: boolean;
  suggestedFrameSize: FrameSize | null;
  fps: number;
  duration: number;
  size: number;
  videoCodec: string;
  audioCodec: string | null;
  sampleRate: number | null;
  channelLayout: string | null;
  container: string;
  colorSpace: string | null;
  colorTransfer: string | null;
  colorPrimaries: string | null;
  colorRange: string | null;
};

export type AudioPatch = "applied" | "noAudio" | "needsStereo" | "tooShort";
export type Output = {
  path: string;
  name: string;
  size: number;
  directory: string;
  audioPatch: AudioPatch;
  encoder: "libx264" | "h264_nvenc" | "h264_mf";
  decoder: "cpu" | "d3d11va" | "cuda";
  shadowLifted: boolean;
};
export type Progress = {
  pass: 1 | 2;
  passes: 1 | 2;
  percent: number;
  passPercent: number;
  elapsed: number;
  speed: number | null;
  encoder: string;
};
export type Update = { jobId: string } & (
  ({ type: "progress" } & Progress) | { type: "log"; line: string } | { type: "finalizing" }
);
export type Failure = { code: string; message: string; detail: string };

export function failure(error: unknown): Failure {
  if (typeof error === "object" && error !== null && "code" in error && "message" in error) {
    return error as Failure;
  }
  return { code: "unexpected", message: "Something went wrong. Try again.", detail: String(error) };
}

export const windowApi = getCurrentWindow();

export const desktop = {
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (value: Settings) => invoke<Settings>("save_settings", { value }),
  inspect: (path: string) => invoke<Media>("inspect_media", { path }),
  convert: (jobId: string, queueId: string, path: string, frameSize: FrameSize | null) =>
    invoke<Output>("start_conversion", { jobId, queueId, path, frameSize }),
  discardCheckpoints: (queueIds: string[]) => invoke<void>("discard_checkpoints", { queueIds }),
  cancel: (jobId: string) => invoke<void>("cancel_conversion", { jobId }),
  openFolder: (path: string) => invoke<void>("open_directory", { path }),
  openResult: (path: string, reveal: boolean) => invoke<void>("open_result", { path, reveal }),
  confirmClose: () => invoke<void>("confirm_close"),
  onCloseRequest: (handle: () => void) => listen("close-requested", handle),
  onUpdate: (onUpdate: (update: Update) => void) =>
    listen<Update>("conversion-update", ({ payload }) => onUpdate(payload)),
  chooseVideo: () =>
    open({
      title: "Add videos",
      multiple: true,
      filters: [
        {
          name: "Video files",
          extensions: [
            "mp4",
            "mov",
            "mkv",
            "webm",
            "m4v",
            "avi",
            "mpg",
            "mpeg",
            "ts",
            "flv",
            "ogv",
            "wmv",
            "mxf",
            "ivf",
          ],
        },
        { name: "All files", extensions: ["*"] },
      ],
    }),
  chooseDirectory: () => open({ title: "Choose an output folder", directory: true }),
  chooseFFmpeg: () =>
    open({ title: "Choose FFmpeg", filters: [{ name: "Executable", extensions: ["exe"] }] }),
};
