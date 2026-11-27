use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

const REPLACED_PACKETS: u32 = 4;
// The Orion sample's AAC-LC program config signals two LFE elements at 48 kHz.
const LFE_SILENCE: [u8; 15] = [
    0xa0, 0x98, 0x00, 0x40, 0x01, 0x28, 0x00, 0x64, 0x00, 0x00, 0x03, 0x50, 0x00, 0x00, 0x38,
];

type Kind = [u8; 4];

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("Truncated MP4 table"))?;
    Ok(u32::from_be_bytes(field.try_into().unwrap()))
}

struct Atom<'a> {
    kind: Kind,
    bytes: &'a [u8],
    body: &'a [u8],
}

fn next_atom<'a>(remaining: &mut &'a [u8]) -> io::Result<Atom<'a>> {
    let size = u32_at(remaining, 0)?;
    let kind = remaining
        .get(4..8)
        .ok_or_else(|| invalid("Truncated MP4 box"))?
        .try_into()
        .unwrap();
    let (size, header) = match size {
        0 => (remaining.len(), 8),
        1 => {
            let extended = remaining
                .get(8..16)
                .ok_or_else(|| invalid("Truncated MP4 box"))?;
            let size = u64::from_be_bytes(extended.try_into().unwrap());
            (
                usize::try_from(size).map_err(|_| invalid("MP4 box too large"))?,
                16,
            )
        }
        size => (size as usize, 8),
    };
    if size < header || size > remaining.len() {
        return Err(invalid("Invalid MP4 box size"));
    }
    let (bytes, rest) = remaining.split_at(size);
    *remaining = rest;
    Ok(Atom {
        kind,
        bytes,
        body: &bytes[header..],
    })
}

fn child(mut body: &[u8], kind: Kind) -> io::Result<&[u8]> {
    while !body.is_empty() {
        let atom = next_atom(&mut body)?;
        if atom.kind == kind {
            return Ok(atom.body);
        }
    }
    Err(invalid("Missing MP4 box"))
}

fn write_atom(out: &mut Vec<u8>, kind: Kind, body: &[u8]) -> io::Result<()> {
    let size = u32::try_from(body.len() + 8).map_err(|_| invalid("MP4 index too large"))?;
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(body);
    Ok(())
}

fn rewrite(
    mut body: &[u8],
    mut update: impl FnMut(Atom<'_>, &mut Vec<u8>) -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len());
    while !body.is_empty() {
        update(next_atom(&mut body)?, &mut out)?;
    }
    Ok(out)
}

fn change_child(
    body: &[u8],
    kind: Kind,
    mut change: impl FnMut(&[u8]) -> io::Result<Vec<u8>>,
) -> io::Result<Vec<u8>> {
    rewrite(body, |atom, out| {
        if atom.kind == kind {
            write_atom(out, kind, &change(atom.body)?)
        } else {
            out.extend_from_slice(atom.bytes);
            Ok(())
        }
    })
}

fn table(body: &[u8], row_size: usize) -> io::Result<std::slice::ChunksExact<'_, u8>> {
    let count = u32_at(body, 4)? as usize;
    let rows = body
        .get(8..)
        .ok_or_else(|| invalid("Truncated MP4 table"))?
        .chunks_exact(row_size);
    if rows.len() != count || !rows.remainder().is_empty() {
        return Err(invalid("Invalid MP4 table count"));
    }
    Ok(rows)
}

