import type { FrameSize } from "./native";

export type SavedVideo = { path: string; frameSize?: FrameSize | null };

const storageKey = "finer.queue.v1";

export function readQueue(): SavedVideo[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(storageKey) ?? "[]");
    if (!Array.isArray(stored)) return [];
    const paths = new Set<string>();
    return stored.flatMap((item): SavedVideo[] => {
      if (typeof item?.path !== "string" || !item.path.trim()) return [];
      const key = item.path.toLowerCase();
      if (paths.has(key)) return [];
      paths.add(key);
      const size = item.frameSize;
      const validSize = (size?.width === 1920 && size.height === 1080)
        || (size?.width === 1080 && (size.height === 1080 || size.height === 1920));
      return [{
        path: item.path,
        frameSize: size === null ? null : validSize ? { width: size.width, height: size.height } : undefined,
      }];
    });
  } catch {
    return [];
  }
}

export function writeQueue(snapshot: string) {
  localStorage.setItem(storageKey, snapshot);
}
