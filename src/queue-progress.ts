import type { Progress } from "./native";
import type { Job } from "./queue";

type Pace = { encoder: string; passes: number; speed: [number?, number?]; workload: number };

export class QueuePace {
  private readonly formats = new Map<string, Pace>();
  private latest?: Pace;

  record(job: Job, progress: Progress) {
    const format = videoFormat(job);
    if (!format) return;
    const previous = this.formats.get(format.key);
    const pace: Pace = previous?.encoder === progress.encoder ? previous : {
      encoder: progress.encoder,
      passes: progress.passes,
      speed: [],
      workload: format.workload,
    };
    if (progress.speed && Number.isFinite(progress.speed)) {
      pace.speed[progress.pass - 1] = progress.speed;
    }
    this.formats.set(format.key, pace);
    this.latest = pace;
  }

  estimate(jobs: Job[]): number | null {
    let seconds = 0;
    for (const job of jobs) {
      if (job.finalizing) continue;
      const format = videoFormat(job);
      if (!job.media || !format) return null;
      const progress = job.progress;
      const pace = this.formats.get(format.key) ?? this.latest;
      if (!pace) return null;
      const pass = progress?.pass ?? 1;
      const passes = progress?.passes ?? pace.passes;
      for (let index = pass; index <= passes; index++) {
        const portion = index === pass ? 1 - (progress?.passPercent ?? 0) / 100 : 1;
        if (portion === 0) continue;
        const speed = (index === pass ? progress?.speed : null)
          ?? ((pace.speed[index - 1] ?? 0) * pace.workload / format.workload);
        if (!speed) return null;
        seconds += job.media.duration * portion / speed;
      }
    }
    return seconds > 0 && Number.isFinite(seconds) ? seconds : null;
  }
}

function videoFormat({ media, frameSize }: Job) {
  if (!media) return null;
  const output = frameSize ?? media;
  return {
    key: `${media.videoCodec}:${media.width}x${media.height}:${output.width}x${output.height}:${media.fps}`,
    workload: (media.width * media.height + output.width * output.height) * media.fps,
  };
}
