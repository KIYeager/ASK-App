"""Tests für das Lesen der Befehlsausgabe (python/ask_terminal/streams.py). Läuft auf jedem System:
python -m unittest tests/test_streams.py -v
"""
import asyncio
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "python"))

from ask_terminal.streams import read_lines  # noqa: E402


def lies(data: bytes, chunk: int = 64 * 1024) -> list:
    async def run():
        reader = asyncio.StreamReader()  # Grenze 64 KiB wie bei einem Befehl
        reader.feed_data(data)
        reader.feed_eof()
        teile = []

        async def handle(line):
            teile.append(line)

        await read_lines(reader, handle, chunk)
        return teile

    return asyncio.run(run())


class StreamsTest(unittest.TestCase):
    def test_zeilen(self):
        self.assertEqual(lies(b"eins\nzwei\r\ndrei"), [b"eins\n", b"zwei\r\n", b"drei"])
        self.assertEqual(lies(b""), [])
        self.assertEqual(lies(b"\n\n"), [b"\n", b"\n"])

    def test_sehr_lange_zeile_geht_nicht_verloren(self):
        lang = b"x" * 1_000_000
        teile = lies(b"vorher\n" + lang + b"\nnachher\n")
        self.assertEqual(teile[0], b"vorher\n")
        self.assertEqual(teile[-1], b"nachher\n")
        self.assertEqual(b"".join(teile), b"vorher\n" + lang + b"\nnachher\n")
        self.assertTrue(all(len(t) <= 2 * 64 * 1024 + 1 for t in teile))

    def test_umlaute_werden_nicht_zerschnitten(self):
        text = ("äöü€😀" * 40000).encode("utf-8")
        teile = lies(text, chunk=1000)
        self.assertEqual(b"".join(teile), text)
        for t in teile:
            t.decode("utf-8")  # jedes Stück für sich gültig

    def test_asyncio_selbst_scheitert_an_langen_zeilen(self):
        # Der Grund für streams.py: So las der Server früher, und der Befehl wurde beendet
        async def run():
            reader = asyncio.StreamReader()
            reader.feed_data(b"x" * 100_000 + b"\n")
            reader.feed_eof()
            async for _ in reader:
                pass

        with self.assertRaises(ValueError):
            asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
