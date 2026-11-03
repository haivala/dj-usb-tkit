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

## On a USB

Each USB's **Audio formats** row (the USB view, see
[USB_CONNECT.md](USB_CONNECT.md#usb-audio-formats)) says which formats export puts on it, in your
order of preference: set it for the players that stick goes to. Export picks each track's file
by it:

- Of the track's own file and its up-to-date transcodes, the one in the first ticked format goes
  on the USB. With FLAC and MP3 both ticked and MP3 first, a track with an MP3 transcode goes as
  the MP3; one without goes as its FLAC.
- None in a ticked format: a lossless track gets a transcode made by the export, in the first
  ticked format the app makes (AIFF, WAV or MP3). The progress bar shows "Converting" first. The
  new transcode stays in the library like one made with Convert…, so the next export reuses it.
  An out-of-date transcode is made again the same way.
- An MP3 or AAC track in a format the USB doesn't allow can't be converted (the app only converts
  from lossless). The export is refused before anything is written, and the message names the
  tracks.
- A file already on the USB is never removed by the export (only a mirror export prunes, as
  rekordbox's does). A track exported as its FLAC before and as its MP3 now is on the USB both
  ways: two USB tracks, each with its own masterContentId, both in the playlist.
- The app playlist's title row says ahead of time what the export will do: "2 tracks will be
  converted to MP3 320 on export", or that tracks can't go on the USB.
- In the app playlist, the FMT column shows the format each track goes on the selected USB in:
  its transcode's (MP3, with the FLAC no longer listed under it), or the one export will convert
  it to (dashed). The tooltip names the track's own file as the source. A track that can't go
  shows its own format with a warning.

![An app playlist for a USB that takes only MP3 and AAC: two FLAC tracks go as their MP3
transcodes, two get converted on export, and one is an MP3 already](assets/transcode-playlist.png)

The USB's databases list the transcode's own file: its format, size, bit rate and length.

### Timing

An MP3's audio starts a little later than its source: the encoder and the player's decoder each
add a short delay, about 25 ms together. A cue at 1:00.000 on the FLAC is at 1:00.025 in the
MP3. Export moves the cues, the first beat and the whole beat grid that much later on the USB,
so they land on the same sound. AIFF and WAV start at the same sample, so nothing moves.

The waveforms on the USB are drawn from the transcode itself. Song structure (phrases) and
everything else in the track's analysis stays as it is.

Going the other way, the app takes the delay off again: a cue edited on the USB's MP3 is saved
on the library track that much earlier, and cues imported from the USB's MP3 land on the
track's own timeline. The app recognises the USB's file as a transcode by its format and exact
size.

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
source folder is removed) takes its transcodes with it. A transcode's
analysis files (below) go with it.

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
  plays the file. rekordbox plays MP3s this way (checked against its
  waveforms). Export moves cues and the grid by it, rounded to whole
  milliseconds (`service/transcode_export.rs`).
- Each transcode has its own analysis bundle in the app data
  (`transcode_analysis/<hash of the transcode's path>/`): `TRANSCODE.*`, drawn
  from the transcode file when it is made (waveforms, and the MP3 seek index
  `PVBR`), and `ANLZ0000.*`, rebuilt at each export from the track's own
  bundle with those waveforms and seek index and its beat grid moved
  (`anlz::transcode_bundle_file`). A transcode made before export used them
  gets `TRANSCODE.*` at its first export.
- MP3s are written without the Info/LAME header frame. rekordbox plays the
  header frame LAME writes as one more frame of audio, which would move the
  file's timeline by 1152 samples; a CBR file needs no header frame.
- Encoding: `backend/src/audio_encode.rs` (symphonia decoding, a PCM
  AIFF/WAV writer, LAME through `mp3lame-encoder`, tags through lofty).
