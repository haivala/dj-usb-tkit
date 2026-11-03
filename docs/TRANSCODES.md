# Transcodes

Older CDJs can't play FLAC or ALAC. **Convert…** in the selection bar (Library
and app playlists) makes another file of each selected track in AIFF, WAV or
MP3 320 kbps.

## One track, more files

A transcode is not a new track. The library keeps one row per track, and the
transcode is an extra file of that track:

- The FMT column shows it on a smaller line under the track's own format
  (`FLAC`, with `MP3` below). Its tooltip gives the format and file size.
- Cues, memory cues, the beat grid, key, BPM, tag edits and playlist entries
  all stay on the track. The transcode has none of its own, so nothing can get
  out of step.
- If the source file changes after the transcode was made, the line turns to
  the warning colour and the tooltip says it is out of date. Convert again to
  replace it.

USB export does not use transcodes yet. Each USB's **Player formats** row (the USB view, see
[USB_CONNECT.md](USB_CONNECT.md#player-formats)) says which formats its players read; an app
playlist's title row counts the tracks in a format the connected USB doesn't allow, and how many
of them have a transcode in one it does.

## Which tracks get one

The app decides, so selecting a mixed set is fine:

- Only lossless files are converted: FLAC, ALAC (`.m4a` files holding ALAC,
  not AAC), WAV and AIFF.
- A file already in the chosen format is skipped (a WAV is not converted to
  WAV).
- A track that already has an up-to-date transcode in that format is skipped.
- A track whose file can't be found is skipped.

Skipped and failed tracks are listed in the Event Log. A file the scan lists
but hasn't indexed yet is indexed first, as adding it to a playlist does.

## Formats

- **AIFF** and **WAV** keep the source's sample rate and bit depth (16 or 24
  bit), sample for sample. WAV is always written as plain PCM.
- **MP3** is always 320 kbps CBR. A source above 48 kHz is brought down to
  44.1 or 48 kHz. There is no VBR option: a CBR file seeks exactly on every
  player, and its size follows from the track's length.

Each transcode gets the track's title, artist, album, genre, year, track
number, BPM, key and cover as the app has them, so edits made in the app are
in the file.

## Where the files go

Settings → Transcodes shows the folder. By default it is `transcodes` inside
the app's data folder; **Change…** picks another one, **Use default** goes
back.

- The folder can't be inside a source folder. The scan would otherwise list
  the transcodes as tracks. Keep it outside the folders other music apps scan
  too, so they don't list the same music twice.
- A transcode mirrors the source's path under its source folder:
  `Music/Crate A/set.flac` becomes `<folder>/Crate A/set.mp3`. A track outside
  every source folder (imported from rekordbox, Mixxx or a USB) goes to
  `<folder>/other/<name>-<id>.<ext>`.
- Changing the folder only affects new transcodes. Existing ones stay where
  they are.

## Removing

**Convert… → Remove transcodes** deletes the selected tracks' transcode files.
A track removed from the library (its file is gone at the next scan, or its
source folder is removed) takes its transcodes with it.

## Converting runs as a job

Converting shows in the progress bar, and its **×** cancels it. Tracks already
converted keep their files; a half-written file is deleted.

## Technical notes

- Table `track_transcodes` (local DB): one row per track and format, with the
  file path and size, the audio facts, and the source file's size and modified
  time when the transcode was made (the out-of-date check).
- `Track.transcodes` is filled on every track read (`service/transcode.rs`,
  `fill_track_transcodes`).
- The MP3's start offset is stored (`offset_ms`): an MP3 starts later than its
  source by the encoder and decoder delay, as a player that doesn't trim it
  plays the file. It is computed from the LAME header the encoder writes, for
  when export uses transcodes.
- Encoding: `backend/src/audio_encode.rs` (symphonia decoding, a PCM
  AIFF/WAV writer, LAME through `mp3lame-encoder`, tags through lofty).
