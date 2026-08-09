# sort4print

Go through a folder of phone photos, pick the ones worth printing, and cut each
one to a print proportion with the place and date burned into a corner.

One Windows `.exe`. No installer, no runtime to install, no network access.

## What it does

- **Pick.** Walk the folder with `←` / `→`, tick with `Space`. The count of
  picked photos is always on screen, and a switch makes Next/Previous walk
  either every photo or only the ones you ticked.
- **Crop.** The cut-out window starts as the largest centred window of your
  print proportion that fits the photo. Drag to move it, drag a handle to
  resize it — the proportions stay locked — and scroll to zoom. The window is
  allowed to reach past the edge of the photo; whatever falls outside prints as
  the border colour.
- **Caption.** City, country and date in a corner, from EXIF. The city comes
  from the GPS fix matched against a city database built into the exe, so it
  works with no network; you can override it per photo or for the whole folder.
- **Export.** The crop is taken from the original at full resolution and never
  resampled, written as JPEG at quality 100 into a folder you choose.

Everything is decoded on background threads: the photos on either side of the
one you are looking at are already loaded, rotated, measured and geocoded before
you get to them.

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
