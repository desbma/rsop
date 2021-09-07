# rsop

[![Build status](https://github.com/desbma/rsop/actions/workflows/ci.yml/badge.svg)](https://github.com/desbma/rsop/actions)
[![License](https://img.shields.io/github/license/desbma/rsop.svg?style=flat)](https://github.com/desbma/rsop/blob/master/LICENSE)

Simple, fast & configurable tool to open and preview files.

Ideal for use with terminal file managers (ranger, lf, nnn, etc.). Can also be used as a general alternative to `xdg-open` and its various clones.

If you spend most time in a terminal, and are unsatisfied by current solutions to associate file types with handler programs, this tool may be for you.

**This tool is a work in progress, not ready to be used yet.**

## Features

- Start program to view/edit file according to extension or MIME type
- Provides two tools (in a single binary):
  - `rso`: open file (`xdg-open` replacement)
  - `rsp`: preview files in terminal, to be used for example in terminal file managers or [`fzf`](https://github.com/junegunn/fzf) preview panel
- Supports opening and previewing from data piped on stdin (very handy for advanced shell scripting, see [below](#show-me-some-cool-stuff-rsop-can-do))
- Supports chainable filters to preprocess data (for example to transparently handle `.log.xz` files)
- Simple config file (no regex or funky conditionals) to describe file formats, handlers, and associate both

Compared to other `xdg-open` alternatives:

- `rsop` is consistent and accurate, unlike say [ranger](https://github.com/ranger/ranger/issues/1804)
- `rsop` does not rely on `.desktop` files (see section [Why no .desktop support](#why-no-desktop-support))
- `rsop` does opening and previewing with a single self contained tool and config file
- `rsop` is not tied to a file manager or a runtime environment, you only need the `rsop` binary and your config file and can use it in interactive terminal sessions, file managers, `fzf` invocations...
- `rsop` is taylored for terminal users (especially the preview feature)
- `rsop` is very fast (see [performance](#performance) section)

## Installation

You need a Rust build environment for example from [rustup](https://rustup.rs/).

```
cargo build --release
strip --strip-all target/release/rsop
install -Dm 755 -t /usr/local/bin target/release/rsop
ln -rsv /usr/local/bin/rs{op,p}
ln -rsv /usr/local/bin/rs{op,o}
```

## Configuration

When first started, `rsop` will create a minimal configuration file usually in `~/.config/rsop/config.toml`.

See comments and example in that file to set up file types and handlers for your needs.

A more advanced example configuration file is also available [here](./config/config.toml.example).

### Usage with ranger

**Warning: because ranger is build on Python's old ncurses version, the preview panel only supports 8bit colors (see https://github.com/ranger/ranger/issues/690#issuecomment-255590479), so if the output seems wrong you may need to tweak handlers to generate 8bit colors instead of 24.**

In `rifle.conf`:

    = rso "$@"

In `scope.sh`:

    RSOP_MODE=open COLUMNS="$2" LINES="$3" exec rsop "$1"

### Usage with lf

Add in `lfrc`:

    set filesep "\n"
    set ifs "\n"
    set previewer ~/.config/lf/preview
    cmd open ${{
       for f in ${fx[@]}; do rso "${f}"; done
       lf -remote "send $id redraw"
    }}

And create `~/.config/lf/preview` with:

    #!/bin/sh
    RSOP_MODE=preview COLUMNS="$2" LINES="$3" exec rsop "$1"

## Show me some cool stuff `rsop` can do

- Simple file explorer with fuzzy searching, using [fd](https://github.com/sharkdp/fd) and [fzf](https://github.com/junegunn/fzf), using `rso` to preview files and `rsp` to open them:

```
fd . | fzf --preview='rsp {}' | xargs rso
```

_TODO image_

- Preview files inside an archive, **without decompressing it entirely**, select one and open it (uses [`bstdtar`](https://www.libarchive.org/), [`fzf`](https://github.com/junegunn/fzf) and `rso`/`rsp`):

```
# preview archive
pa() {
    local -r archive="${1:?}"
    bsdtar -tf "${archive}" |
        grep -v '/$' |
        fzf -m --preview="bsdtar -xOf \"${archive}\" {} | rsp" |
        xargs -r bsdtar -xOf "${archive}" |
        rso
}
```

_TODO image_

## Performance

`rsop` is quite fast. In practice it rarely matters because choosing with which program to open or preview files is usually so quick it is not perceptible. However performance can matter if for example you are decompressing a huge `tar.gz` archive to preview its content.
To help with that, `rsop` uses the [`splice` system call](https://man7.org/linux/man-pages/man2/splice.2.html) if available on your platform. In the `.tar.gz` example this allows decompressing data with `gzip` or `pigz` and passing it to tar (or whatever you have configured to handle `application/x-tar` MIME type), **without wasting time to copy data in user space** between the two programs.

Other stuff `rsop` does to remain quick:

- it is written in Rust (forgetting the RiiR memes, this avoid the 20-50ms startup time of for example Python interpreters)
- it uses hashtables to search for handlers from MIME types or extensions in constant time
- it uses the great [tree_magic_mini crate](https://crates.io/crates/tree_magic_mini) for fast MIME identification

## FAQ

### Why no [`.desktop`](https://specifications.freedesktop.org/desktop-entry-spec/latest/) support?

- `.desktop` do not provide a _preview_ action separate from _open_.
- One may need to pipe several programs to get to desired behavior, `.desktop` files does not help with this.
- Many programs do not ship one, especially command line tools, so this would be incomplete anyway.
- On a philosophical level, with `.desktop` files, the program's author (or packager) decides which MIME types to support, and which arguments to pass to the program. This is a wrong paradidm, as this is fundamentally a user's decision.

### What does `rsop` stands for?

"**R**eally **S**imple **O**pener/**P**reviewer" or "**R**eliable **S**imple **O**pener/**P**reviewer" or "**R**u**S**t **O**pener/**P**reviewer"

I haven't really decided yet...

## License

[MIT](./LICENSE)
