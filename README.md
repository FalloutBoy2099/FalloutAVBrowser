# FalloutAVBrowser

A cross-platform (Windows, Linux, macOS) audio and video player for the files of Fallout 1/2, Arcanum and Temple of
Elemental Evil, written in Rust with egui + wgpu. It has the same interface as its sibling
[FalloutImageBrowser](../FalloutImageBrowser): folder tree on the left, the media in the centre, properties on the right.
Archives (`.dat`, `.zip`, `.7z`) are browsed like folders, and files are played straight from them without extracting.

## Formats

| Format | Content | Support |
|---|---|---|
| ACM | Interplay audio: Fallout 1/2 music, sounds and speech (also the `WAVC` variant) | play, convert |
| MVE | Interplay MVE video, 8-bit (Fallout cut-scenes), with its PCM / DPCM sound | play, convert |
| BIK | Bink 1 video (Arcanum, Temple of Elemental Evil), revisions f, g, h, i and k, with its audio (DCT and RDFT) | play, convert |
| MP3 | Arcanum / ToEE speech and music | play, convert |
| WAV | PCM 8, 16, 24 and 32-bit, 32-bit float | play, convert |

Not supported: 16-bit MVE, Bink revision `b`, Bink 2, `.SVE` subtitles.

Notes:

* **ACM channels.** The channel count in the header of Fallout's ACM files is not reliable (sounds and speech say
  "stereo" but are mono). *Automatic* mode (Navigation → *ACM channels*) plays a file as stereo when it lies in a `music`
  folder or when the signal is clearly interleaved stereo, and as mono otherwise; *Mono* and *Stereo* force it.
* **Audio output** uses the default output device. The audio clock drives the video, so picture and sound stay in sync.
* **Bink** pictures are decoded in software; seeking jumps to the nearest key frame and decodes forward from there.

## Archives

| Archive | Games | Notes |
|---|---|---|
| `.dat` (DAT1) | Fallout 1 | LZSS |
| `.dat` (DAT2) | Fallout 2 | zlib |
| `.dat`, `.patch0` … `.patch9` (Troika, `1TAD` footer) | Arcanum, Temple of Elemental Evil | stored or zlib; both layouts (full paths, or parent / child indices) are read |
| `.zip` | any | deflate, deflate64, LZMA |
| `.7z` | any | LZMA, LZMA2, deflate, BCJ filters; reading a file of a *solid* archive decodes its block from the start |

Encrypted ZIP / 7z archives, and 7z archives that use bzip2, PPMd or zstd, are not supported (their entries are listed but
cannot be read). Archive items can be copied, converted and checksummed like normal files, but not moved or renamed. Right-click
an item of an archive → *Extract…* copies it to a folder you choose.

## Interface

* **Bottom bar** (centred): *Folder tree* checkbox, *Current folder* button (Ctrl+L: edit the folder path to switch quickly),
  previous / next file, *Loop*, *Auto-next*, *Volume*, *Speed* (0.2x to 5x, pitch follows speed), *Properties* checkbox.
  *Auto-next* plays the following file when one ends; combined with *Loop* it loops over the whole folder, otherwise
  *Loop* repeats the current file.
* **Player bar** (bottom of the centre area): play / pause, stop, position / duration and a progress slider. Click on the
  waveform of an audio file, or on the slider, to seek. Click on a video to play or pause; the mouse wheel zooms, drag pans,
  double-click toggles fit / 100 %.
* **Left panel**: a filter box (file names of the current folder), a bar of up to 8 quick-access buttons, and the tree.
  The first three buttons are reserved (Desktop, Documents, Downloads, with their Linux and macOS equivalents); the five
  others are your favourites, shown with the first letter of the folder name. Right-click a folder → *Favorites* adds it
  (greyed out when the five places are taken); right-click a favourite → *Remove from favorites*.
  Typing letters or digits while the mouse is over the tree jumps to the next name that starts with them.
  Right-click menu: *Expand all*, *Collapse others*, *Refresh* (F5), *Favorites*, *Open in Explorer*, *Extract…*.
* **Right panel** (properties): file (name, folder, size, SHA-256, modified, created, format), audio (duration, sample
  rate, channels, samples), video (size, frame rate, frames). Icon buttons at the bottom: copy the path | copy to…,
  move to…, rename… | convert file… | copy all the information as text, JSON or XML, copy the SHA-256.
  Copy the SHA-256 of a file, then open another one: the panel tells you whether the two are identical; the cross stops the
  comparison.