fn patch_samples(stbl: &[u8], tail_offset: u64) -> io::Result<Option<Vec<u8>>> {
    let stsz = child(stbl, *b"stsz")?;
    let size = u32_at(stsz, 4)?;
    let samples = u32_at(stsz, 8)?;
    if samples <= REPLACED_PACKETS {
        return Ok(None);
    }
    let keep = samples - REPLACED_PACKETS;
    let mut sizes = stsz[..4].to_vec();
    sizes.extend_from_slice(&[0; 4]);
    sizes.extend_from_slice(&samples.to_be_bytes());
    if size == 0 {
        let (original, trailing) = stsz[12..].as_chunks::<4>();
        if original.len() != samples as usize || !trailing.is_empty() {
            return Err(invalid("Invalid MP4 sample sizes"));
        }
        sizes.extend_from_slice(&stsz[12..12 + keep as usize * 4]);
    } else {
        for _ in 0..keep {
            sizes.extend_from_slice(&size.to_be_bytes());
        }
    }
    for _ in 0..REPLACED_PACKETS {
        sizes.extend_from_slice(&(LFE_SILENCE.len() as u32).to_be_bytes());
    }

    let original_runs = child(stbl, *b"stsc")?;
    let mut runs = table(original_runs, 12)?.peekable();
    let mut per_chunk = 0;
    let mut sample_index = 0u32;
    let (offset_kind, offsets) = chunk_offsets(stbl)?;
    let wide = offset_kind == *b"co64";
    let mut new_offsets = Vec::new();
    let mut new_runs = Vec::new();
    for (index, row) in table(offsets, offset_width(offset_kind))?.enumerate() {
        let chunk = index as u32 + 1;
        if let Some(next) = runs.peek() {
            let start = u32_at(next, 0)?;
            if start < chunk {
                return Err(invalid("Invalid MP4 chunk order"));
            }
            if start == chunk {
                per_chunk = u32_at(next, 4)?;
                if u32_at(next, 8)? != 1 {
                    return Err(invalid("Unexpected AAC sample description"));
                }
                runs.next();
            }
        }
        if per_chunk == 0 {
            return Err(invalid("MP4 chunk has no samples"));
        }
        if sample_index < keep {
            let count = per_chunk.min(keep - sample_index);
            if new_runs.last().is_none_or(|&(_, prior)| prior != count) {
                new_runs.push((chunk, count));
            }
            new_offsets.push(read_offset(row));
        }
        sample_index = sample_index
            .checked_add(per_chunk)
            .ok_or_else(|| invalid("MP4 sample count overflow"))?;
    }
    if sample_index != samples || runs.next().is_some() {
        return Err(invalid("MP4 chunks do not cover all audio samples"));
    }
    let tail_chunk = new_offsets.len() as u32 + 1;
    if new_runs
        .last()
        .is_none_or(|&(_, prior)| prior != REPLACED_PACKETS)
    {
        new_runs.push((tail_chunk, REPLACED_PACKETS));
    }
    new_offsets.push(tail_offset);

    let mut chunk_runs = original_runs[..4].to_vec();
    chunk_runs.extend_from_slice(&(new_runs.len() as u32).to_be_bytes());
    for (chunk, count) in new_runs {
        chunk_runs.extend_from_slice(&chunk.to_be_bytes());
        chunk_runs.extend_from_slice(&count.to_be_bytes());
        chunk_runs.extend_from_slice(&1u32.to_be_bytes());
    }
    let wide = wide || tail_offset > u64::from(u32::MAX);
    let mut chunk_offsets = offsets[..4].to_vec();
    chunk_offsets.extend_from_slice(&(new_offsets.len() as u32).to_be_bytes());
    for offset in new_offsets {
        if wide {
            chunk_offsets.extend_from_slice(&offset.to_be_bytes());
        } else {
            chunk_offsets.extend_from_slice(&(offset as u32).to_be_bytes());
        }
    }
    rewrite(stbl, |atom, out| match &atom.kind {
        b"stsz" => write_atom(out, atom.kind, &sizes),
        b"stsc" => write_atom(out, atom.kind, &chunk_runs),
        b"co64" | b"stco" => {
            write_atom(out, if wide { *b"co64" } else { *b"stco" }, &chunk_offsets)
        }
        _ => {
            out.extend_from_slice(atom.bytes);
            Ok(())
        }
    })
    .map(Some)
}

fn video_encoder_tag() -> io::Result<Vec<u8>> {
    let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
    data.extend_from_slice(concat!("finer ", env!("CARGO_PKG_VERSION"), "\0").as_bytes());
    let mut name = Vec::new();
    write_atom(&mut name, *b"data", &data)?;
    let mut tags = Vec::new();
    write_atom(&mut tags, *b"\xa9too", &name)?;
    let mut meta = vec![0; 4];
    let mut handler = [0; 25];
    handler[8..12].copy_from_slice(b"mdir");
    write_atom(&mut meta, *b"hdlr", &handler)?;
    write_atom(&mut meta, *b"ilst", &tags)?;
    let mut udta = Vec::new();
    write_atom(&mut udta, *b"meta", &meta)?;
    Ok(udta)
}

