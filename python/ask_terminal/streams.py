"""Ausgabe eines Befehls zeilenweise lesen, ohne Grenze für die Zeilenlänge.

asyncio bricht «async for line in stream» mit einem Fehler ab, sobald eine Zeile länger als 64 KiB ist
(eine Zeile JSON, Base64, verkleinertes HTML). open-terminal beendet den Befehl dann, und die Ausgabe
geht verloren. Hier kommen überlange Zeilen in Stücken weiter.
"""

CHUNK = 64 * 1024


async def read_lines(stream, handle, chunk: int = CHUNK) -> None:
    """Ruft «await handle(zeile)» für jede Zeile (mit Zeilenende) auf; überlange Zeilen in Stücken."""
    pending = b""
    while True:
        data = await stream.read(chunk)
        if not data:
            break
        pending += data
        *lines, pending = pending.split(b"\n")
        for line in lines:
            await handle(line + b"\n")
        if len(pending) >= chunk:
            # Kein Zeichen in der Mitte trennen: unvollständige UTF-8-Folge am Ende zurückhalten
            cut = len(pending)
            while cut > len(pending) - 4 and cut > 0 and (pending[cut - 1] & 0xC0) == 0x80:
                cut -= 1
            if cut > 0 and pending[cut - 1] >= 0xC0:
                cut -= 1
            if cut <= 0:
                cut = len(pending)
            await handle(pending[:cut])
            pending = pending[cut:]
    if pending:
        await handle(pending)
