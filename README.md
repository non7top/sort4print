# sort4print

Go through a folder of phone photos, pick the ones worth printing, and cut each
one to a print proportion with the place and date burned into a corner.

One Windows `.exe`. No installer, no runtime to install, no network access.

## What it does

- **Pick.** Walk the folder with `←` / `→`, tick with `Space`. A picked photo is
  unmistakable: the space around it turns green, the crop window turns green, and
  it is labelled in words as well as colour. The switch in the toolbar makes
  Next/Previous walk **all** photos, only the **selected** ones, or only the
  **unselected** ones — that last is the second pass, once the obvious keepers
  are done and only the undecided ones are worth looking at.
- **Crop.** The cut-out window starts as the largest centred window of your
  print proportion that fits the photo. Drag to move it, drag a handle to
  resize it — the proportions stay locked — and scroll to zoom. Edges and the
  centre lines are magnetic, so a flush edge or an exact fit lands by hand; the
  centre pulls twice as far as an edge and wins when both are in reach, since
  dead centre is the framing most often wanted and the most obvious when it is
  a few pixels out.
  The window may reach past the photo, and whatever falls outside prints as the
  border colour; it will not grow past the point where the photo fits inside it
  entirely, since past there every extra pixel is border and none is picture.
  The window turns green once the photo is picked.
- **Caption.** City, country, your own name for the spot, and the date, in a
  corner. City and country come from the GPS fix matched against a database
  built into the exe, so it works with no network; both can be overridden per
  photo or for the whole folder.
- **Notes stay put.** Ticks, crop windows, place overrides and descriptions are
  written to `sort4print-notes.ini` in the folder of photos, so closing the
  program half way through a holiday does not throw the work away. Deleting
  that file discards those choices and nothing else.
- **Export.** The crop is taken from the original at full resolution and never
  resampled, written as JPEG at quality 100 into a folder you choose.

Everything is decoded on background threads: the photos on either side of the
one you are looking at are already loaded, rotated, measured and geocoded before
you get to them.

## Speed

Three things stop a folder of eleven thousand photos being unusable.

**Filmstrip rows use the thumbnail the camera already put in the file.** Reading
a preview that is already there costs a fraction of a millisecond; decoding the
twelve-megapixel original to produce the same postage stamp costs a hundred of
them and a core to do it on. It is only accepted when its shape matches the
photo's, since a few cameras pad theirs and a stretched thumbnail is worse than a
slow one.

**Decoded images are kept on disk.** Upright, already shrunk, as small JPEGs in
the per-user cache directory — a fraction of the size of the original and a
fraction of the time to decode. The key includes the photo's size and
modification time, so an edited photo simply gets a new key and a stale entry can
never be served. A budget is enforced by discarding least recently used entries.
Everything in there is derived: deleting the directory costs time, never work.

**Previews always beat thumbnails to the workers.** They are held in separate
queues, and the thumbnail backlog is capped — scrolling a long list would
otherwise queue thousands of jobs and leave the photo you are actually looking at
waiting behind them. Rows still on screen ask again next frame, so dropping the
oldest request costs nothing.

**Read all** in the filmstrip goes through the whole folder once, filling the
cache, so that browsing afterwards waits for nothing. It runs in the background,
hands work out only as earlier jobs finish so it never gets in your way, shows
progress, and can be stopped.

## Keyboard and mouse

| | |
|---|---|
| `←` `→` | previous / next photo |
| `Space` | pick or unpick this photo |
| `Enter` | export everything picked |
| `Alt`+arrows | nudge the crop window (hold `Shift` for bigger steps) |
| drag | move the crop window |
| drag a handle | resize it, proportions locked |
| scroll | zoom the crop window |
| `Ctrl`+scroll | zoom the view, to look closely — changes nothing exported |
| middle-drag | pan the view |

## Building

**CI builds the exe.** Every pull request runs the core test suite on Linux,
runs it again on Windows, builds the release binary there, and posts a comment
on the PR with a download link to the artifact. That is the intended way to get
a build — no local Rust toolchain required.

A disposable container is available for local work, needing nothing on the host
but Docker. It runs as your own uid/gid, so anything it writes into the working
directory belongs to you, and it leaves nothing behind except named cache
volumes.

```sh
./x test          # core test suite, native Linux
./x check         # type-check the whole workspace against the Windows target
./x build         # cross-compile a release build -> dist/sort4print.exe
```

`./x <anything else>` runs that command inside the container. Be warned that the
first `check` or `build` compiles the whole dependency tree for Windows, which
is a long, CPU-hungry job; the test suite is far cheaper and covers everything
except the widget code.

### Refreshing the city database

`assets/cities.bin` is committed, so a normal build needs no network. To rebuild
it from a newer GeoNames dump:

```sh
curl -o data/cities15000.zip https://download.geonames.org/export/dump/cities15000.zip
curl -o data/countryInfo.txt https://download.geonames.org/export/dump/countryInfo.txt
unzip -o data/cities15000.zip -d data
./x pack-cities
```

## What is remembered

Two files, because two different things are being remembered.

**Your settings** — print size, caption text and font, date format, output
folder, prefetch — go in `sort4print.ini` with the program. They belong to you,
not to any particular batch of photos.

**Your decisions about these photos** — which are ticked, how each is cropped,
place overrides, descriptions, and which photo you were looking at — go in
`sort4print-notes.ini` in the folder of photos. They belong to the pictures, so
copying that folder elsewhere takes them along, and sorting a second folder does
not disturb the first. Reopening a folder returns you to the photo you left off
at; if that file has since gone, you land near where it was.

