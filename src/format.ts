import type { FrameSize } from "./native";

const decimal = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });

export function sizeLabel(bytes: number) {
  if (bytes < 1_000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${decimal.format(bytes / 1_000)} KB`;
  if (bytes < 1_000_000_000) return `${decimal.format(bytes / 1_000_000)} MB`;
  return `${decimal.format(bytes / 1_000_000_000)} GB`;
}

export function timeLabel(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const remainingSeconds = total % 60;
  const padTime = (n: number) => String(n).padStart(2, "0");
  return hours
    ? `${hours}:${padTime(minutes)}:${padTime(remainingSeconds)}`
    : `${padTime(minutes)}:${padTime(remainingSeconds)}`;
}

export function fileLabel(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

export const fpsLabel = (fps: number) => `${decimal.format(fps)} fps`;

export function remainingLabel(seconds: number) {
  const total = Math.max(1, Math.ceil(seconds));
  if (total < 60) return `${total}s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes}m ${total % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

export function frameSizeLabel({ width, height }: FrameSize) {
  return `${width} × ${height}`;
}
