"""Erzeugt resources/icon.png und resources/icon.ico (Sprechblase mit Prompt auf Petrol) sowie
resources/splash.bmp, das Startbild der einzelnen EXE während des Entpackens."""
import os

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIZE = 1024
BG = (11, 110, 117)
FG = (255, 255, 255)


def font(size):
    for name in ("segoeuib.ttf", "arialbd.ttf"):
        path = os.path.join(os.environ.get("WINDIR", r"C:\Windows"), "Fonts", name)
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def gradient(top, bottom):
    g = Image.new("RGBA", (SIZE, SIZE))
    d = ImageDraw.Draw(g)
    for y in range(SIZE):
        t = y / (SIZE - 1)
        d.line([(0, y), (SIZE, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    return g


def thick_line(d, pts, width, fill):
    d.line(pts, fill=fill, width=width, joint="curve")
    r = width / 2
    for x, y in pts:
        d.ellipse((x - r, y - r, x + r, y + r), fill=fill)


# Icon: weisse Sprechblase mit Prompt «›_» auf Petrol-Verlauf. Nur Formen, keine Schrift,
# damit das Ergebnis auf jedem Rechner gleich aussieht.
mask = Image.new("L", (SIZE, SIZE), 0)
ImageDraw.Draw(mask).rounded_rectangle((32, 32, SIZE - 32, SIZE - 32), radius=220, fill=255)
img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
img.paste(gradient((18, 132, 140), (8, 78, 84)), (0, 0), mask)
draw = ImageDraw.Draw(img)
draw.rounded_rectangle((200, 230, 824, 700), radius=150, fill=FG)
draw.polygon([(330, 660), (300, 830), (480, 690)], fill=FG)
thick_line(draw, [(360, 370), (470, 465), (360, 560)], 70, BG)
draw.rounded_rectangle((520, 520, 670, 580), radius=24, fill=BG)

img.resize((512, 512), Image.LANCZOS).save(os.path.join(ROOT, "resources", "icon.png"))
img.save(os.path.join(ROOT, "resources", "icon.ico"), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("Icon geschrieben")

# Startbild: 24-Bit-BMP ohne Transparenz, sonst zeigt der NSIS-Starter es nicht an
splash = Image.new("RGB", (480, 270), BG)
draw = ImageDraw.Draw(splash)
for text, size, y in (("ASK", 120, 105), ("wird gestartet …", 26, 200), ("Das dauert einen Moment.", 18, 236)):
    f = font(size)
    box = draw.textbbox((0, 0), text, font=f)
    draw.text(((480 - (box[2] - box[0])) / 2 - box[0], y - (box[3] - box[1]) / 2 - box[1]), text, font=f, fill=FG)
splash.save(os.path.join(ROOT, "resources", "splash.bmp"))
print("Startbild geschrieben")