Only choices actually made are recorded. Opening a photo gives it the default
centred window, and that is not written down — it is recomputed identically next
time — so the file's size follows the work done rather than the number of photos
in the folder. A few hundred decisions is tens of kilobytes whether the folder
holds two hundred pictures or eleven thousand.

The notes are never overwritten in place: a new copy is written alongside,
flushed to disk, and moved into position, keeping the previous one as
`sort4print-notes.ini.bak`. Each file ends with an `# end` marker.

Which copy is read follows one rule — *never discard what you cannot read*:

- The live file finished being written → it wins, even if the backup holds more.
  Deliberately unpicking everything has to be believed.
- The live file was cut short but the backup is intact → the backup wins. This is
  the case the backup exists for.
- Neither carries a marker (both predate it, or both were cut short) → whichever
  knows about more photos wins.
- A file is there with bytes in it but nothing can be made of it → nothing is
  written for that folder at all, and the status bar says so. An unreadable file
  is never replaced by an empty one.

A missing marker means "suspect", never "worthless": it is also exactly what a
file from an older version looks like, and treating that as corruption is how a
folder's worth of decisions was thrown away once.

Both are written as soon as you let go of a control, not only on a clean exit,
so killing the program loses at most the gesture in progress. The one thing not
remembered is which photos you have already exported: that is about this
session, and the files themselves are the record.

## Settings

Settings live in `sort4print.ini`, next to the exe when that folder is writable
(so the program is portable on a stick) and in `%APPDATA%\sort4print\`
otherwise. The About tab shows the path in use.

The file is written with a comment above every key explaining it. Everything the
panels can set is in there, plus one thing they cannot: extra date locales.

### Date formats and languages

English and Russian month names are compiled in. Any other language is a section
in the ini:

```ini
[date]
locale = de
format = {d} {MMMMo} {yyyy}

[locale.de]
label = Deutsch
months_short = Jan,Feb,Mär,Apr,Mai,Jun,Jul,Aug,Sep,Okt,Nov,Dez
months_long = Januar,Februar,März,April,Mai,Juni,Juli,August,September,Oktober,November,Dezember
months_long_of = Januar,Februar,März,April,Mai,Juni,Juli,August,September,Oktober,November,Dezember
```

`months_long_of` is the form used after a day number. It matters in languages
that decline: Russian wants `Октябрь 2025` standing alone but `5 октября 2025`
after a number. Where a language does not distinguish them, repeat
`months_long`; if the key is missing it is copied automatically.

Format tokens:

| token | means | example |
|---|---|---|
| `{yyyy}` `{yy}` | year | 2025, 25 |
| `{MMMM}` | month, long, standing alone | October |
| `{MMMMo}` | month, long, after a day number | октября |
| `{MMM}` | month, short | Oct |
| `{MM}` `{M}` | month, number | 10 |
| `{dd}` `{d}` | day | 05, 5 |
| `{HH}` `{mm}` | time | 09, 07 |

The default is `{MMM} '{yy}` → `Oct '25`.

### The caption font

The brief asked for "Colibri Black". There is no such font: the Microsoft face
is spelled **Calibri**, and it ships Light, Regular and Bold — there is no Black
weight to select. The default is therefore Calibri Bold, its heaviest genuine
cut, loaded from the system font folder. The panel lists every installed font
with a filter box and a style list, and previews the result live with the same
renderer that writes the file. Picking a font that genuinely has a Black weight
(Arial Black, Archivo Black, …) gets you one.

## If it does not start

Every run writes `sort4print.log` beside the exe (or in `%TEMP%` when that
folder is read-only), and anything fatal also raises a dialog pointing at it.
The log names the rendering backend in use.

Two backends are compiled in and tried in turn: **glow**, which is OpenGL, then
**wgpu**, which is Direct3D 12 on Windows. Neither works everywhere, and an
Optimus laptop is the awkward case where the two halves of the same machine
disagree — OpenGL cannot get a context on the Intel side, and wgpu's Direct3D
path fails on the NVIDIA side with "Invalid surface".

OpenGL is tried first because of *how* it fails rather than how often: it
returns an error and lets the next backend be tried, where wgpu panics from
inside the driver. That panic is caught, but unwinding out of a half-started
graphics stack is not something to do by choice. Carrying both backends is what
the exe's size mostly buys.

## Known limits

- **HEIC is not read.** iPhones shoot it by default. Decoding it needs libheif,
  a large C library that would end the single-file, no-install property. Set the
  phone to "Most Compatible", or convert first.
- Cities come from a list of settlements above ~15 000 inhabitants, so a photo
  taken in a village resolves to the nearest sizeable town. The panel offers the
  eight nearest alternatives, a name search, and a free-text override.
- Saving settings rewrites the ini from the current state, so hand-written
  comments in it are not preserved.

## Layout

```
core/    all the logic — crop maths, EXIF, geocoding, fonts, caption
         rendering, export. Builds and tests natively on Linux with no
         windowing system.
app/     the egui interface and the background loaders.
tools/   the build-time packer for the city database.
```

The split exists so the parts worth testing can be tested in the same container
that cross-compiles the exe, without a display.

## Attribution

City and country data derived from [GeoNames](https://www.geonames.org/),
licensed CC BY 4.0.
