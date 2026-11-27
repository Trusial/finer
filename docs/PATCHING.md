# How Finer patches videos

The TikTok upload method in Finer changes the **last four audio packets** in an MP4. The audio track declares stereo, but those four AAC packets describe two silent low-frequency effects (LFE) channels. That mismatch at the end of the track is the core of the method.

## What happens during export

1. FFmpeg encodes the video as H.264 and the audio as AAC-LC in an MP4. The audio patch runs when the encoded track is stereo at 48 kHz and has at least five packets.
2. Finer writes four copies of the 15-byte AAC packet below into a new `mdat` box at the end of the file. The original media bytes stay in the file.
3. Finer rewrites the audio sample tables in `moov`. The last four entries point to the new packets, and the tables record their sizes, chunk layout, and offsets. The track's stereo description, sample count, and timestamps are preserved. The video packets are untouched by the patch.

```text
a0 98 00 40 01 28 00 64 00 00 03 50 00 00 38
```

Those packets cover about 85 ms. A decoder can encounter a channel layout change near the end, so keep the original clip for further editing.

## Why the MP4 index stays at the front

The `moov` box contains the track information and tells the player where to find each packet. Finer keeps the rewritten `moov` ahead of the media data. If the index needs more room, Finer moves the media data forward and updates the offsets. Keeping the index up front fixed desktop playback in testing.

The patch changes the file TikTok receives. What TikTok does with the audio mismatch during upload processing is unknown. **Export details** tells you whether Finer applied the patch to your file.

[FFmpeg export code](../engine/src/encode.rs) · [MP4 patch code](../engine/src/mp4.rs)
