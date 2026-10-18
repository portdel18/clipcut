# Draws the ClipCut app icon (original work): an amber "C" ring around a play triangle, cut by a
# diagonal gap like a splice, on a dark rounded square. Writes the sizes Tauri bundles.
import sys, math
from PIL import Image, ImageDraw
out = sys.argv[1]
S = 1024
def draw(S):
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    r = S * 0.22
    d.rounded_rectangle([S*0.04, S*0.04, S*0.96, S*0.96], radius=r, fill=(23, 22, 26, 255))
    amber = (244, 196, 67, 255)
    c = S / 2; R = S * 0.33; w = S * 0.085
    # "C": a ring open on the right
    d.arc([c-R, c-R, c+R, c+R], start=40, end=320, fill=amber, width=int(w))
    # play triangle in the middle
    t = S * 0.15
    d.polygon([(c - t*0.75, c - t), (c - t*0.75, c + t), (c + t*0.95, c)], fill=amber)
    # the splice: a diagonal gap through the triangle
    g = S * 0.028
    d.line([(c - t*1.3, c + t*1.1), (c + t*0.9, c - t*1.1)], fill=(23, 22, 26, 255), width=int(g))
    return im
big = draw(S)
for name, px in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)]:
    big.resize((px, px), Image.LANCZOS).save(f"{out}/{name}")
big.resize((256, 256), Image.LANCZOS).save(f"{out}/icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
