import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { useState } from "react";
import { sizeLabel } from "./format";
import type { AudioPatch, Output } from "./native";
import { Button } from "./ui";

const patchMessages: Record<AudioPatch, string> = {
  applied: "Applied",
  noAudio: "No audio",
  needsStereo: "Needs 48 kHz stereo",
  tooShort: "Audio too short",
};

const encoders: Record<Output["encoder"], string> = {
  libx264: "CPU (x264)",
  h264_nvenc: "GPU (NVIDIA)",
  h264_mf: "GPU (Media Foundation)",
};

const decoders: Record<Output["decoder"], string> = {
  cpu: "CPU",
  d3d11va: "GPU (Direct3D)",
  cuda: "GPU (NVIDIA)",
};

export function ExportDetails({ output, onClose, returnFocus }: {
  output: Output;
  onClose: () => void;
  returnFocus: () => void;
}) {
  const [open, setOpen] = useState(true);

  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content
          className="modal export-details"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            returnFocus();
            onClose();
          }}
        >
          <div className="modal-header">
            <div className="modal-heading">
              <Dialog.Title>Export details</Dialog.Title>
              <Dialog.Description className="truncate" title={output.name}>{output.name}</Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <Button variant="icon" aria-label="Close details"><X /></Button>
            </Dialog.Close>
          </div>
          <div className="export-details-body">
            <dl className="export-facts">
              <div><dt>Encoding</dt><dd>{encoders[output.encoder]}</dd></div>
              <div><dt>Decoding</dt><dd>{decoders[output.decoder]}</dd></div>
              <div><dt>File size</dt><dd>{sizeLabel(output.size)}</dd></div>
              <div><dt>Audio patch</dt><dd>{patchMessages[output.audioPatch]}</dd></div>
              <div><dt>Shadow Lift</dt><dd>{output.shadowLifted ? "Applied" : "Off"}</dd></div>
            </dl>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
