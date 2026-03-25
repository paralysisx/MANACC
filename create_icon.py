"""
Creates a hexagonal LAV app icon with gold border and text.
Outputs: src-tauri/icons/icon.ico + icon.png
"""
from PIL import Image, ImageDraw, ImageFont
import math
import os

OUT_DIR = r'C:\Users\hrist\lol-account-manager-tauri\src-tauri\icons'

# Colors
DARK_NAVY   = (8,   14,  26,  255)
GOLD_BORDER = (200, 155, 60,  255)
GOLD_TEXT   = (220, 178, 80,  255)
TRANSPARENT = (0,   0,   0,   0  )


def hex_points(cx, cy, r, rotation_deg=0):
    pts = []
    for i in range(6):
        angle = math.radians(60 * i + rotation_deg)
        pts.append((cx + r * math.cos(angle), cy + r * math.sin(angle)))
    return pts


def create_icon(size):
    img  = Image.new('RGBA', (size, size), TRANSPARENT)
    draw = ImageDraw.Draw(img)
    cx, cy = size / 2, size / 2

    r_outer  = size * 0.46
    border_w = max(2, size * 0.032)
    r_inner  = r_outer - border_w

    pts_outer = hex_points(cx, cy, r_outer,  0)
    pts_inner = hex_points(cx, cy, r_inner,  0)
    pts_deco  = hex_points(cx, cy, r_inner - max(1, size * 0.012), 0)

    # Gold outer → dark inner creates the border ring
    draw.polygon(pts_outer, fill=GOLD_BORDER)
    draw.polygon(pts_inner, fill=DARK_NAVY)

    # Faint inner decoration hexagon
    if size >= 48:
        draw.polygon(pts_deco, outline=(200, 155, 60, 55))

    # Text "LAV"
    if size >= 32:
        text      = 'LAV'
        font_size = max(10, int(size * 0.37))
        font      = None

        for fp in [
            r'C:\Windows\Fonts\ariblk.ttf',
            r'C:\Windows\Fonts\impact.ttf',
            r'C:\Windows\Fonts\arialbd.ttf',
            r'C:\Windows\Fonts\arial.ttf',
        ]:
            if os.path.exists(fp):
                try:
                    font = ImageFont.truetype(fp, font_size)
                    break
                except Exception:
                    pass

        if font is None:
            font = ImageFont.load_default()

        bbox = draw.textbbox((0, 0), text, font=font)
        tw   = bbox[2] - bbox[0]
        th   = bbox[3] - bbox[1]
        tx   = cx - tw / 2 - bbox[0]
        ty   = cy - th / 2 - bbox[1]

        # Subtle shadow for larger sizes
        if size >= 64:
            so = max(1, int(size * 0.012))
            draw.text((tx + so, ty + so), text, font=font, fill=(0, 0, 0, 140))

        draw.text((tx, ty), text, font=font, fill=GOLD_TEXT)

    return img


def main():
    os.makedirs(OUT_DIR, exist_ok=True)

    # Create a 256x256 base; PIL resizes it for each ICO size
    big      = create_icon(256)
    ico_path = os.path.join(OUT_DIR, 'icon.ico')
    big.save(
        ico_path,
        format='ICO',
        sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)],
    )
    print(f'ICO saved: {ico_path}')

    png_path = os.path.join(OUT_DIR, 'icon.png')
    big.save(png_path)
    print(f'PNG saved: {png_path}')

    print('Done!')


if __name__ == '__main__':
    main()
