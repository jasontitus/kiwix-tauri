#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Regenerate the reader-path ZIM fixtures.

These fixtures cover archive shapes that stock Kiwix downloads rarely have:

* no_main_page.zim  - one article, no MainPage metadata. libzim's
  getMainEntry() throws EntryNotFound("No main page"); the reader must fall
  back to a random article instead of failing to open the archive.
* no_articles.zim   - media only, no front article. There is nothing readable
  to open, so "Random article" must stay a quiet, recorded notice.
* large_article.zim - a main article whose HTML is larger than the reader's
  16 MiB single-read limit. It must stream with the bridge injected from the
  first prefix instead of returning 413.
* media.zim         - a main article with a media entry larger than the
  server's 1 MiB streaming range, so byte-range requests, mid-file seeks and
  concurrent reads are exercised against the real archive reader instead of a
  whole-entry buffer. The payload repeats, so the committed fixture stays
  small while the item size stays large.
* zimit_warc.zim    - a nested Zimit2-shaped replay with Wombat scripts and a
  `.warc` entry, proving the shared server serves the replay scripts and
  `application/warc` assets without a service worker.
* preview.zim       - redirected and encoded thumbnail paths, safe local bases,
  external sources, unsupported/oversized images and text-only fallback, plus
  a hostile deeply nested page and a Wikipedia-sized article whose excerpt is
  read from the 256 KiB preview prefix.