fn without_global_encoder(udta: &[u8]) -> io::Result<Vec<u8>> {
    rewrite(udta, |atom, out| {
        if atom.kind != *b"meta" {
            out.extend_from_slice(atom.bytes);
            return Ok(());
        }
        let mut meta = atom
            .body
            .get(..4)
            .ok_or_else(|| invalid("Invalid MP4 metadata"))?
            .to_vec();
        let mut has_metadata = false;
        let entries = rewrite(&atom.body[4..], |entry, out| {
            if entry.kind == *b"ilst" {
                let list = rewrite(entry.body, |tag, out| {
                    if tag.kind != *b"\xa9too" {
                        out.extend_from_slice(tag.bytes);
                    }
                    Ok(())
                })?;
                if !list.is_empty() {
                    has_metadata = true;
                    write_atom(out, entry.kind, &list)?;
                }
            } else {
                has_metadata |= entry.kind != *b"hdlr";
                out.extend_from_slice(entry.bytes);
            }
            Ok(())
        })?;
        if has_metadata {
            meta.extend_from_slice(&entries);
            write_atom(out, atom.kind, &meta)?;
        }
        Ok(())
    })
}

fn patch_index(moov: &[u8], tail_offset: u64) -> io::Result<Option<Vec<u8>>> {
    let mut patched_audio = false;
    let index = rewrite(moov, |atom, out| {
        match &atom.kind {
            b"trak" => {
                let mdia = child(atom.body, *b"mdia")?;
                match child(mdia, *b"hdlr")?.get(8..12) {
                    Some(b"soun") => {
                        let stbl = child(child(mdia, *b"minf")?, *b"stbl")?;
                        if let Some(index) = patch_samples(stbl, tail_offset)? {
                            let track = change_child(atom.body, *b"mdia", |mdia| {
                                let mdia = change_child(mdia, *b"hdlr", |body| {
                                    let mut handler = body
                                        .get(..24)
                                        .ok_or_else(|| invalid("Invalid audio handler"))?
                                        .to_vec();
                                    handler.extend_from_slice(b"SoundHandler\0");
                                    Ok(handler)
                                })?;
                                change_child(&mdia, *b"minf", |minf| {
                                    change_child(minf, *b"stbl", |_| Ok(index.clone()))
                                })
                            })?;
                            patched_audio = true;
                            return write_atom(out, atom.kind, &track);
                        }
                    }
                    Some(b"vide") => {
                        let mut track = atom.body.to_vec();
                        write_atom(&mut track, *b"udta", &video_encoder_tag()?)?;
                        return write_atom(out, atom.kind, &track);
                    }
                    _ => {}
                }
            }
            b"udta" => {
                let metadata = without_global_encoder(atom.body)?;
                if !metadata.is_empty() {
                    write_atom(out, atom.kind, &metadata)?;
                }
                return Ok(());
            }
            _ => {}
        }
        out.extend_from_slice(atom.bytes);
        Ok(())
    })?;
    Ok(patched_audio.then_some(index))
}

struct FileAtom {
    kind: Kind,
    body: u64,
    end: u64,
}

impl FileAtom {
    fn read(file: &mut File, start: u64, end_of_file: u64) -> io::Result<Self> {
        file.seek(SeekFrom::Start(start))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let (size, header_size) = match u32_at(&header, 0)? {
            0 => (end_of_file - start, 8),
            1 => {
                let mut extended = [0; 8];
                file.read_exact(&mut extended)?;
                (u64::from_be_bytes(extended), 16)
            }
            size => (u64::from(size), 8),
        };
        if size < header_size || size > end_of_file - start {
            return Err(invalid("Invalid MP4 box length"));
        }
        Ok(Self {
            kind: header[4..].try_into().unwrap(),
            body: start + header_size,
            end: start + size,
        })
    }

