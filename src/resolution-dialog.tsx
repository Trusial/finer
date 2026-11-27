import * as Dialog from "@radix-ui/react-dialog";
import { useRef, useState } from "react";
import { frameSizeLabel } from "./format";
import type { FrameSize, Media } from "./native";
import { Button } from "./ui";

export function ResolutionDialog({ media, suggested, matching, onChoose }: {
  media: Media;
  suggested: FrameSize;
  matching: number;
  onChoose: (size: FrameSize | null, all: boolean) => void;
}) {
  const [open, setOpen] = useState(true);
  const [all, setAll] = useState(false);
  const choice = useRef<FrameSize | null>(null);
  const original = frameSizeLabel(media);
  const target = frameSizeLabel(suggested);
  const scale = Math.min(suggested.width / media.width, suggested.height / media.height);
  const action = scale > 1 ? "Upscale" : scale < 1 ? "Downscale" : "Resize";

  function choose(size: FrameSize | null) {
    choice.current = size;
    setOpen(false);
  }

  return (
    <Dialog.Root open={open}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content
          className="modal resolution-modal"
          onEscapeKeyDown={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (!open) onChoose(choice.current, all);
          }}
        >
          <div className="modal-header">
            <Dialog.Title>Unsupported frame size</Dialog.Title>
          </div>
          <Dialog.Description className="confirmation-copy resolution-copy">
            <strong>{media.name}</strong> is {original}. The recommended size is {target}.
            {" "}Resizing keeps the picture’s proportions and adds black bars if needed.
            {(media.width % 2 !== 0 || media.height % 2 !== 0) &&
              " For H.264, use the suggested size to avoid odd dimensions."}
          </Dialog.Description>
          {matching > 1 && (
            <label className="resolution-match">
              <input type="checkbox" checked={all} onChange={(event) => setAll(event.target.checked)} />
              Apply to {matching} matching videos
            </label>
          )}
          <div className="modal-footer resolution-actions">
            <Button disabled={media.width % 2 !== 0 || media.height % 2 !== 0} onClick={() => choose(null)}>
              Keep {original}
            </Button>
            <Button variant="primary" onClick={() => choose(suggested)}>
              {action} to {target}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