Usage: python3 scripts/make-reader-fixtures.py [output-dir]
Requires the libzim Python bindings (libzim.writer) and libzim >= 8.
"""
import base64
import os
import sys

from libzim.writer import Compression, Creator, Hint, Item, StringProvider

# The media fixture entry must exceed the server's 1 MiB streaming range.
MEDIA_SIZE = 3 * 1024 * 1024


class TextItem(Item):
    def __init__(self, path, title, mimetype, content, front_article=False, compress=False):
        super().__init__()
        self._path = path
        self._title = title
        self._mimetype = mimetype
        self._content = content
        self._front_article = front_article
        self._compress = compress

    def get_path(self):
        return self._path

    def get_title(self):
        return self._title

    def get_mimetype(self):
        return self._mimetype

    def get_contentprovider(self):
        return StringProvider(self._content)

    def get_hints(self):
        # libzim only compresses an oversized item into its own cluster when the
        # item asks for it; without this a large fixture is stored verbatim.
        return {Hint.FRONT_ARTICLE: self._front_article, Hint.COMPRESS: self._compress}


def build(name, items, main_path=None, redirects=(), metadata=()):
    if os.path.exists(name):
        os.remove(name)
    with Creator(name).config_compression(Compression.zstd) as creator:
        if main_path:
            creator.set_mainpath(main_path)
        for key, value in (
            ("Title", "Reader fixture"),
            ("Language", "eng"),
            ("Creator", "kiwix-workbench tests"),
            ("Publisher", "kiwix-workbench tests"),
            ("Date", "2026-01-01"),
            ("Description", "Reader-path regression fixture"),
            *metadata,
        ):
            creator.add_metadata(key, value)
        for item in items:
            creator.add_item(item)
        for path, target in redirects:
            creator.add_redirection(path, path, target, {})


def main():
    output = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "src-tauri",
        "tests",
        "fixtures",
    )
    os.makedirs(output, exist_ok=True)

    png = base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAACgAAAAYCAIAAAAH5iiXAAAAK0lEQVR4nGMUqNjCMBCAaUBsHbV41OJRi0ctHrV41OJRi0ctHrV41GKaAAAZ/QFsMC3okAAAAABJRU5ErkJggg=="
    )
    preview_pages = {
        "index.html": '<a href="redirect.html">Image article</a><a href="external.html">External image</a>',
        "A/nested/article.html": '<p>Offline thumbnail excerpt.</p><img src="../../I/photo%20%2523.png">',
        "base.html": '<base href="I/"><p>Local base.</p><img src="photo%20%2523.png">',
        "external.html": '<p>External sources stay offline.</p><img src="https://example.org/photo.png"><img src="//example.org/photo.png">',
        "external-base.html": '<base href="https://example.org/"><p>External base stays offline.</p><img src="I/photo%20%2523.png">',
        "fallback.html": '<p>Later valid image.</p><img src="missing.png"><img src="I/vector.svg"><img src="I/large.png"><img src="I/photo%20%2523.png">',
        "lazy.html": '<p>Lazy image.</p><img src="data:image/gif;base64,R0lG" data-src="I/photo%20%2523.png">',
        "text.html": '<p>Text without artwork.</p>',
        # About 250 KB of nesting: unbounded, html5ever takes seconds on it.
        "deep.html": "<div>" * 50000,
        # More than the 256 KiB preview prefix of Wikipedia-shaped markup.
        "large.html": '<div class="mw-body"><div class="mw-parser-output"><table class="infobox"><tbody>'
        + '<tr><th scope="row">Field</th><td><div class="plainlist"><ul><li><a href="x">Value</a></li></ul></div></td></tr>' * 400
        + '</tbody></table><p>Large article <b>lead</b> paragraph.</p>'
        + '<p>Body with <a href="x" title="x">links</a> and <i>a <a href="y">citation</a></i><sup class="reference"><a href="#n">[1]</a></sup>.</p>' * 3000
        + '</div></div>',
    }
    build(
        os.path.join(output, "preview.zim"),
        [TextItem(path, path, "text/html", f"<!doctype html><html><body>{body}</body></html>", True,
                  compress=path in ("deep.html", "large.html"))
         for path, body in preview_pages.items()] + [
            TextItem("I/photo %23.png", "Photo", "image/png", png),
            TextItem("I/vector.svg", "Vector", "image/svg+xml", '<svg xmlns="http://www.w3.org/2000/svg"/>'),
            TextItem("I/large.png", "Oversized", "image/png", png + bytes(256 * 1024)),
        ],
        main_path="index.html",
        redirects=[("redirect.html", "A/nested/article.html")],
    )

    build(
        os.path.join(output, "no_main_page.zim"),
        [
            TextItem(
                "A/Only.html",
                "Only article",
                "text/html",
                "<!doctype html><html><head><title>Only article</title></head>"
                "<body><h1>Only article</h1></body></html>",
                front_article=True,
            )
        ],
    )

    build(
        os.path.join(output, "no_articles.zim"),
        [
            TextItem(
                "M/photo.jpg",
                "Photo",
                "image/jpeg",
                b"\xff\xd8\xff\xe0fixture-jpeg",
            )
        ],
    )

    # Repetitive text keeps the compressed fixture small while the item size
    # stays above the 16 MiB read limit.
    paragraph = "<p>" + ("reader path fixture text " * 64) + "</p>"
    body = paragraph * ((17 * 1024 * 1024) // len(paragraph) + 1)
    large = (
        "<!doctype html><html><head><title>Large article</title></head>"
        f"<body><h1>Large article</h1>{body}</body></html>"
    )
    assert len(large.encode("utf-8")) > 16 * 1024 * 1024
    build(
        os.path.join(output, "large_article.zim"),
        [TextItem("Large.html", "Large article", "text/html", large, True)],
        main_path="Large.html",
    )

    # Three 1 MiB streaming ranges, so a range request spans chunk boundaries.
    # The pattern repeats every 4096 bytes to keep the fixture compressed while
    # the entry the server serves stays 3 MiB.
    pattern = bytes(range(256)) * 16
    media = pattern * (MEDIA_SIZE // len(pattern))
    assert len(media) == MEDIA_SIZE
    build(
        os.path.join(output, "media.zim"),
        [
            TextItem(
                "index.html",
                "Media fixture",
                "text/html",
                "<!doctype html><html><head><title>Media fixture</title></head>"
                '<body><video src="media/probe.mp4" controls></video></body></html>',
                True,
            ),
            TextItem("media/probe.mp4", "Probe video", "video/mp4", media, compress=True),
        ],
        main_path="index.html",
    )


    build(
        os.path.join(output, "zimit_warc.zim"),
        [
            TextItem(
                "example.org/guide/index.html",
                "Zimit fixture",
                "text/html",
                "<!doctype html><html><head><title>Zimit fixture</title>"
                '<script src="../../_zim_static/wombat.js"></script>'
                '<script src="../../_zim_static/wombatSetup.js"></script></head>'
                '<body><h1>Zimit2 static replay</h1>'
                '<p>No service worker is required.</p>'
                '<a href="../../capture.warc">WARC capture</a></body></html>',
                True,
            ),
            TextItem(
                "_zim_static/wombat.js",
                "Wombat",
                "text/javascript",
                'window.__kiwixZimitWombat = true;\n',
            ),
            TextItem(
                "_zim_static/wombatSetup.js",
                "Wombat setup",
                "text/javascript",
                'window.__kiwixWombatSetup = true;\n',
            ),
            TextItem(
                "capture.warc",
                "WARC capture",
                "application/warc",
                "WARC/1.0\r\nWARC-Type: response\r\n"
                "WARC-Target-URI: https://example.org/\r\n"
                "Content-Length: 12\r\n\r\nhello world\n",
            ),
        ],
        main_path="example.org/guide/index.html",
        metadata=(
            ("Scraper", "warc2zim fixture"),
            ("Tags", "_ftindex:yes;_category:other"),
        ),
    )


if __name__ == "__main__":
    main()
