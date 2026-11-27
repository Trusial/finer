import * as Dialog from "@radix-ui/react-dialog";
import { Check, CircleAlert, Copy, Info, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { type JobLog, type Notify } from "./queue";
import { Button } from "./ui";

type Toast = {
  id: number;
  message: string;
  kind: "info" | "success" | "error";
  exiting?: boolean;
  action?: { label: string; run: () => void };
};

export function useNotifications() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const counter = useRef(0);
  const timers = useRef(new Map<number, number>());

  useEffect(
    () => () => {
      timers.current.forEach(window.clearTimeout);
    },
    [],
  );

  function dismiss(id: number) {
    window.clearTimeout(timers.current.get(id));
    setToasts((previous) => previous.map((toast) => (toast.id === id ? { ...toast, exiting: true } : toast)));
    timers.current.set(
      id,
      window.setTimeout(() => {
        timers.current.delete(id);
        setToasts((previous) => previous.filter((toast) => toast.id !== id));
      }, 145),
    );
  }

  const notify: Notify = (message, kind = "info", action) => {
    const id = ++counter.current;
    setToasts((previous) => [...previous.slice(-2), { id, message, kind, action }]);
    timers.current.set(
      id,
      window.setTimeout(() => dismiss(id), kind === "error" ? 7500 : 4200),
    );
  };

  return { toasts, notify, dismiss };
}

export function Notifications({ toasts, dismiss }: { toasts: Toast[]; dismiss: (id: number) => void }) {
  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="false">
      {toasts.map(({ id, message, kind, action, exiting }) => {
        const Icon = kind === "error" ? CircleAlert : kind === "success" ? Check : Info;
        return (
          <div key={id} className="toast" data-exiting={exiting || undefined} inert={exiting}>
            <Icon />
            <span>{message}</span>
            {action && (
              <Button
                variant="ghost"
                className="compact"
                onClick={() => {
                  action.run();
                  dismiss(id);
                }}
              >
                {action.label}
              </Button>
            )}
            <Button
              variant="icon"
              className="compact-icon"
              aria-label="Dismiss notification"
              onClick={() => dismiss(id)}
            >
              <X />
            </Button>
          </div>
        );
      })}
    </div>
  );
}

const logToken =
  /(\[[^\]\r\n]+\]|"(?:[^"\\]|\\.)*"|(?:^|(?<=\s))-[a-z][\w:-]*|\b(?:warning|error|fatal|failed|invalid)\b|\b\d+(?:\.\d+)?(?:%|x|kbits?\/s|kbps|fps|kHz|KiB|MiB)?)/gi;

function highlight(line: string) {
  const pieces = [];
  let end = 0;
  for (const match of line.matchAll(logToken)) {
    const index = match.index;
    const token = match[0];
    pieces.push(line.slice(end, index));
    const kind = token.startsWith("[")
      ? "prefix"
      : token.startsWith('"')
        ? "string"
        : token.startsWith("-")
          ? "flag"
          : /^\d/.test(token)
            ? "number"
            : token.toLowerCase() === "warning"
              ? "warning"
              : "error";
    pieces.push(
      <span key={index} className={`log-${kind}`}>
        {token}
      </span>,
    );
    end = index + token.length;
  }
  pieces.push(line.slice(end));
  return pieces;
}

function snapshot(read: () => JobLog | undefined) {
  const log = read();
  return { text: log?.lines.join("\n") ?? "", firstLine: log?.firstLine ?? 1, count: log?.lines.length ?? 0 };
}

export function LogDialog({
  name,
  read,
  onClose,
  onCopy,
  returnFocus,
}: {
  name: string;
  read: () => JobLog | undefined;
  onClose: () => void;
  onCopy: (value: string) => void;
  returnFocus: () => void;
}) {
  const [open, setOpen] = useState(true);
  const [log, setLog] = useState(() => snapshot(read));
  const viewport = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const getLog = useRef(read);
  getLog.current = read;

  useEffect(() => {
    const timer = window.setInterval(() => {
      const next = snapshot(getLog.current);
      setLog((previous) =>
        previous.text === next.text && previous.firstLine === next.firstLine ? previous : next,
      );
    }, 180);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    if (following.current && viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight;
  }, [log]);

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content
          className="modal log-modal"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            returnFocus();
            onClose();
          }}
        >
          <div className="modal-header">
            <div className="modal-heading">
              <Dialog.Title>Logs</Dialog.Title>
              <Dialog.Description className="truncate">{name}</Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <Button variant="icon" aria-label="Close logs">
                <X />
              </Button>
            </Dialog.Close>
          </div>
          <div
            className="log-output"
            aria-label="Conversion logs"
            role="region"
            tabIndex={0}
            ref={viewport}
            onScroll={({ currentTarget: el }) => {
              following.current = el.scrollHeight - el.clientHeight - el.scrollTop < 12;
            }}
          >
            {log.count ? (
              <ol start={log.firstLine} className="log-lines">
                {log.text.split("\n").map((line, index) => (
                  <li key={log.firstLine + index}>
                    <span className="log-line-number" aria-hidden="true">
                      {log.firstLine + index}
                    </span>
                    <code>{highlight(line)}</code>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="log-placeholder">Waiting for FFmpeg output…</p>
            )}
          </div>
          <div className="modal-footer">
            <Button disabled={!log.text} onClick={() => onCopy(log.text)}>
              <Copy />
              Copy logs
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