* **Convert file…** (File menu, Ctrl+E): a modal window. Audio: WAV 8-bit, 16-bit or 32-bit float, or raw 16-bit PCM, with
  sample rate, channels (mono / stereo), peak normalisation to -1 dB and a start / end range. Video: the current frame as
  PNG, or a PNG sequence (first / last frame, one frame out of N), or the audio track. Any file can also be copied as is
  (which extracts it from an archive). Output folder = source folder, last folder used or a folder you pick; a progress bar
  and a cancel button; no existing file is ever overwritten (`name (2).wav`).
* **Menus**: *File* (open folder / file / archive, refresh, convert, open with the default program, exit), *View* (panels,
  full screen, waveform colour, remembered window size and position), *Navigation* (current folder, file sort order, mouse
  side buttons = previous / next file or play-pause / next file, ACM channels), *Language* and *Help* (associate audio /
  video files with the program, About).
* Drag and drop of a file (its folder becomes the current folder), a folder or an archive.
* 16 languages (French, English, Spanish, Italian, Portuguese, Polish, Danish, Finnish, Greek, Hungarian, Dutch, Romanian,
  Swedish, Czech, Chinese, Russian). The default language is the system's (`FAV_LANG=xx` forces one). Translations are in
  `assets/lang/*.txt` (every language has every string; the strings added by this project were translated without a native speaker's review, corrections are welcome).

Shortcuts: Space play / pause, ← / → seek 5 s, ↑ / ↓ (or Page Up / Page Down, Backspace) previous / next file, Home / End first
/ last file, `+` `-` zoom, Enter 100 %, `F` fit, `T` folder tree, `I` properties, `F5` refresh, `F11` full screen, `F1` about,
Ctrl+N open folder, Ctrl+O open file, Ctrl+D open archive, Ctrl+L current folder, Ctrl+E convert, Ctrl+Enter open with the
default program, Ctrl+Q quit.

## Command line

Run without arguments, or with a file, folder or archive, it opens the window (and detaches from the terminal on Windows).
Options that start with `--` run without a window and return an exit code (0 success, 1 failure):

```
fallout_av_browser [path]                                  # opens the GUI (file, folder or archive)
fallout_av_browser --info <file>                           # format, duration, sample rate, channels, video size
fallout_av_browser --ls <archive> [filter]                 # list the files of an archive (.dat, .zip, .7z)
fallout_av_browser --wav <file> <out.wav>                  # audio of a file as 16-bit WAV
fallout_av_browser --frame <file.mve|bik> <n> <out.png>    # frame n (from 0) as PNG
fallout_av_browser --convert <file> <wav16|wav8|f32|raw|frame|seq|orig> <folder> [rate] [channels] [normalize 0|1]
```

A file inside an archive is addressed like a normal path: `D:\Fallout2\master.dat\art\cuts\afailed.mve`,
`"C:\Games\Arcanum\modules\Arcanum.dat\sound\speech\slide\v8_m.mp3"`.

## Building

```
cargo run --release -- [file-or-folder]
```

`.\publish.ps1` (PowerShell 7) builds the release binaries for Windows, Linux and macOS (x64 and arm64) and writes one zip per
platform into `publish\`; run `Get-Help .\publish.ps1 -Full` for the prerequisites of the cross-compiled targets.
On Linux (Debian, Ubuntu…) and macOS, `./publish.sh` does the same; `./publish.sh --help` lists the options and the one-time
setup. Linux builds need the ALSA development files (`libasound2-dev` on Debian / Ubuntu) for the audio output.

## Credits and license

The code of this project is released under the [MIT License](LICENSE).

* The ACM decoder is a port of *libacm* by Marko Kreen (ISC license).
* The Bink decoder is a port of FFmpeg's Bink video and audio decoders (Konstantin Shishkov, Peter Ross; LGPL 2.1 or
  later). A distributed binary that includes it is therefore subject to the LGPL's terms for that part.
* The MVE and Interplay DPCM decoders follow the format descriptions of the multimedia wiki and the Fallout wiki.
* Troika archives follow the description of the ReWiki *Temple of Elemental Evil DAT* page.