    fn payload(&self, file: &mut File, limit: u64) -> io::Result<Vec<u8>> {
        let len = self.end - self.body;
        if len > limit {
            return Err(invalid("MP4 index too large"));
        }
        let len = usize::try_from(len).map_err(|_| invalid("MP4 index too large"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(io::Error::other)?;
        bytes.resize(len, 0);
        file.seek(SeekFrom::Start(self.body))?;
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

fn track_index(mut movie: &[u8], kind: Kind) -> io::Result<&[u8]> {
    while !movie.is_empty() {
        let track = next_atom(&mut movie)?;
        if track.kind == *b"trak" {
            let media = child(track.body, *b"mdia")?;
            if child(media, *b"hdlr")?.get(8..12) == Some(&kind) {
                return child(child(media, *b"minf")?, *b"stbl");
            }
        }
    }
    Err(invalid("MP4 track is missing"))
}

fn find_patch(file: &mut File) -> io::Result<bool> {
    const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
    let tail_len = u64::from(REPLACED_PACKETS) * LFE_SILENCE.len() as u64;
    let end = file.metadata()?.len();
    let mut cursor = 0;
    let mut movie = None;
    let mut tail = None;
    let mut saw_media = false;

    while cursor < end {
        let atom = FileAtom::read(file, cursor, end)?;
        cursor = atom.end;
        match &atom.kind {
            b"moov" => movie = Some(atom),
            b"mdat" => {
                if saw_media && atom.end - atom.body == tail_len {
                    let bytes = atom.payload(file, tail_len)?;
                    if bytes
                        .as_chunks::<{ LFE_SILENCE.len() }>()
                        .0
                        .iter()
                        .all(|packet| *packet == LFE_SILENCE)
                    {
                        tail = Some(atom.body);
                    }
                }
                saw_media = true;
            }
            _ => {}
        }
    }

    let (Some(movie), Some(tail)) = (movie, tail) else {
        return Ok(false);
    };
    let movie = movie.payload(file, MAX_INDEX_BYTES)?;
    let audio = track_index(&movie, *b"soun")?;
    let (kind, offsets) = chunk_offsets(audio)?;
    let width = offset_width(kind);
    let offsets = table(offsets, width)?;
    let chunks = offsets.len() as u32;
    let last_offset = offsets.last().map(read_offset);
    let last_run = table(child(audio, *b"stsc")?, 12)?.last();

    let sizes = child(audio, *b"stsz")?;
    let count = u32_at(sizes, 8)?;
    if last_offset != Some(tail) || count <= REPLACED_PACKETS {
        return Ok(false);
    }
    let Some(run) = last_run else {
        return Ok(false);
    };
    let first_chunk = u32_at(run, 0)?;
    if first_chunk == 0
        || first_chunk > chunks
        || u32_at(run, 4)? != REPLACED_PACKETS
        || u32_at(run, 8)? != 1
    {
        return Ok(false);
    }
    let fixed_size = u32_at(sizes, 4)?;
    if fixed_size != 0 {
        return Ok(fixed_size == LFE_SILENCE.len() as u32);
    }
    let (samples, extra) = sizes[12..].as_chunks::<4>();
    Ok(extra.is_empty()
        && samples.len() == count as usize
        && samples
            .iter()
            .rev()
            .take(REPLACED_PACKETS as usize)
            .all(|size| u32::from_be_bytes(*size) == LFE_SILENCE.len() as u32))
}

fn chunk_offsets(samples: &[u8]) -> io::Result<(Kind, &[u8])> {
    if let Ok(offsets) = child(samples, *b"co64") {
        Ok((*b"co64", offsets))
    } else {
        child(samples, *b"stco").map(|offsets| (*b"stco", offsets))
    }
}

fn offset_width(kind: Kind) -> usize {
    if kind == *b"co64" {
        8
    } else {
        4
    }
}

fn read_offset(bytes: &[u8]) -> u64 {
    match bytes.len() {
        4 => u64::from(u32::from_be_bytes(bytes.try_into().unwrap())),
        8 => u64::from_be_bytes(bytes.try_into().unwrap()),
        _ => unreachable!("chunk offsets are four or eight bytes"),
    }
}

pub(crate) fn is_patched(path: &Path) -> bool {
    File::open(path)
        .and_then(|mut file| find_patch(&mut file))
        .unwrap_or(false)
}

fn shift_chunk_offsets(index: &[u8], data_start: u64, shift: u64) -> io::Result<Vec<u8>> {
    change_child(index, *b"trak", |track| {
        change_child(track, *b"mdia", |media| {
            change_child(media, *b"minf", |info| {
                change_child(info, *b"stbl", |samples| {
                    rewrite(samples, |atom, out| {
                        if !matches!(&atom.kind, b"stco" | b"co64") {
                            out.extend_from_slice(atom.bytes);
                            return Ok(());
                        }
                        let offsets: Vec<u64> = table(atom.body, offset_width(atom.kind))?
                            .map(|row| {
                                let offset = read_offset(row);
                                if offset < data_start {
                                    return Err(invalid("MP4 chunk precedes media data"));
                                }
                                offset
                                    .checked_add(shift)
                                    .ok_or_else(|| invalid("MP4 chunk offset overflow"))
                            })
                            .collect::<io::Result<_>>()?;
                        let wide = atom.kind == *b"co64"
                            || offsets.iter().any(|&offset| u32::try_from(offset).is_err());
                        let mut body = atom.body[..8].to_vec();
                        for offset in offsets {
                            if wide {
                                body.extend_from_slice(&offset.to_be_bytes());
                            } else {
                                body.extend_from_slice(&(offset as u32).to_be_bytes());
                            }
                        }
                        write_atom(out, if wide { *b"co64" } else { *b"stco" }, &body)
                    })
                })
            })
        })
    })
}

fn shift_media(file: &mut File, start: u64, end: u64, shift: u64) -> io::Result<()> {
    if shift == 0 {
        return Ok(());
    }
    let new_end = end
        .checked_add(shift)
        .ok_or_else(|| invalid("MP4 size overflow"))?;
    file.set_len(new_end)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut cursor = end;
    // Copy backwards so moving bytes forward never overwrites unread media.
    while cursor > start {
        let count = (cursor - start).min(buffer.len() as u64) as usize;
        cursor -= count as u64;
        file.seek(SeekFrom::Start(cursor))?;
        file.read_exact(&mut buffer[..count])?;
        file.seek(SeekFrom::Start(cursor + shift))?;
        file.write_all(&buffer[..count])?;
    }
    Ok(())
}

fn place_index(index: Vec<u8>, start: u64, data_start: u64) -> io::Result<(Vec<u8>, u64)> {
    let room = data_start - start;
    let mut shift = 0;
    let mut adjusted = None;
    loop {
        let len = adjusted.as_ref().unwrap_or(&index).len() as u64 + 8;
        if len <= room + shift && matches!(room + shift - len, 0 | 8..) {
            return Ok((adjusted.unwrap_or(index), shift));
        }
        // A wider chunk table can grow the index again as a file crosses 4 GiB.
        // Widths can only increase, so each pass either fits or promotes a table.
        shift = len + 8 - room.min(len + 8);
        adjusted = Some(shift_chunk_offsets(&index, data_start, shift)?);
    }
}

pub(crate) fn patch_audio_tail(path: &Path) -> io::Result<bool> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_size = file.metadata()?.len();
    let mut start = 0;
    while start < file_size {
        let atom = FileAtom::read(&mut file, start, file_size)?;
        if atom.kind == *b"moov" {
            let original = atom.payload(&mut file, u64::MAX)?;
            let Some(index) = patch_index(&original, file_size + 8)? else {
                return Ok(false);
            };
            let mut data_start = atom.end;
            while data_start < file_size {
                let next = FileAtom::read(&mut file, data_start, file_size)?;
                if next.kind != *b"free" {
                    break;
                }
                data_start = next.end;
            }

            let (index, shift) = place_index(index, start, data_start)?;
            let mut top = Vec::new();
            write_atom(&mut top, *b"moov", &index)?;
            let padding = usize::try_from(data_start + shift - start - top.len() as u64)
                .map_err(|_| invalid("MP4 index padding too large"))?;
            if padding != 0 {
                write_atom(&mut top, *b"free", &vec![0; padding - 8])?;
            }
            let mut tail = Vec::new();
            write_atom(
                &mut tail,
                *b"mdat",
                &LFE_SILENCE.repeat(REPLACED_PACKETS as usize),
            )?;

            shift_media(&mut file, data_start, file_size, shift)?;
            file.seek(SeekFrom::Start(start))?;
            file.write_all(&top)?;
            file.seek(SeekFrom::End(0))?;
            file.write_all(&tail)?;
            return Ok(true);
        }
        if atom.kind == *b"mdat" {
            break;
        }
        start = atom.end;
    }
    Err(invalid("MP4 faststart index is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growing_front_index_rechecks_offsets_after_promoting_to_co64() {
        let wrap = |kind, body: &[u8]| {
            let mut bytes = Vec::new();
            write_atom(&mut bytes, kind, body).unwrap();
            bytes
        };
        let offset = u32::MAX - 5;
        let mut chunks = [0; 4].to_vec();
        chunks.extend_from_slice(&1u32.to_be_bytes());
        chunks.extend_from_slice(&offset.to_be_bytes());
        let index = [*b"stco", *b"stbl", *b"minf", *b"mdia", *b"trak"]
            .into_iter()
            .fold(chunks, |body, kind| wrap(kind, &body));

        let start = 32;
        let room = (index.len() + 4) as u64;
        let data_start = start + room;
        let (adjusted, shift) = place_index(index, start, data_start).unwrap();
        assert_eq!(shift, 16);

        let mut sample_table = &adjusted[..];
        for kind in [*b"trak", *b"mdia", *b"minf", *b"stbl"] {
            sample_table = child(sample_table, kind).unwrap();
        }
        let (kind, offsets) = chunk_offsets(sample_table).unwrap();
        assert_eq!(kind, *b"co64");
        let stored = table(offsets, 8).unwrap().next().unwrap();
        assert_eq!(read_offset(stored), u64::from(offset) + shift);
        assert_eq!(adjusted.len() as u64 + 16, room + shift);
    }
}
