# Regenerate the icons with:
#   python3 scripts/icon.py icon-source.png && npx tauri icon icon-source.png -o src-tauri/icons
# Hablog's icon: a gold pixel H on a dark tile; its crossbar is the two
# directions of traffic (red in, purple out).
import sys
from PIL import Image

N = 32
img = Image.new("RGBA", (N, N), (0, 0, 0, 0))
px = img.load()

def rect(x0, y0, x1, y1, color):
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            px[x, y] = color

OUTLINE = (6, 8, 16, 255)
# Tile with cut corners and a vertical gradient in bands.
bands = [(46, 62, 128), (40, 54, 115), (34, 45, 100), (29, 37, 80), (24, 30, 64)]
for y in range(1, 31):
    for x in range(1, 31):
        corner = min(x - 1, 30 - x) + min(y - 1, 30 - y)
        if corner < 2:
            continue
        edge = min(x - 1, 30 - x, y - 1, 30 - y) == 0 or corner == 2
        px[x, y] = OUTLINE if edge else bands[min(4, (y - 2) // 6)] + (255,)
# Floor tiles along the bottom, like the window's banner.
for x in range(3, 29):
    px[x, 27] = (58, 74, 146, 255) if (x - 3) % 5 else (40, 52, 110, 255)
    px[x, 28] = (47, 60, 122, 255)
# A highlight along the top edge.
for x in range(4, 28):
    px[x, 2] = (66, 84, 160, 255)

GOLD, LIGHT, DARK = (245, 197, 66, 255), (255, 224, 138, 255), (138, 106, 18, 255)
IN, OUT = (255, 107, 107, 255), (179, 140, 255, 255)

def stem(x0):
    rect(x0 + 1, 7, x0 + 5, 24, DARK)      # shadow, one pixel down and right
    rect(x0, 6, x0 + 4, 23, GOLD)
    rect(x0, 6, x0, 23, LIGHT)
    rect(x0, 6, x0 + 4, 6, LIGHT)

stem(7)
stem(20)
rect(12, 18, 19, 18, DARK)
rect(12, 13, 15, 17, IN)
rect(16, 13, 19, 17, OUT)
rect(12, 13, 15, 13, (255, 160, 160, 255))
rect(16, 13, 19, 13, (212, 190, 255, 255))

out = sys.argv[1]
img.resize((1024, 1024), Image.NEAREST).save(out)
