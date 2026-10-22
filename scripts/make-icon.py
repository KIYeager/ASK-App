"""Erzeugt resources/icon.png und resources/icon.ico (Schriftzug «ASK» auf Petrol)."""
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


img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)
draw.rounded_rectangle((32, 32, SIZE - 32, SIZE - 32), radius=200, fill=BG)
f = font(400)
box = draw.textbbox((0, 0), "ASK", font=f)
w, h = box[2] - box[0], box[3] - box[1]
draw.text(((SIZE - w) / 2 - box[0], (SIZE - h) / 2 - box[1] - 20), "ASK", font=f, fill=FG)
# Kleine Linie als Hinweis auf den Arbeitsordner/Terminal-Cursor
draw.rounded_rectangle((SIZE / 2 - 150, 760, SIZE / 2 + 150, 800), radius=20, fill=(255, 255, 255, 200))

img.resize((512, 512), Image.LANCZOS).save(os.path.join(ROOT, "resources", "icon.png"))
img.save(os.path.join(ROOT, "resources", "icon.ico"), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("Icon geschrieben")
